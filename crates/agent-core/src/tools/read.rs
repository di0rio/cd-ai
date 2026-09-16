use std::fs;
use std::path::Path;

use crate::permissions::{ApprovalAction, PermissionDecision, PermissionKind};
use crate::redactor;
use crate::tools::{
    DirEntry, EventSink, ListDirectoryArgs, ListDirectoryResult, MAX_LIST_ENTRIES, MAX_READ_BYTES,
    MAX_READ_LINES, ReadFileArgs, ReadFileResult, Responder, ToolEngine, ToolError, display_path,
    read_utf8_lossy,
};

/// read_file: resolved path, secret files require approval and return only the anonymous view.
pub fn read_file(
    engine: &mut ToolEngine,
    args: ReadFileArgs,
    events: EventSink,
    responder: Responder,
) -> Result<(PermissionDecision, ReadFileResult), ToolError> {
    let canonical = engine.workspace.resolve(&args.path)?;
    let secret_kind = redactor::detect_path_secret(&canonical);

    let decision = engine.authorize(
        events,
        responder,
        PermissionKind::ReadFile {
            secret: secret_kind.is_some(),
        },
        ApprovalAction::ReadFile {
            path: display_path(&engine.workspace, &canonical),
        },
    );
    if decision == PermissionDecision::Denied {
        return Err(ToolError::PermissionDenied {
            reason: "leitura de arquivo de secret negada".to_string(),
        });
    }

    let content = read_utf8_lossy(&canonical)?;
    let total_lines = content.lines().count() as u64;

    if let Some(kind) = secret_kind {
        // Even approved, a secret file returns only the anonymous view (design §6.4).
        let view = redactor::secret_file_view(&kind, &content);
        engine.emit(
            events,
            crate::events::ToolEvent::FileRead {
                path: display_path(&engine.workspace, &canonical),
                start_line: 0,
                line_count: 0,
                total_lines,
                truncated: false,
                redacted: total_lines as usize,
            },
        );
        return Ok((
            decision,
            ReadFileResult {
                path: display_path(&engine.workspace, &canonical),
                text: String::new(),
                start_line: 0,
                end_line: 0,
                total_lines,
                is_truncated: false,
                redacted: total_lines as usize,
                secret: Some(view),
            },
        ));
    }

    // Normal read: 1-indexed lines, optional window, hard caps (design §2.1).
    let start_line = args.start_line.unwrap_or(1).max(1);
    let mut end_line = args.end_line.unwrap_or(u64::MAX);
    if end_line < start_line {
        return Err(ToolError::Io("endLine menor que startLine".to_string()));
    }
    let limit_end = start_line.saturating_add(MAX_READ_LINES - 1);
    if end_line > limit_end {
        end_line = limit_end;
    }

    let mut text = String::new();
    let mut delivered = 0u64;
    let mut bytes = 0usize;
    let mut line_number = 0u64;
    for line in content.lines() {
        line_number += 1;
        if line_number < start_line {
            continue;
        }
        if line_number > end_line {
            break;
        }
        let next_bytes = bytes + line.len() + 1;
        if next_bytes > MAX_READ_BYTES {
            break;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line);
        bytes = next_bytes;
        delivered += 1;
    }

    let redacted = redactor::redact(&text);
    let redacted_count = redacted.count();
    let is_truncated = lines_in(&text) < lines_in(&content);

    engine.emit(
        events,
        crate::events::ToolEvent::FileRead {
            path: display_path(&engine.workspace, &canonical),
            start_line,
            line_count: delivered,
            total_lines,
            truncated: is_truncated,
            redacted: redacted_count,
        },
    );

    let end_line = start_line + delivered.saturating_sub(1);

    Ok((
        decision,
        ReadFileResult {
            path: display_path(&engine.workspace, &canonical),
            text: redacted.text,
            start_line,
            end_line,
            total_lines,
            is_truncated,
            redacted: redacted_count,
            secret: None,
        },
    ))
}

fn lines_in(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// list_directory: non-recursive, sorted, symlinks never followed (design §2.5).
pub fn list_directory(
    engine: &mut ToolEngine,
    args: ListDirectoryArgs,
) -> Result<(ListDirectoryResult, usize), ToolError> {
    let canonical = engine.workspace.resolve(&args.path)?;
    let metadata = fs::metadata(&canonical).map_err(map_io_fs(&canonical))?;
    if !metadata.is_dir() {
        return Err(ToolError::NotADirectory);
    }
    let mut names: Vec<String> = Vec::new();
    for entry in fs::read_dir(&canonical).map_err(|error| ToolError::Io(error.to_string()))? {
        let entry = entry.map_err(|error| ToolError::Io(error.to_string()))?;
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| ToolError::Io(error.to_string()))?;
        // A symlink is reported with both booleans false so the agent cannot walk through it.
        names.push(entry.file_name().to_string_lossy().into_owned());
        let _ = metadata;
    }
    names.sort();

    let entries: Vec<DirEntry> = names
        .into_iter()
        .map(|name| {
            let path = canonical.join(&name);
            let metadata = fs::symlink_metadata(&path);
            let (is_dir, is_file, size) = match metadata {
                Ok(metadata) => {
                    let kind = metadata.file_type();
                    (kind.is_dir(), kind.is_file(), metadata.len())
                }
                Err(_) => (false, false, 0),
            };
            DirEntry {
                name,
                is_dir,
                is_file,
                size,
            }
        })
        .collect();

    let total_scanned = entries.len();
    let shown = entries.into_iter().take(MAX_LIST_ENTRIES).collect();
    Ok((
        ListDirectoryResult {
            path: display_path(&engine.workspace, &canonical),
            entries: shown,
        },
        total_scanned,
    ))
}

fn map_io_fs(_path: &Path) -> impl FnOnce(std::io::Error) -> ToolError {
    move |error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ToolError::NotFound
        } else {
            ToolError::Io(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn boot(dir: &std::path::Path) -> ToolEngine {
        let ws = crate::workspace::Workspace::open(dir).unwrap();
        ToolEngine::new(ws, "task_test")
    }

    fn grant()
    -> impl FnMut(crate::permissions::ApprovalRequest) -> crate::permissions::ApprovalResponse {
        |_| crate::permissions::ApprovalResponse::Granted
    }

    fn deny()
    -> impl FnMut(crate::permissions::ApprovalRequest) -> crate::permissions::ApprovalResponse {
        |_| crate::permissions::ApprovalResponse::Denied { reason: None }
    }

    fn no_events() -> impl FnMut(crate::events::ToolEventMessage) {
        |_| {}
    }

    #[test]
    fn reads_lines_and_windows() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "l1\nl2\nl3\nl4").unwrap();
        let mut engine = boot(dir.path());

        let (_, result) = read_file(
            &mut engine,
            ReadFileArgs {
                path: "file.txt".into(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap();
        assert_eq!(result.total_lines, 4);
        assert_eq!(result.text, "l1\nl2\nl3\nl4");
        assert!(!result.is_truncated);

        let (_, window) = read_file(
            &mut engine,
            ReadFileArgs {
                path: "file.txt".into(),
                start_line: Some(2),
                end_line: Some(3),
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap();
        assert_eq!(window.text, "l2\nl3");
        assert_eq!(window.start_line, 2);
        assert_eq!(window.end_line, 3);
    }

    #[test]
    fn missing_file_is_not_found() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let err = read_file(
            &mut engine,
            ReadFileArgs {
                path: "nope.txt".into(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap_err();
        assert_eq!(err, ToolError::NotFound);
    }

    #[test]
    fn outside_workspace_blocked_before_any_io() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let err = read_file(
            &mut engine,
            ReadFileArgs {
                path: outside.path().join("x.txt").to_string_lossy().into_owned(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::OutsideWorkspace));
    }

    #[test]
    fn truncates_large_read() {
        let dir = tempdir().unwrap();
        let mut content = String::new();
        for i in 0..(MAX_READ_LINES + 50) {
            content.push_str(&format!("linha número {i}\n"));
        }
        std::fs::write(dir.path().join("grande.txt"), &content).unwrap();
        let mut engine = boot(dir.path());
        let (_, result) = read_file(
            &mut engine,
            ReadFileArgs {
                path: "grande.txt".into(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap();
        assert!(result.is_truncated);
        assert_eq!(result.end_line, MAX_READ_LINES);
        assert!(result.total_lines > MAX_READ_LINES);
    }

    #[test]
    fn secret_file_requires_approval_and_returns_view() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "TOKEN=sk_live_abcdef\nP=123\n").unwrap();
        let mut denied_engine = boot(dir.path());

        let err = read_file(
            &mut denied_engine,
            ReadFileArgs {
                path: ".env".into(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut deny(),
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));

        let mut engine = boot(dir.path());
        let (decision, result) = read_file(
            &mut engine,
            ReadFileArgs {
                path: ".env".into(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap();
        assert_eq!(decision, PermissionDecision::Granted);
        // Values never reach the output, even when approved.
        assert!(!result.text.contains("sk_live"));
        let view = result.secret.clone().expect("secret view");
        match view {
            crate::redactor::SecretFileView::DotEnv { keys } => {
                assert_eq!(keys.len(), 2);
                assert!(keys.iter().all(|k| !k.key.contains("sk_")));
            }
            _ => panic!("esperado DotEnv"),
        }
    }

    #[test]
    fn redacts_tokens_in_plain_content() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("log.txt"), "falhou com ghp_abcDEF123xyz\n").unwrap();
        let mut engine = boot(dir.path());
        let (_, result) = read_file(
            &mut engine,
            ReadFileArgs {
                path: "log.txt".into(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap();
        assert!(!result.text.contains("ghp_abcDEF123"));
        assert!(result.text.contains("[REDIGIDO:"));
        assert_eq!(result.redacted, 1);
    }

    #[test]
    fn lists_directory_sorted() {
        let dir = tempdir().unwrap();
        for name in ["b.txt", "a.txt", "sub"] {
            let path = dir.path().join(name);
            if name == "sub" {
                std::fs::create_dir(&path).unwrap();
            } else {
                std::fs::write(&path, "x").unwrap();
            }
        }
        let mut engine = boot(dir.path());
        let (result, total) =
            list_directory(&mut engine, ListDirectoryArgs { path: ".".into() }).unwrap();
        let names: Vec<_> = result.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "b.txt", "sub"]);
        assert_eq!(total, 3);
        assert!(result.entries[0].is_file);
        assert!(result.entries[2].is_dir);
    }

    #[cfg(unix)]
    #[test]
    fn lists_symlink_but_does_not_follow() {
        use std::os::unix::fs::symlink;
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), "x").unwrap();
        symlink(dir.path().join("real.txt"), dir.path().join("link.txt")).unwrap();
        let mut engine = boot(dir.path());
        let (result, _) =
            list_directory(&mut engine, ListDirectoryArgs { path: ".".into() }).unwrap();
        let link = result
            .entries
            .iter()
            .find(|e| e.name == "link.txt")
            .unwrap();
        assert!(!link.is_file && !link.is_dir);
    }

    #[test]
    fn directory_input_is_not_a_file() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pasta")).unwrap();
        let mut engine = boot(dir.path());
        let err = read_file(
            &mut engine,
            ReadFileArgs {
                path: "pasta".into(),
                start_line: None,
                end_line: None,
            },
            &mut no_events(),
            &mut grant(),
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::NotAFile));
    }

    #[test]
    fn full_engine_run_emits_events() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "hello").unwrap();
        let mut engine = boot(dir.path());
        let mut events = Vec::new();
        {
            let mut sink = |message: crate::events::ToolEventMessage| events.push(message.event);
            let _outcome = engine.run_tool(
                crate::tools::ToolRequest::ReadFile(ReadFileArgs {
                    path: "x.txt".into(),
                    start_line: None,
                    end_line: None,
                }),
                &mut sink,
                &mut grant(),
            );
        }
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                crate::events::ToolEvent::ToolStarted { tool, .. } => tool.as_str(),
                crate::events::ToolEvent::ToolCompleted { tool, .. } => tool.as_str(),
                crate::events::ToolEvent::FileRead { .. } => "fileRead",
                _ => "other",
            })
            .collect();
        assert!(kinds.contains(&"readFile"));
        assert!(kinds.contains(&"fileRead"), "{kinds:?}");
    }
}
