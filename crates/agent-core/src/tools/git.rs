//! Read-only git tools against the user's repository (SPEC §15.2 / §19, plan 019).
//!
//! Argv is fixed: the model cannot pass `--hard`, remotes, or pager tricks. The shadow repo is
//! never these tools' GIT_DIR.

use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::permissions::PermissionDecision;
use crate::redactor;
use crate::sandbox::{SandboxExec, constrain};
use crate::tools::cancel::CancelToken;
use crate::tools::{
    EventSink, GitBranchArgs, GitBranchResult, GitDiffArgs, GitDiffResult, GitLogArgs, GitLogEntry,
    GitLogResult, GitStatusArgs, GitStatusResult, MAX_OUTPUT_BYTES, Responder, ToolEngine,
    ToolError, display_path,
};

const GIT_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_LOG: usize = 20;
const MAX_LOG: usize = 100;

pub fn git_status(
    engine: &mut ToolEngine,
    _args: GitStatusArgs,
    _events: EventSink,
    _responder: Responder,
) -> Result<(PermissionDecision, GitStatusResult), ToolError> {
    let cwd = engine.workspace.root().to_path_buf();
    let output = run_user_git(
        engine,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        &cwd,
    )?;
    Ok((
        PermissionDecision::Auto,
        GitStatusResult {
            output: redact_output(&output),
        },
    ))
}

pub fn git_diff(
    engine: &mut ToolEngine,
    args: GitDiffArgs,
    _events: EventSink,
    _responder: Responder,
) -> Result<(PermissionDecision, GitDiffResult), ToolError> {
    let mut argv = vec![
        "diff".to_string(),
        "--no-ext-diff".to_string(),
        "--no-color".to_string(),
    ];
    if let Some(path) = args.path {
        let canonical = engine.workspace.resolve(&path)?;
        argv.push("--".to_string());
        argv.push(display_path(&engine.workspace, &canonical));
    }
    let cwd = engine.workspace.root().to_path_buf();
    let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let output = run_user_git(engine, &argv_refs, &cwd)?;
    Ok((
        PermissionDecision::Auto,
        GitDiffResult {
            diff: redact_output(&output),
        },
    ))
}

pub fn git_log(
    engine: &mut ToolEngine,
    args: GitLogArgs,
    _events: EventSink,
    _responder: Responder,
) -> Result<(PermissionDecision, GitLogResult), ToolError> {
    let max = args
        .max_count
        .unwrap_or(DEFAULT_LOG as u64)
        .clamp(1, MAX_LOG as u64);
    let count = max.to_string();
    let cwd = engine.workspace.root().to_path_buf();
    let output = run_user_git(
        engine,
        &[
            "log",
            "--no-ext-diff",
            "--no-color",
            "--format=%H%x09%s%x09%cI",
            "-n",
            &count,
        ],
        &cwd,
    )?;
    let mut entries = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        let hash = parts.next().unwrap_or("").to_string();
        let subject = parts.next().unwrap_or("").to_string();
        let at = parts.next().unwrap_or("").to_string();
        entries.push(GitLogEntry {
            hash,
            subject: redactor::redact(&subject).text,
            at,
        });
    }
    Ok((PermissionDecision::Auto, GitLogResult { entries }))
}

pub fn git_branch(
    engine: &mut ToolEngine,
    _args: GitBranchArgs,
    _events: EventSink,
    _responder: Responder,
) -> Result<(PermissionDecision, GitBranchResult), ToolError> {
    let cwd = engine.workspace.root().to_path_buf();
    let output = run_user_git(engine, &["branch", "--list", "--no-color"], &cwd)?;
    Ok((
        PermissionDecision::Auto,
        GitBranchResult {
            output: redact_output(&output),
        },
    ))
}

fn redact_output(text: &str) -> String {
    redactor::redact(text).text
}

fn run_user_git(
    engine: &mut ToolEngine,
    args: &[&str],
    cwd: &std::path::Path,
) -> Result<String, ToolError> {
    if engine.cancel_token().is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_PAGER", "cat")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    constrain(
        &mut command,
        SandboxExec {
            workspace: engine.workspace.root().to_path_buf(),
            allow_network: false,
        },
    );
    let mut child = command.spawn().map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => ToolError::Io("git não encontrado no PATH".to_string()),
        _ => ToolError::Io(format!("não foi possível executar git: {error}")),
    })?;

    let cancel = engine.cancel_token().clone();
    let output = wait_capped(&mut child, GIT_TIMEOUT, MAX_OUTPUT_BYTES, &cancel)?;
    if output.cancelled {
        return Err(ToolError::Cancelled);
    }
    if output.timed_out {
        return Err(ToolError::Io("git excedeu o tempo limite".to_string()));
    }
    if output.exit_code != Some(0) {
        let detail = output.output.trim();
        if detail.contains("not a git repository") {
            return Err(ToolError::Io(
                "este workspace não é um repositório git".to_string(),
            ));
        }
        return Err(ToolError::Io(if detail.is_empty() {
            "git falhou".to_string()
        } else {
            redactor::redact(detail).text
        }));
    }
    Ok(output.output)
}

struct GitRun {
    exit_code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    output: String,
}

fn wait_capped(
    child: &mut std::process::Child,
    timeout: Duration,
    cap: usize,
    cancel: &CancelToken,
) -> Result<GitRun, ToolError> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_h = stdout.map(|pipe| std::thread::spawn(move || drain(pipe, cap)));
    let stderr_h = stderr.map(|pipe| std::thread::spawn(move || drain(pipe, cap)));
    let deadline = Instant::now() + timeout;
    let mut cancelled = false;
    let mut timed_out = false;
    let exit = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| ToolError::Io(error.to_string()))?
        {
            break status.code();
        }
        if cancel.is_cancelled() {
            cancelled = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout_h.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = stderr_h.and_then(|h| h.join().ok()).unwrap_or_default();
    let mut output = stdout;
    if !stderr.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&stderr);
    }
    Ok(GitRun {
        exit_code: exit,
        timed_out,
        cancelled,
        output,
    })
}

fn drain(mut pipe: impl std::io::Read, cap: usize) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let room = cap.saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..n.min(room)]);
                if buf.len() >= cap {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{ApprovalRequest, ApprovalResponse};
    use crate::tools::{ToolEngine, ToolEventMessage, ToolRequest};
    use crate::workspace::Workspace;
    use std::fs;
    use tempfile::tempdir;

    fn git_works() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn run_ok(dir: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn status_on_a_repo_and_error_without_one() {
        if !git_works() {
            return;
        }
        let empty = tempdir().unwrap();
        let ws = Workspace::open(empty.path()).unwrap();
        let mut engine =
            ToolEngine::with_mode(ws, "task_git", crate::permissions::PermissionMode::Auto);
        let mut seen = Vec::new();
        let mut sink = |m: ToolEventMessage| seen.push(m);
        let mut grant = |_: ApprovalRequest| ApprovalResponse::Granted;
        let outcome = engine.run_tool(
            ToolRequest::GitStatus(GitStatusArgs {}),
            &mut sink,
            &mut grant,
        );
        assert!(!outcome.ok);
        let error = outcome
            .error
            .as_ref()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            error.contains("não é um repositório git"),
            "unexpected error: {error}"
        );

        let repo = tempdir().unwrap();
        fs::write(repo.path().join("a.txt"), "x\n").unwrap();
        run_ok(repo.path(), &["init"]);
        run_ok(
            repo.path(),
            &[
                "-c",
                "user.name=dev",
                "-c",
                "user.email=dev@local",
                "-c",
                "commit.gpgsign=false",
                "add",
                "a.txt",
            ],
        );
        run_ok(
            repo.path(),
            &[
                "-c",
                "user.name=dev",
                "-c",
                "user.email=dev@local",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "init",
            ],
        );
        let ws = Workspace::open(repo.path()).unwrap();
        let mut engine =
            ToolEngine::with_mode(ws, "task_git", crate::permissions::PermissionMode::Auto);
        let mut seen = Vec::new();
        let mut sink = |m: ToolEventMessage| seen.push(m);
        let mut grant = |_: ApprovalRequest| ApprovalResponse::Granted;
        let outcome = engine.run_tool(
            ToolRequest::GitLog(GitLogArgs { max_count: Some(5) }),
            &mut sink,
            &mut grant,
        );
        assert!(outcome.ok, "{outcome:?}");
        let crate::tools::ToolOutput::GitLog(result) = outcome.data.unwrap() else {
            panic!("git_log");
        };
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].subject, "init");

        let mut seen = Vec::new();
        let mut sink = |m: ToolEventMessage| seen.push(m);
        let outcome = engine.run_tool(
            ToolRequest::GitStatus(GitStatusArgs {}),
            &mut sink,
            &mut grant,
        );
        assert!(outcome.ok, "{outcome:?}");

        let mut seen = Vec::new();
        let mut sink = |m: ToolEventMessage| seen.push(m);
        let outcome = engine.run_tool(
            ToolRequest::GitDiff(GitDiffArgs { path: None }),
            &mut sink,
            &mut grant,
        );
        assert!(outcome.ok, "{outcome:?}");

        let mut seen = Vec::new();
        let mut sink = |m: ToolEventMessage| seen.push(m);
        let outcome = engine.run_tool(
            ToolRequest::GitBranch(GitBranchArgs {}),
            &mut sink,
            &mut grant,
        );
        assert!(outcome.ok);
    }
}
