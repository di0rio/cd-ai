use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::permissions::{
    ApprovalAction, CommandClass, PermissionDecision, PermissionKind, classify,
};
use crate::redactor;
use crate::sandbox::{self, SandboxExec};
use crate::tools::cancel::CancelToken;
use crate::tools::edit::inside_git_dir;
use crate::tools::{
    CommandResult, DEFAULT_COMMAND_TIMEOUT_MS, EventSink, MAX_OUTPUT_BYTES, Responder,
    RunCommandArgs, ToolEngine, ToolError,
};

/// The model picks the timeout; this bounds it (and keeps `Instant + timeout` from overflowing).
const MAX_COMMAND_TIMEOUT_MS: u64 = 30 * 60 * 1000;

/// run_command: argv only, never a shell (design D2). Commands classified `read` (and `validate`,
/// with a filesystem sandbox) run automatically (§20.4); every other class asks the user with the
/// exact argv, never a model summary, and the class is part of the approval (design D5/D8).
pub fn run_command(
    engine: &mut ToolEngine,
    args: RunCommandArgs,
    events: EventSink,
    responder: Responder,
) -> Result<(PermissionDecision, CommandResult), ToolError> {
    let argv = &args.argv;
    if argv.is_empty() || argv[0].trim().is_empty() {
        return Err(ToolError::UnknownCommand);
    }
    if argv
        .iter()
        .any(|token| crate::permissions::has_shell_metachar(token))
    {
        return Err(ToolError::CompoundCommand);
    }

    let cwd = match args.cwd.clone() {
        Some(cwd) => {
            let canonical = engine.workspace.resolve(&cwd)?;
            if !std::fs::metadata(&canonical)
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
            {
                return Err(ToolError::NotADirectory);
            }
            canonical
        }
        None => engine.workspace.root().to_path_buf(),
    };

    // §20.4: `read` is `auto` in all three permission modes; `validate` too, but only with a
    // filesystem sandbox (it runs repo code). Write commands become auto only in AUTO/FULL ACCESS
    // when a filesystem sandbox is actually on (D6). Network, destructive and unknown always ask.
    // The decision never comes from a model claim (§20.5).
    let class = escalate_for_paths(classify(argv), argv, &engine.workspace, &cwd);
    let decision = engine.authorize(
        events,
        responder,
        PermissionKind::RunCommand {
            class: class.clone(),
        },
        ApprovalAction::RunCommand {
            argv: argv.clone(),
            class: class.clone(),
            cwd: crate::tools::display_path(&engine.workspace, &cwd),
        },
    );
    if decision == PermissionDecision::Denied {
        let reason = if class == CommandClass::Validate && !engine.sandbox_caps().filesystem {
            "comando de validação negado: sem sandbox ele executa código do repositório e exige \
             aprovação (cd-ai sandbox-setup restaura a execução automática)"
        } else {
            "comando negado"
        };
        return Err(ToolError::PermissionDenied {
            reason: reason.to_string(),
        });
    }

    let id = engine.command_next_id;
    engine.command_next_id += 1;
    // Network is released only for an approved `network` command (D3). Every other class,
    // including a granted `unknown`, is born without a route.
    let allow_network = class == CommandClass::Network && decision == PermissionDecision::Granted;
    engine.emit(
        events,
        crate::events::ToolEvent::CommandStarted {
            id,
            argv: argv.clone(),
            class,
        },
    );

    let timeout = Duration::from_millis(
        args.timeout_ms
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS)
            .min(MAX_COMMAND_TIMEOUT_MS),
    );
    let started = Instant::now();
    let mut command = sandbox::command(
        &argv[0],
        &SandboxExec {
            workspace: engine.workspace.root().to_path_buf(),
            allow_network,
        },
    )
    .map_err(|error| ToolError::Io(format!("sandbox: {error}")))?;
    command
        .args(harden_git(argv))
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    platform_spawn_setup(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| ToolError::Io(format!("não foi possível executar: {error}")))?;

    // Cloned up front: the engine is borrowed mutably again to emit the events below.
    let cancel = engine.cancel_token().clone();
    let status = wait_with_timeout(&mut child, timeout, MAX_OUTPUT_BYTES, &cancel);
    let duration_ms = started.elapsed().as_millis() as u64;

    let (output, truncated, timed_out) = (
        redactor::redact(&status.output).text,
        status.output_capped,
        status.timed_out,
    );
    let exit_code = status.exit_code;
    engine.emit(
        events,
        crate::events::ToolEvent::CommandCompleted {
            id,
            exit_code,
            duration_ms,
            truncated,
            output_len: output.len(),
        },
    );

    // The tree is already dead; the task gets the cancellation, not a partial result.
    if status.cancelled {
        return Err(ToolError::Cancelled);
    }

    Ok((
        decision,
        CommandResult {
            id,
            exit_code,
            duration_ms,
            output,
            truncated,
            timed_out,
        },
    ))
}

/// Arguments for the program after `argv[0]`. For `git` they gain what the git tools always pass
/// (`tools/git.rs`): a repository's own config (`core.fsmonitor`, `diff.<driver>.textconv`, an
/// external diff) can name a program for `status`/`diff`/`log`/`show` to run, and those four are
/// classified `read`, which runs without asking.
fn harden_git(argv: &[String]) -> Vec<String> {
    let is_git = argv[0].rsplit(['/', '\\']).next().is_some_and(|name| {
        name.eq_ignore_ascii_case("git") || name.eq_ignore_ascii_case("git.exe")
    });
    if !is_git {
        return argv[1..].to_vec();
    }
    let mut hardened = vec!["-c".to_string(), "core.fsmonitor=false".to_string()];
    hardened.extend(argv[1..].iter().cloned());
    if matches!(
        argv.get(1).map(String::as_str),
        Some("diff" | "log" | "show")
    ) {
        // Right after the subcommand: anything later could sit after a `--`.
        hardened.splice(3..3, ["--no-ext-diff", "--no-textconv"].map(String::from));
    }
    hardened
}

/// The sandbox leaves reads open everywhere, so an automatic command reads whatever its arguments
/// name. An argument that points outside the workspace or at a secret file drops the command to
/// `unknown`: the user sees the exact argv instead of the command running on its own. The same
/// goes for an argument inside `.git` (hooks and config name programs git runs on its own, and
/// `touch`/`cp`/`mv`/`tee` are automatic writes), and `git checkout <existing path>` is a
/// discard of worktree changes, `destructive`.
fn escalate_for_paths(
    class: CommandClass,
    argv: &[String],
    workspace: &crate::workspace::Workspace,
    cwd: &std::path::Path,
) -> CommandClass {
    if !matches!(
        class,
        CommandClass::Read | CommandClass::Validate | CommandClass::Write
    ) {
        return class;
    }
    if names_git_dir(argv, workspace, cwd) {
        return CommandClass::Unknown;
    }
    if class == CommandClass::Write && checks_out_a_path(argv, workspace, cwd) {
        return CommandClass::Destructive;
    }
    let reaches_out = argv.iter().skip(1).any(|token| {
        let value = if token.starts_with('-') {
            match token.split_once('=') {
                Some((_, value)) => value,
                None => return false,
            }
        } else {
            token.as_str()
        };
        // `HEAD:.env` names the file after the colon.
        let named = value.rsplit_once(':').map_or(value, |(_, path)| path);
        let secret = |path: &str| {
            workspace.resolve(path).map_or(true, |canonical| {
                redactor::detect_path_secret(&canonical).is_some()
            })
        };
        workspace.resolve(value).is_err() || secret(value) || secret(named)
    });
    if reaches_out {
        CommandClass::Unknown
    } else {
        class
    }
}

/// The values an argv hands to its program as paths: positional arguments, the value of
/// `--flag=value`, and the part after the colon of `rev:path`.
fn path_candidates(argv: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    for token in argv.iter().skip(1) {
        let value = if token.starts_with('-') {
            match token.split_once('=') {
                Some((_, value)) => value,
                None => continue,
            }
        } else {
            token.as_str()
        };
        out.push(value);
        if let Some((_, path)) = value.rsplit_once(':') {
            out.push(path);
        }
    }
    out
}

/// Both readings of a relative path, from the workspace root and from the command's cwd, as the
/// canonical paths `Workspace::resolve` accepts.
fn resolved_both_ways(
    workspace: &crate::workspace::Workspace,
    cwd: &std::path::Path,
    path: &str,
) -> Vec<std::path::PathBuf> {
    [workspace.resolve(path), workspace.resolve(cwd.join(path))]
        .into_iter()
        .flatten()
        .collect()
}

/// Whether the command runs inside `.git` or has an argument that points into it.
fn names_git_dir(
    argv: &[String],
    workspace: &crate::workspace::Workspace,
    cwd: &std::path::Path,
) -> bool {
    inside_git_dir(workspace, cwd)
        || path_candidates(argv).into_iter().any(|path| {
            resolved_both_ways(workspace, cwd, path)
                .iter()
                .any(|canonical| inside_git_dir(workspace, canonical))
        })
}

/// `git checkout <x>` where `x` is a file or folder that exists: a pathspec, not a branch, so it
/// overwrites the worktree. Only the disk can tell it from a branch name, and a name that is both
/// is treated as the destructive reading.
fn checks_out_a_path(
    argv: &[String],
    workspace: &crate::workspace::Workspace,
    cwd: &std::path::Path,
) -> bool {
    let is_git = argv[0].rsplit(['/', '\\']).next().is_some_and(|name| {
        name.eq_ignore_ascii_case("git") || name.eq_ignore_ascii_case("git.exe")
    });
    if !is_git || argv.get(1).map(String::as_str) != Some("checkout") {
        return false;
    }
    argv.iter()
        .skip(2)
        .filter(|token| !token.starts_with('-'))
        .any(|path| {
            resolved_both_ways(workspace, cwd, path)
                .iter()
                .any(|canonical| canonical != workspace.root() && canonical.exists())
        })
}

struct RunStatus {
    exit_code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    output_capped: bool,
    output: String,
}

/// Polls the child, killing the whole tree on timeout or cancellation. Readers drain the
/// pipes on background threads so a chatty or quiet child never deadlocks the timeout.
fn wait_with_timeout(
    child: &mut Child,
    timeout: Duration,
    cap: usize,
    cancel: &CancelToken,
) -> RunStatus {
    let stdout = child
        .stdout
        .take()
        .map(|pipe| thread::spawn(move || drain_capped(pipe, cap)));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(move || drain_capped(pipe, cap)));

    let deadline = Instant::now() + timeout;
    let mut cancelled = false;
    let exit = loop {
        if let Some(status) = child.try_wait().unwrap_or(None) {
            break status.code();
        }
        if cancel.is_cancelled() {
            cancelled = true;
            break None;
        }
        if Instant::now() >= deadline {
            break None; // timed out
        }
        thread::sleep(Duration::from_millis(10));
    };

    // Kill before joining the readers: they only finish once the child closes the pipes.
    if exit.is_none() {
        kill_process_tree(child);
    }

    let stdout = stdout
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let stderr = stderr
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    let output_capped = stdout.len() >= cap || stderr.len() >= cap;

    let mut text = String::from_utf8_lossy(&stdout).into_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&String::from_utf8_lossy(&stderr));

    RunStatus {
        exit_code: exit,
        timed_out: exit.is_none() && !cancelled,
        cancelled,
        output_capped,
        output: text,
    }
}

/// Keeps reading after the cap and drops the excess: a reader that stops leaves the pipe full,
/// and the child then blocks on write until the timeout kills it.
fn drain_capped(mut pipe: impl Read, cap: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..n.min(room)]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    buf
}

#[cfg(unix)]
fn platform_spawn_setup(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Own process group so the whole tree dies together on timeout.
    command.process_group(0);
}

#[cfg(not(unix))]
fn platform_spawn_setup(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(windows_new_process_group());
}

#[cfg(not(unix))]
fn windows_new_process_group() -> u32 {
    0x0000_0200 // CREATE_NEW_PROCESS_GROUP
}

/// Silent on purpose: a child that already exited on its own is not an error, and
/// cancellation takes this path all the time.
///
/// Process-group kill is the real tree-kill. `Child::kill` is the fallback when the
/// group signal is ignored (restricted containers, no `CAP_KILL` on the group) — without
/// it, cancel waits for the command to finish because the pipe readers never see EOF.
#[cfg(unix)]
fn kill_process_tree(child: &mut Child) {
    let pgid = child.id() as i64;
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

/// Silent on purpose: `taskkill` prints "The process NNN not found." whenever the child
/// is already gone, which the cancel path hits routinely.
#[cfg(not(unix))]
fn kill_process_tree(child: &mut Child) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

/// Per-platform argv for tests that spawn real processes (plan 015, D15): the gate
/// runs on Windows 11 too, where `echo`, `ls` and `sleep` are not executables.
#[cfg(test)]
pub(crate) mod test_argv {
    fn owned(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    pub fn echo(text: &str) -> Vec<String> {
        if cfg!(windows) {
            owned(&["cmd", "/C", "echo", text])
        } else {
            owned(&["echo", text])
        }
    }

    pub fn list_cwd() -> Vec<String> {
        if cfg!(windows) {
            owned(&["cmd", "/C", "dir", "/B"])
        } else {
            owned(&["ls"])
        }
    }

    pub fn sleep_secs(n: u64) -> Vec<String> {
        if cfg!(windows) {
            owned(&[
                "powershell",
                "-NoProfile",
                "-Command",
                &format!("Start-Sleep -Seconds {n}"),
            ])
        } else {
            owned(&["sleep", &n.to_string()])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn boot(dir: &Path) -> ToolEngine {
        let ws = crate::workspace::Workspace::open(dir).unwrap();
        ToolEngine::new(ws, "task_cmd")
    }

    fn cv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    fn approve(
        _request: crate::permissions::ApprovalRequest,
    ) -> crate::permissions::ApprovalResponse {
        crate::permissions::ApprovalResponse::Granted
    }

    /// Proves the automatic path: a `read`/`validate` command must never reach a responder.
    fn never_asked(
        request: crate::permissions::ApprovalRequest,
    ) -> crate::permissions::ApprovalResponse {
        panic!("approval must not be requested for {:?}", request.action);
    }

    fn approvals_asked(seen: &[crate::events::ToolEventMessage]) -> usize {
        seen.iter()
            .filter(|message| {
                matches!(
                    message.event,
                    crate::events::ToolEvent::ApprovalRequired { .. }
                )
            })
            .count()
    }

    fn started_class(
        seen: &[crate::events::ToolEventMessage],
    ) -> Option<crate::permissions::CommandClass> {
        seen.iter().find_map(|message| match &message.event {
            crate::events::ToolEvent::CommandStarted { class, .. } => Some(class.clone()),
            _ => None,
        })
    }

    #[test]
    fn runs_echo_and_reports_exit() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (decision, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: test_argv::echo("ola"),
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        // `echo` is a read command on unix (auto) and a `cmd /C` wrapper on Windows (approved).
        assert_ne!(decision, PermissionDecision::Denied);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.contains("ola"));
        assert!(!result.timed_out);
    }

    #[test]
    fn validate_command_asks_without_a_sandbox_and_says_why_when_denied() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        engine.set_sandbox_caps(crate::sandbox::SandboxCapabilities {
            filesystem: false,
            network_block: false,
        });
        let mut asked = 0usize;
        let mut deny = |_: crate::permissions::ApprovalRequest| {
            asked += 1;
            crate::permissions::ApprovalResponse::Denied { reason: None }
        };
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["cargo", "fmt", "--check", "--version"]),
                cwd: None,
                timeout_ms: Some(30_000),
            },
            &mut sink,
            &mut deny,
        )
        .unwrap_err();

        assert_eq!(asked, 1);
        match err {
            ToolError::PermissionDenied { reason } => {
                assert!(reason.contains("sandbox-setup"), "{reason}")
            }
            other => panic!("esperava PermissionDenied, veio {other:?}"),
        }
    }

    #[test]
    fn validate_command_runs_without_approval_with_a_sandbox() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        engine.set_sandbox_caps(crate::sandbox::SandboxCapabilities {
            filesystem: true,
            network_block: true,
        });
        let mut seen = Vec::new();
        let mut sink = |message: crate::events::ToolEventMessage| seen.push(message);

        // §20.4: `validate` is `auto` with a sandbox; `never_asked` panics if the approval path
        // is taken.
        let (decision, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["cargo", "fmt", "--check", "--version"]),
                cwd: None,
                timeout_ms: Some(30_000),
            },
            &mut sink,
            &mut never_asked,
        )
        .unwrap();

        assert_eq!(decision, PermissionDecision::Auto);
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(approvals_asked(&seen), 0);
        assert_eq!(
            started_class(&seen),
            Some(crate::permissions::CommandClass::Validate)
        );
    }

    #[test]
    fn read_command_runs_without_approval() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut seen = Vec::new();
        let mut sink = |message: crate::events::ToolEventMessage| seen.push(message);

        // The tempdir is not a repository, so the exit code is irrelevant: what matters is that
        // `git status` (class read) never opens an approval. CommandStarted is emitted either way.
        let _ = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["git", "status", "--short"]),
                cwd: None,
                timeout_ms: Some(30_000),
            },
            &mut sink,
            &mut never_asked,
        );

        assert_eq!(approvals_asked(&seen), 0);
        assert_eq!(
            started_class(&seen),
            Some(crate::permissions::CommandClass::Read)
        );
    }

    #[test]
    fn write_command_still_asks_for_approval() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut seen = Vec::new();
        let mut sink = |message: crate::events::ToolEventMessage| seen.push(message);
        let mut asked = 0usize;
        let mut count = |_: crate::permissions::ApprovalRequest| {
            asked += 1;
            crate::permissions::ApprovalResponse::Granted
        };

        // `git add -A` is class write: outside read/validate the approval is still mandatory.
        let _ = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["git", "add", "-A"]),
                cwd: None,
                timeout_ms: Some(30_000),
            },
            &mut sink,
            &mut count,
        );

        assert_eq!(asked, 1);
        assert_eq!(approvals_asked(&seen), 1);
        assert_eq!(
            started_class(&seen),
            Some(crate::permissions::CommandClass::Write)
        );
    }

    #[test]
    fn rejects_compound_shell_argv() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: vec![
                    "echo".into(),
                    "a".into(),
                    "|".into(),
                    "rm".into(),
                    "-rf".into(),
                    "/".into(),
                ],
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut approve,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::CompoundCommand));
    }

    #[test]
    fn timeout_kills_and_marks_timed_out() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (_, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: test_argv::sleep_secs(5),
                cwd: None,
                timeout_ms: Some(200),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert!(result.timed_out);
        assert!(result.exit_code.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn chatty_command_finishes_instead_of_timing_out() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        // ~1.3 MB of output: far past the cap and the pipe buffer. The child must still run to
        // completion, so the readers have to keep draining after the cap is hit.
        let (_, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["seq", "1", "200000"]),
                cwd: None,
                timeout_ms: Some(10_000),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert!(!result.timed_out, "a chatty command must not time out");
        assert_eq!(result.exit_code, Some(0));
        assert!(result.truncated);
        assert!(result.output.len() <= 2 * MAX_OUTPUT_BYTES + 1);
    }

    #[cfg(unix)]
    #[test]
    fn automatic_commands_ask_when_an_argument_reaches_outside_or_a_secret() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=abc\n").unwrap();
        std::fs::write(dir.path().join("notas.txt"), "oi\n").unwrap();
        std::fs::write(outside.path().join("fora.txt"), "segredo\n").unwrap();
        let fora = outside
            .path()
            .join("fora.txt")
            .to_string_lossy()
            .into_owned();

        for argv in [
            cv(&["cat", &fora]),
            cv(&["cat", "../fora.txt"]),
            cv(&["cat", ".env"]),
            cv(&["grep", &format!("--file={fora}"), "x"]),
            cv(&["git", "show", "HEAD:.env"]),
            cv(&["cargo", "test", &format!("--manifest-path={fora}")]),
        ] {
            let mut engine = boot(dir.path());
            let mut sink = |_: crate::events::ToolEventMessage| {};
            let mut asked = 0usize;
            let mut refuse = |_: crate::permissions::ApprovalRequest| {
                asked += 1;
                crate::permissions::ApprovalResponse::Denied { reason: None }
            };
            let err = run_command(
                &mut engine,
                RunCommandArgs {
                    argv: argv.clone(),
                    cwd: None,
                    timeout_ms: None,
                },
                &mut sink,
                &mut refuse,
            )
            .unwrap_err();
            assert_eq!(asked, 1, "{argv:?} ran without approval");
            assert!(matches!(err, ToolError::PermissionDenied { .. }));
        }

        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};
        let (decision, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["cat", "notas.txt"]),
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut never_asked,
        )
        .unwrap();
        assert_eq!(decision, PermissionDecision::Auto);
        assert!(result.output.contains("oi"));
    }

    fn repo_with_git_dir() -> (tempfile::TempDir, crate::workspace::Workspace) {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git").join("hooks")).unwrap();
        std::fs::write(dir.path().join(".git").join("config"), "[core]\n").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src").join("a.rs"), "fn a() {}\n").unwrap();
        let workspace = crate::workspace::Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    #[test]
    fn write_commands_aimed_at_dot_git_need_approval() {
        let (_dir, workspace) = repo_with_git_dir();
        let root = workspace.root().to_path_buf();
        let shown = |path: &str| root.join(path).to_string_lossy().into_owned();
        for argv in [
            cv(&["touch", ".git/hooks/pre-commit"]),
            cv(&["cp", "x", ".git/hooks/pre-commit"]),
            cv(&["mv", "x", ".git/config"]),
            cv(&["tee", ".git/config"]),
            cv(&["mkdir", ".git/hooks/sub"]),
            // Any case, any depth, with `..` in between, an absolute path or a `--flag=` value.
            cv(&["touch", ".GIT/hooks/post-merge"]),
            cv(&["touch", "src/../.git/hooks/pre-push"]),
            cv(&["touch", &shown(".git/hooks/pre-commit")]),
            cv(&["cp", "--target-directory=.git/hooks", "x"]),
            cv(&["cat", ".git/config"]),
        ] {
            assert_eq!(
                escalate_for_paths(classify(&argv), &argv, &workspace, &root),
                CommandClass::Unknown,
                "{argv:?}"
            );
        }
        // Relative to the cwd, not only to the workspace root.
        let hooks = root.join(".git").join("hooks");
        let argv = cv(&["touch", "pre-commit"]);
        assert_eq!(
            escalate_for_paths(classify(&argv), &argv, &workspace, &hooks),
            CommandClass::Unknown
        );
        let argv = cv(&["touch", "../config"]);
        assert_eq!(
            escalate_for_paths(classify(&argv), &argv, &workspace, &hooks),
            CommandClass::Unknown
        );
        // Ordinary writes, and names that merely start like `.git`, stay as they were.
        for argv in [
            cv(&["touch", "novo.txt"]),
            cv(&["mkdir", "-p", "src/x"]),
            cv(&["touch", ".gitignore"]),
            cv(&["touch", ".github/workflows/ci.yml"]),
            cv(&["cat", "src/a.rs"]),
        ] {
            assert_eq!(
                escalate_for_paths(classify(&argv), &argv, &workspace, &root),
                classify(&argv),
                "{argv:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_dot_git_is_dot_git() {
        let (dir, workspace) = repo_with_git_dir();
        std::os::unix::fs::symlink(dir.path().join(".git"), dir.path().join("atalho")).unwrap();
        let root = workspace.root().to_path_buf();
        let argv = cv(&["touch", "atalho/hooks/pre-commit"]);
        assert_eq!(
            escalate_for_paths(classify(&argv), &argv, &workspace, &root),
            CommandClass::Unknown
        );
    }

    #[test]
    fn write_command_aimed_at_dot_git_asks_before_running() {
        let (dir, _workspace) = repo_with_git_dir();
        let mut engine = boot(dir.path());
        engine.set_sandbox_caps(crate::sandbox::SandboxCapabilities {
            filesystem: true,
            network_block: true,
        });
        let mut sink = |_: crate::events::ToolEventMessage| {};
        let mut asked = 0usize;
        let mut refuse = |_: crate::permissions::ApprovalRequest| {
            asked += 1;
            crate::permissions::ApprovalResponse::Denied { reason: None }
        };
        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["touch", ".git/hooks/pre-commit"]),
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut refuse,
        )
        .unwrap_err();
        assert_eq!(asked, 1, "touch .git/hooks ran without approval");
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
        assert!(!dir.path().join(".git/hooks/pre-commit").exists());
    }

    #[test]
    fn git_checkout_of_an_existing_path_is_destructive() {
        let (_dir, workspace) = repo_with_git_dir();
        let root = workspace.root().to_path_buf();
        for argv in [
            cv(&["git", "checkout", "src/a.rs"]),
            cv(&["git", "checkout", "src"]),
            cv(&["git", "checkout", "HEAD", "src/a.rs"]),
        ] {
            assert_eq!(
                escalate_for_paths(classify(&argv), &argv, &workspace, &root),
                CommandClass::Destructive,
                "{argv:?}"
            );
        }
        // A branch name (nothing by that name on disk) is a plain checkout.
        for argv in [
            cv(&["git", "checkout", "main"]),
            cv(&["git", "checkout", "feature/x"]),
            cv(&["git", "checkout", "-b", "novo"]),
        ] {
            assert_eq!(
                escalate_for_paths(classify(&argv), &argv, &workspace, &root),
                CommandClass::Write,
                "{argv:?}"
            );
        }
    }

    #[test]
    fn git_read_commands_run_with_the_hardening_the_git_tools_use() {
        let hardened = |args: &[&str]| harden_git(&cv(args));
        // No program in the repository's config can be named for `status`.
        assert_eq!(
            hardened(&["git", "status", "--short"]),
            cv(&["-c", "core.fsmonitor=false", "status", "--short"])
        );
        for subcommand in ["diff", "log", "show"] {
            assert_eq!(
                hardened(&["git", subcommand, "HEAD~1"]),
                cv(&[
                    "-c",
                    "core.fsmonitor=false",
                    subcommand,
                    "--no-ext-diff",
                    "--no-textconv",
                    "HEAD~1"
                ])
            );
        }
        // Other programs and other subcommands are left alone.
        assert_eq!(hardened(&["ls", "-la"]), cv(&["-la"]));
        assert_eq!(
            hardened(&["git", "add", "-A"]),
            cv(&["-c", "core.fsmonitor=false", "add", "-A"])
        );
    }

    #[test]
    fn huge_timeout_from_the_model_does_not_panic() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (_, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: test_argv::echo("ok"),
                cwd: None,
                timeout_ms: Some(u64::MAX),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn cancel_kills_running_command() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let cancel = CancelToken::default();
        engine.set_cancel(cancel.clone());

        let mut seen = Vec::new();
        let mut sink = |message: crate::events::ToolEventMessage| seen.push(message);

        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            cancel.cancel();
        });

        let started = Instant::now();
        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: test_argv::sleep_secs(5),
                cwd: None,
                timeout_ms: Some(30_000),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap_err();
        let elapsed = started.elapsed();
        canceller.join().unwrap();

        assert!(matches!(err, ToolError::Cancelled));
        assert!(
            elapsed < Duration::from_secs(3),
            "cancelling must not wait for the command: took {elapsed:?}"
        );
        let completed = seen
            .iter()
            .rev()
            .find_map(|message| match &message.event {
                crate::events::ToolEvent::CommandCompleted { exit_code, .. } => Some(*exit_code),
                _ => None,
            })
            .expect("a cancelled command still reports CommandCompleted");
        assert!(completed.is_none());
    }

    #[test]
    fn denied_command_does_not_run() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};
        let mut deny = |_: crate::permissions::ApprovalRequest| {
            crate::permissions::ApprovalResponse::Denied { reason: None }
        };

        // `touch` is class write, so it is denied on every platform before anything is spawned.
        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["touch", "nao-roda.txt"]),
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut deny,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
        assert!(!dir.path().join("nao-roda.txt").exists());
    }

    #[test]
    fn compound_hiding_behind_a_validate_command_is_refused() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        // The automatic path must not be reachable by pairing a validate command with a dangerous
        // one: compound argv is refused before the class is even consulted (D2).
        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["bun", "test", "&&", "rm", "-rf", "build"]),
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut never_asked,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::CompoundCommand));
    }

    #[test]
    fn unknown_cwd_rejected() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: test_argv::echo("x"),
                cwd: Some("nao/existe".into()),
                timeout_ms: None,
            },
            &mut sink,
            &mut approve,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::NotADirectory));
    }

    #[test]
    fn running_command_runs_inside_workspace_cwd() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("marcador.txt"), "x").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (_, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: test_argv::list_cwd(),
                cwd: None,
                timeout_ms: Some(2000),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.contains("marcador.txt"));
    }

    #[test]
    fn auto_mode_write_command_runs_without_approval_when_fs_sandbox_is_on() {
        let dir = tempdir().unwrap();
        let ws = crate::workspace::Workspace::open(dir.path()).unwrap();
        let mut engine =
            ToolEngine::with_mode(ws, "task_cmd", crate::permissions::PermissionMode::Auto);
        engine.set_sandbox_caps(crate::sandbox::SandboxCapabilities {
            filesystem: true,
            network_block: true,
        });
        let mut asked = 0usize;
        let mut count = |_: crate::permissions::ApprovalRequest| {
            asked += 1;
            crate::permissions::ApprovalResponse::Granted
        };
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (decision, _) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["git", "add", "-A"]),
                cwd: None,
                timeout_ms: Some(30_000),
            },
            &mut sink,
            &mut count,
        )
        .unwrap();

        assert_eq!(asked, 0);
        assert_eq!(decision, PermissionDecision::Auto);
    }

    #[test]
    fn auto_mode_write_command_asks_without_a_filesystem_sandbox() {
        let dir = tempdir().unwrap();
        let ws = crate::workspace::Workspace::open(dir.path()).unwrap();
        let mut engine =
            ToolEngine::with_mode(ws, "task_cmd", crate::permissions::PermissionMode::Auto);
        engine.set_sandbox_caps(crate::sandbox::SandboxCapabilities {
            filesystem: false,
            network_block: false,
        });
        let mut asked = 0usize;
        let mut count = |_: crate::permissions::ApprovalRequest| {
            asked += 1;
            crate::permissions::ApprovalResponse::Granted
        };
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let _ = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["git", "add", "-A"]),
                cwd: None,
                timeout_ms: Some(30_000),
            },
            &mut sink,
            &mut count,
        );

        assert_eq!(asked, 1);
    }

    #[test]
    fn auto_mode_still_asks_for_network_commands() {
        let dir = tempdir().unwrap();
        let ws = crate::workspace::Workspace::open(dir.path()).unwrap();
        let mut engine =
            ToolEngine::with_mode(ws, "task_cmd", crate::permissions::PermissionMode::Auto);
        engine.set_sandbox_caps(crate::sandbox::SandboxCapabilities {
            filesystem: true,
            network_block: true,
        });
        let mut asked = 0usize;
        let mut count = |_: crate::permissions::ApprovalRequest| {
            asked += 1;
            crate::permissions::ApprovalResponse::Denied { reason: None }
        };
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: cv(&["curl", "-s", "http://127.0.0.1"]),
                cwd: None,
                timeout_ms: Some(2_000),
            },
            &mut sink,
            &mut count,
        )
        .unwrap_err();

        assert_eq!(asked, 1);
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
    }
}
