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

    let result = read_resolved(
        &canonical,
        display_path(&engine.workspace, &canonical),
        &args,
        secret_kind,
    )?;
    engine.emit(
        events,
        crate::events::ToolEvent::FileRead {
            path: result.path.clone(),
            start_line: result.start_line,
            line_count: read_line_count(&result),
            total_lines: result.total_lines,
            truncated: result.is_truncated,
            redacted: result.redacted,
        },
    );

    Ok((decision, result))
}

fn read_line_count(result: &ReadFileResult) -> u64 {
    if result.text.is_empty() {
        0
    } else {
        result
            .end_line
            .saturating_sub(result.start_line)
            .saturating_add(1)
    }
}

/// Path-resolved read used by the sequential tool and by parallel batches (plan 023).
pub(crate) fn read_resolved(
    canonical: &Path,
    relative: String,
    args: &ReadFileArgs,
    secret_kind: Option<redactor::SecretKind>,
) -> Result<ReadFileResult, ToolError> {
    let content = read_utf8_lossy(canonical)?;
    let total_lines = content.lines().count() as u64;

    if let Some(kind) = secret_kind {
        let view = redactor::secret_file_view(&kind, &content);
        return Ok(ReadFileResult {
            path: relative,
            text: String::new(),
            start_line: 0,
            end_line: 0,
            total_lines,
            is_truncated: false,
            redacted: total_lines as usize,
            secret: Some(view),
        });
    }

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
    let end_line = start_line + delivered.saturating_sub(1);

    Ok(ReadFileResult {
        path: relative,
        text: redacted.text,
        start_line,
        end_line,
        total_lines,
        is_truncated,
        redacted: redacted_count,
        secret: None,
    })
}

/// Independent non-secret reads, in request order. The caller still authorizes and emits.
pub(crate) fn read_many(
    workspace: &crate::workspace::Workspace,
    args: Vec<ReadFileArgs>,
) -> Vec<Result<ReadFileResult, ToolError>> {
    if args.len() < 2 {
        return args
            .into_iter()
            .map(|item| read_one(workspace, item))
            .collect();
    }
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(args.len());
        for item in args {
            handles.push(scope.spawn(move || read_one(workspace, item)));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    })
}

fn read_one(
    workspace: &crate::workspace::Workspace,
    args: ReadFileArgs,
) -> Result<ReadFileResult, ToolError> {
    let canonical = workspace.resolve(&args.path)?;
    let secret_kind = redactor::detect_path_secret(&canonical);
    read_resolved(
        &canonical,
        display_path(workspace, &canonical),
        &args,
        secret_kind,
    )
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

    /// Phase 13 baseline: independent reads in one turn are sequential today. The numbers
    /// here justify (or reject) a parallel batch in the loop.
    #[test]
    fn measure_sequential_vs_parallel_independent_reads() {
        use std::time::Instant;

        let dir = tempdir().unwrap();
        let paths: Vec<String> = (0..16)
            .map(|i| {
                let name = format!("f{i:02}.txt");
                // 64 KiB each: large enough for I/O to show, small enough for the test.
                std::fs::write(dir.path().join(&name), vec![b'x'; 64 * 1024]).unwrap();
                name
            })
            .collect();

        let mut engine = boot(dir.path());
        let started = Instant::now();
        for name in &paths {
            let _ = read_file(
                &mut engine,
                ReadFileArgs {
                    path: name.clone(),
                    start_line: None,
                    end_line: None,
                },
                &mut no_events(),
                &mut grant(),
            )
            .unwrap();
        }
        let sequential = started.elapsed();

        let started = Instant::now();
        let loaded = crate::tools::read::read_many(
            &engine.workspace,
            paths
                .iter()
                .map(|name| ReadFileArgs {
                    path: name.clone(),
                    start_line: None,
                    end_line: None,
                })
                .collect(),
        );
        let parallel = started.elapsed();
        assert_eq!(loaded.len(), 16);
        assert!(loaded.iter().all(|item| item.is_ok()));

        eprintln!("phase13 reads: sequential={sequential:?} parallel={parallel:?}");
        // Ceiling, not a microbenchmark: Phase 13 was ~2.1 s before the linear redactor.
        // Shared CI/VMs land around 50–250 ms; 500 ms still flags a regression to the old path.
        assert!(
            sequential.as_millis() < 500,
            "sequential reads after linear redact: {sequential:?}"
        );
        assert!(
            parallel.as_millis() < 500,
            "parallel reads hung: {parallel:?}"
        );
    }
}
