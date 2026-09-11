use std::io::Write;
use std::path::Path;

use similar::{ChangeTag, TextDiff};

use crate::permissions::{ApprovalAction, PermissionDecision};
use crate::tools::{
    EditFileArgs, EditFileResult, EventSink, IfExists, Responder, ToolEngine, ToolError,
    WriteFileArgs, WriteFileResult, display_path, read_utf8_lossy, sha256_hex,
};

/// Where and what to replace: `(start, end, old_block, new_block, fuzzy)`.
type PendingEdit = (usize, usize, String, String, bool);

/// edit_file: same-block replace with exact → fuzzy (whitespace-tolerant) matching,
/// syntax validation and an atomic write (design D3/D5/D9). Every write asks for approval.
pub fn edit_file(
    engine: &mut ToolEngine,
    args: EditFileArgs,
    events: EventSink,
    responder: Responder,
) -> Result<(PermissionDecision, EditFileResult), ToolError> {
    let canonical = engine.workspace.resolve(&args.path)?;
    let original = read_utf8_lossy(&canonical)?;
    let hash_before = sha256_hex(original.as_bytes());

    let apply = apply_edit(&original, &args.old_text, &args.new_text)?;
    let Some((start, end, old_block, new_block, fuzzy)) = apply else {
        return Err(ToolError::EditNotFound);
    };

    let mut content = original.clone();
    content.replace_range(start..end, &new_block);
    parse_check(&canonical, &content)?;
    let diff = unified_diff(&original, &content);

    let decision = engine.ask_approval(
        events,
        responder,
        ApprovalAction::EditFile {
            path: display_path(&engine.workspace, &canonical),
            diff,
        },
    );
    if decision == PermissionDecision::Denied {
        return Err(ToolError::PermissionDenied {
            reason: "edição negada".to_string(),
        });
    }

    let hash_after = sha256_hex(content.as_bytes());
    atomic_write(&canonical, content.as_bytes())?;

    let removed = count_lines(&old_block);
    let added = count_lines(&new_block);

    engine.emit(
        events,
        crate::events::ToolEvent::FileChanged {
            path: display_path(&engine.workspace, &canonical),
            diff: unified_diff(&original, &content),
            fuzzy,
            hash_before: hash_before.clone(),
            hash_after: hash_after.clone(),
        },
    );
    engine.emit(
        events,
        crate::events::ToolEvent::CheckpointCreated {
            hash: hash_after.clone(),
        },
    );

    Ok((
        decision,
        EditFileResult {
            path: display_path(&engine.workspace, &canonical),
            changed_lines: removed + added,
            removed,
            added,
            hash_before,
            hash_after,
            fuzzy,
        },
    ))
}

/// write_file: create or overwrite, always via approval, with syntax validation and
/// an atomic write. `ifExists: "overwrite"` is required to replace an existing file.
pub fn write_file(
    engine: &mut ToolEngine,
    args: WriteFileArgs,
    events: EventSink,
    responder: Responder,
) -> Result<(PermissionDecision, WriteFileResult), ToolError> {
    if args.content.is_empty() {
        return Err(ToolError::Io("content vazio".to_string()));
    }
    let canonical = engine.workspace.resolve(&args.path)?;

    let exists = fs_meta(&canonical)
        .map(|meta| meta.is_file())
        .unwrap_or(false);
    if exists && args.if_exists == IfExists::Error {
        return Err(ToolError::AlreadyExists);
    }
    if let Some(parent) = canonical
        .parent()
        .filter(|parent| !canonical.exists() && !parent.exists())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| ToolError::Io(format!("{parent:?}: {error}")))?;
    }

    parse_check(&canonical, &args.content)?;

    let decision = engine.ask_approval(
        events,
        responder,
        ApprovalAction::WriteFile {
            path: display_path(&engine.workspace, &canonical),
            size: args.content.len() as u64,
        },
    );
    if decision == PermissionDecision::Denied {
        return Err(ToolError::PermissionDenied {
            reason: "gravação negada".to_string(),
        });
    }

    let hash = sha256_hex(args.content.as_bytes());
    atomic_write(&canonical, args.content.as_bytes())?;

    engine.emit(events, crate::events::ToolEvent::CheckpointCreated { hash });

    Ok((
        decision,
        WriteFileResult {
            path: display_path(&engine.workspace, &canonical),
            size: args.content.len(),
        },
    ))
}

/// Returns `(start, end, old_block, new_block, fuzzy)` for the single replacement.
fn apply_edit(
    original: &str,
    old_text: &str,
    new_text: &str,
) -> Result<Option<PendingEdit>, ToolError> {
    if old_text.is_empty() {
        return Ok(None);
    }
    let occurrences = byte_matches(original, old_text);
    if !occurrences.is_empty() {
        // Design "closest snippet": among several equal blocks, pick the one closest
        // to the end of the file (the most recent spot).
        let (start, end) = occurrences[occurrences.len() - 1];
        return Ok(Some((
            start,
            end,
            old_text.to_string(),
            new_text.to_string(),
            false,
        )));
    }
    // Fuzzy: whitespace-collapsed search (covers re-wrapping of lines).
    if let Some((start, end)) = fuzzy_locate(original, old_text) {
        let old_block = original[start..end].to_string();
        return Ok(Some((start, end, old_block, new_text.to_string(), true)));
    }
    Ok(None)
}

fn byte_matches(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = haystack[cursor..].find(needle) {
        let start = cursor + found;
        let end = start + needle.len();
        matches.push((start, end));
        cursor = end;
    }
    matches
}

struct Token {
    start: usize,
    end: usize,
    text: String,
}

/// Whitespace-delimited tokens with their original byte ranges.
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut scan = text.char_indices().peekable();
    while let Some((idx, ch)) = scan.next() {
        if ch.is_whitespace() {
            continue;
        }
        let start = idx;
        let mut end = idx + ch.len_utf8();
        while let Some(&(next_idx, next_ch)) = scan.peek() {
            if next_ch.is_whitespace() {
                break;
            }
            end = next_idx + next_ch.len_utf8();
            scan.next();
        }
        tokens.push(Token {
            start,
            end,
            text: text[start..end].to_string(),
        });
    }
    tokens
}

/// Whitespace-collapsed window search: the old block must match as a contiguous
/// token run (extra words break the match); a single unique window is required.
fn fuzzy_locate(original: &str, want: &str) -> Option<(usize, usize)> {
    let want_tokens = tokenize(want);
    if want_tokens.is_empty() {
        return None;
    }
    let text = tokenize(original);
    if text.len() < want_tokens.len() {
        return None;
    }
    let mut found: Option<(usize, usize)> = None;
    for window in text.windows(want_tokens.len()) {
        let matches = window
            .iter()
            .zip(want_tokens.iter())
            .all(|(token, wanted)| token.text == wanted.text);
        if matches {
            let span = (window[0].start, window[window.len() - 1].end);
            if found.is_some() {
                return None; // ambiguous even after collapsing whitespace
            }
            found = Some(span);
        }
    }
    found
}

fn fs_meta(path: &Path) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(path).ok()
}

/// Syntax check for the languages the model can edit (design D3).
fn parse_check(path: &Path, content: &str) -> Result<(), ToolError> {
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let (kind, language) = match extension {
        "tsx" => (
            "typescript",
            tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TSX),
        ),
        "rs" => (
            "rust",
            tree_sitter::Language::new(tree_sitter_rust::LANGUAGE),
        ),
        "ts" | "js" | "jsx" => (
            "typescript",
            tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
        ),
        _ => return Ok(()),
    };

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| ToolError::Io(format!("{kind}: {error}")))?;
    let tree = parser
        .parse(content, None)
        .ok_or_else(|| ToolError::ParseFailed {
            detail: "não foi possível produzir uma árvore".into(),
            line: 1,
        })?;

    if let Some(node) = find_error(tree.root_node()) {
        let position = node.start_position();
        let snippet = node
            .utf8_text(content.as_bytes())
            .unwrap_or("?")
            .to_string();
        return Err(ToolError::ParseFailed {
            detail: format!("{} perto de {:?}", node.kind(), snippet),
            line: position.row + 1,
        });
    }
    Ok(())
}

fn find_error<'t>(node: tree_sitter::Node<'t>) -> Option<tree_sitter::Node<'t>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_error(child) {
            return Some(found);
        }
    }
    None
}

/// Unified diff (from `similar`) in the `---- before / ++++ after` shape (design D1).
fn unified_diff(before: &str, after: &str) -> String {
    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    let mut header_pending = true;
    for change in diff.iter_all_changes() {
        if change.tag() == ChangeTag::Equal {
            continue;
        }
        if header_pending {
            out.push_str("--- before\n+++ after\n");
            header_pending = false;
        }
        let sign = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        out.push(sign);
        out.push_str(change.value());
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count()
    }
}

/// Atomic write: sibling temp file, write, fsync, rename (design D5).
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ToolError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    for attempt in 1..=64 {
        let tmp = parent.join(format!(".{file_name}.{}.{attempt}.tmp", std::process::id()));
        let mut file = match std::fs::File::create(&tmp) {
            Ok(file) => file,
            Err(_) => continue,
        };
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                let _ = std::fs::remove_file(&tmp);
                ToolError::Io(error.to_string())
            })?;
        return std::fs::rename(&tmp, path).map_err(|error| ToolError::Io(error.to_string()));
    }
    Err(ToolError::Io(
        "não foi possível escrever o arquivo".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn boot(dir: &Path) -> ToolEngine {
        let ws = crate::workspace::Workspace::open(dir).unwrap();
        ToolEngine::new(ws, "task_edit")
    }

    fn approve(
        _request: crate::permissions::ApprovalRequest,
    ) -> crate::permissions::ApprovalResponse {
        crate::permissions::ApprovalResponse::Granted
    }

    fn deny(_request: crate::permissions::ApprovalRequest) -> crate::permissions::ApprovalResponse {
        crate::permissions::ApprovalResponse::Denied {
            reason: Some("não".into()),
        }
    }

    #[test]
    fn exact_unique_replacement() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "um\ndois\ntres\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (decision, result) = edit_file(
            &mut engine,
            EditFileArgs {
                path: "x.txt".into(),
                old_text: "dois".into(),
                new_text: "DOIS".into(),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert_eq!(decision, PermissionDecision::Granted);
        assert!(!result.fuzzy);
        assert_eq!(result.removed, 1);
        assert_eq!(result.added, 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.txt")).unwrap(),
            "um\nDOIS\ntres\n"
        );
    }

    #[test]
    fn closest_snippet_wins_on_repeated_block() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "erro\nmeio\nerro\nfim\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (_, result) = edit_file(
            &mut engine,
            EditFileArgs {
                path: "x.txt".into(),
                old_text: "erro".into(),
                new_text: "OK".into(),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert!(!result.fuzzy);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.txt")).unwrap(),
            "erro\nmeio\nOK\nfim\n"
        );
    }

    #[test]
    fn fuzzy_whitespace_replacement() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "let value = 1;\nlet other = 2;\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (_, result) = edit_file(
            &mut engine,
            EditFileArgs {
                path: "x.txt".into(),
                old_text: "let\nvalue = 1;".into(), // line-wrapped, same tokens
                new_text: "let value = 42;".into(),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert!(result.fuzzy);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.txt")).unwrap(),
            "let value = 42;\nlet other = 2;\n"
        );
    }

    #[test]
    fn fuzzy_ignores_extra_words() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "a b c d\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};
        // "a c" is not a contiguous token run in "a b c d": no match, erroring safely.
        let err = edit_file(
            &mut engine,
            EditFileArgs {
                path: "x.txt".into(),
                old_text: "a\nc".into(),
                new_text: "X".into(),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::EditNotFound));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.txt")).unwrap(),
            "a b c d\n"
        );
    }

    #[test]
    fn edit_not_found() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "abc\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = edit_file(
            &mut engine,
            EditFileArgs {
                path: "x.txt".into(),
                old_text: "xyz".into(),
                new_text: "q".into(),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::EditNotFound));
    }

    #[test]
    fn parse_error_blocks_write() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.ts"), "const a = 1;\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = edit_file(
            &mut engine,
            EditFileArgs {
                path: "x.ts".into(),
                old_text: "const a = 1;".into(),
                new_text: "const a = ;".into(),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::ParseFailed { .. }));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.ts")).unwrap(),
            "const a = 1;\n"
        );
    }

    #[test]
    fn write_file_refuses_overwrite_unless_requested() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("existe.txt"), "velho\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let err = write_file(
            &mut engine,
            WriteFileArgs {
                path: "existe.txt".into(),
                content: "novo\n".into(),
                if_exists: IfExists::Error,
            },
            &mut sink,
            &mut approve,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::AlreadyExists));

        let (_, result) = write_file(
            &mut engine,
            WriteFileArgs {
                path: "existe.txt".into(),
                content: "novo\n".into(),
                if_exists: IfExists::Overwrite,
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert_eq!(result.size, 5);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("existe.txt")).unwrap(),
            "novo\n"
        );
    }

    #[test]
    fn write_creates_parent_dirs() {
        let dir = tempdir().unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        write_file(
            &mut engine,
            WriteFileArgs {
                path: "src/nested/novo.md".into(),
                content: "# titulo\n".into(),
                if_exists: IfExists::Error,
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert!(dir.path().join("src/nested/novo.md").is_file());
    }

    #[test]
    fn denied_edit_writes_nothing() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "original\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};
        let err = edit_file(
            &mut engine,
            EditFileArgs {
                path: "x.txt".into(),
                old_text: "original".into(),
                new_text: "mudado".into(),
            },
            &mut sink,
            &mut deny,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied { .. }));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.txt")).unwrap(),
            "original\n"
        );
    }

    #[test]
    fn rust_edit_allowed() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
        let mut engine = boot(dir.path());
        let mut sink = |_: crate::events::ToolEventMessage| {};

        let (_, result) = edit_file(
            &mut engine,
            EditFileArgs {
                path: "lib.rs".into(),
                old_text: "fn main() {}".into(),
                new_text: "fn main_a() {}\nfn main() {}".into(),
            },
            &mut sink,
            &mut approve,
        )
        .unwrap();
        assert!(result.added >= 1);
    }
}
