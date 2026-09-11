use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::permissions::{ApprovalAction, PermissionDecision, classify};
use crate::redactor;
use crate::tools::{
    CommandResult, DEFAULT_COMMAND_TIMEOUT_MS, EventSink, MAX_OUTPUT_BYTES, Responder,
    RunCommandArgs, ToolEngine, ToolError,
};

/// run_command: argv only, never a shell (design D2). Every command asks the user,
/// with the exact argv; the class is part of the approval (design D5/D8).
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

    let class = classify(argv);
    let decision = engine.ask_approval(
        events,
        responder,
        ApprovalAction::RunCommand {
            argv: argv.clone(),
            class: class.clone(),
            cwd: crate::tools::display_path(&engine.workspace, &cwd),
        },
    );
    if decision == PermissionDecision::Denied {
        return Err(ToolError::PermissionDenied {
            reason: "comando negado".to_string(),
        });
    }

    let id = engine.command_next_id;
    engine.command_next_id += 1;
    engine.emit(
        events,
        crate::events::ToolEvent::CommandStarted {
            id,
            argv: argv.clone(),
            class,
        },
    );

    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS));
    let started = Instant::now();
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    platform_spawn_setup(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| ToolError::Io(format!("não foi possível executar: {error}")))?;

    let status = wait_with_timeout(&mut child, timeout, MAX_OUTPUT_BYTES);
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

struct RunStatus {
    exit_code: Option<i32>,
    timed_out: bool,
    output_capped: bool,
    output: String,
}

/// Polls the child, killing the whole tree on timeout. Readers drain the pipes on
/// background threads so a chatty or quiet child never deadlocks the timeout.
fn wait_with_timeout(child: &mut Child, timeout: Duration, cap: usize) -> RunStatus {
    let stdout = child
        .stdout
        .take()
        .map(|pipe| thread::spawn(move || drain_capped(pipe, cap)));
    let stderr = child
        .stderr
        .take()
        .map(|pipe| thread::spawn(move || drain_capped(pipe, cap)));

    let deadline = Instant::now() + timeout;
    let exit = loop {
        if let Some(status) = child.try_wait().unwrap_or(None) {
            break status.code();
        }
        if Instant::now() >= deadline {
            break None; // timed out: caller kills and notifies
        }
        thread::sleep(Duration::from_millis(10));
    };

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

    let timed_out = exit.is_none();
    if timed_out {
        kill_process_tree(child);
    }
    RunStatus {
        exit_code: exit,
        timed_out,
        output_capped,
        output: text,
    }
}

fn drain_capped(pipe: impl Read, cap: usize) -> Vec<u8> {
    let mut reader = std::io::BufReader::new(pipe);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() >= cap {
                    break;
                }
            }
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

#[cfg(unix)]
fn kill_process_tree(child: &Child) {
    let pgid = child.id() as i64;
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pgid}")])
        .status();
}

#[cfg(not(unix))]
fn kill_process_tree(child: &Child) {
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .status();
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

    fn approve(
        _request: crate::permissions::ApprovalRequest,
    ) -> crate::permissions::ApprovalResponse {
        crate::permissions::ApprovalResponse::Granted
    }

    #[test]
    fn runs_echo_and_reports_exit() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (decision, result) = run_command(
            &mut engine,
            RunCommandArgs {
                argv: vec!["echo".into(), "ola".into()],
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert_eq!(decision, PermissionDecision::Granted);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.contains("ola"));
        assert!(!result.timed_out);
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
                argv: vec!["sleep".into(), "5".into()],
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

    #[test]
    fn denied_command_does_not_run() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};
        let mut deny = |_: crate::permissions::ApprovalRequest| {
            crate::permissions::ApprovalResponse::Denied { reason: None }
        };

        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: vec!["echo".into(), "nao-roda".into()],
                cwd: None,
                timeout_ms: None,
            },
            &mut sink,
            &mut deny,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
    }

    #[test]
    fn unknown_cwd_rejected() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = run_command(
            &mut engine,
            RunCommandArgs {
                argv: vec!["echo".into(), "x".into()],
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
                argv: vec!["ls".into()],
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
}
