//! Deterministic verification and the optional LLM review (SPEC §13, plan 018).
//!
//! The Coder still owns the tools. This module only *judges* what is already on disk: parse,
//! cheap diff checks, then the workspace's validation commands. A sentence from the model is
//! never evidence.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::agent::profile::validation_argv;
use crate::agent::state::FileChange;
use crate::permissions::{CommandClass, classify};
use crate::redactor;
use crate::tools::edit::parse_check;
use crate::workspace::Workspace;

/// How many added+removed diff lines count as "disproportionate" without a plan to compare to.
const MAX_DIFF_LINES: usize = 4_000;
/// How much of a failed command's output travels back to the model.
const MAX_FAIL_OUTPUT_CHARS: usize = 4_000;
/// Diff budget inside the isolated review prompt.
const MAX_REVIEW_DIFF_CHARS: usize = 8_000;

/// Names that are lockfiles or generated manifests. Touching one is a deterministic fail
/// (SPEC §13.1); the model can still explain it after a correction, but it is never silent.
const LOCKFILE_NAMES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "composer.lock",
    "Gemfile.lock",
    "poetry.lock",
    "go.sum",
];

/// Outcome of one verification pass. The loop maps this onto `completed` / `completed_unvalidated`
/// or a correction message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Files changed, parse+diff passed, and a `validate` command exited 0 after the last edit.
    Pass { evidence: Vec<String> },
    /// Something concrete failed. The model may still fix it.
    Fail { reasons: Vec<String> },
    /// Nothing to validate, or no validation command exists. Never a success.
    Unvalidated { why: String },
    /// File checks passed; the loop still has to run these argv through the engine.
    Run { argv: Vec<Vec<String>> },
}

/// Isolated LLM review after a deterministic pass (SPEC §13.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Review {
    Pass,
    ChangesRequired {
        reasons: Vec<String>,
    },
    /// Empty or unparseable: do not block. Scripted eval lands here.
    Skip,
}

/// Snapshot the loop hands the Verifier. Diffs are the last `FileChanged` per path.
pub struct VerifyInput<'a> {
    pub workspace: &'a Workspace,
    pub request: &'a str,
    pub files_changed: &'a [FileChange],
    pub diffs: &'a HashMap<String, String>,
    /// `validate` argv that already exited 0 since the last file change.
    pub passed_validate_since_edit: &'a [Vec<String>],
}

/// File checks (parse + diff) then whether commands still need to run.
pub fn judge_files(input: &VerifyInput<'_>) -> Verdict {
    if input.files_changed.is_empty() {
        return Verdict::Unvalidated {
            why: "nenhum arquivo foi alterado".to_string(),
        };
    }

    let mut reasons = Vec::new();
    reasons.extend(parse_changed_files(input.workspace, input.files_changed));
    reasons.extend(diff_checks(input.files_changed, input.diffs));
    if !reasons.is_empty() {
        return Verdict::Fail { reasons };
    }

    let needed = validation_argv(input.workspace);
    if !input.passed_validate_since_edit.is_empty() {
        let evidence: Vec<String> = input
            .passed_validate_since_edit
            .iter()
            .map(|argv| {
                format!(
                    "{} (já tinha passado depois da última edição)",
                    argv.join(" ")
                )
            })
            .collect();
        return Verdict::Pass { evidence };
    }
    if needed.is_empty() {
        return Verdict::Unvalidated {
            why: "o workspace não declara comandos de validação".to_string(),
        };
    }

    Verdict::Run { argv: needed }
}

/// True when this argv is a validation command that already succeeded after the last edit.
pub fn is_successful_validate(argv: &[String], exit_code: Option<i32>) -> bool {
    exit_code == Some(0) && classify(argv) == CommandClass::Validate
}

/// User message that sends the model back to fix (plan 018, D7). Portuguese: the model answers
/// the user in pt-BR, and this is the next instruction it will see.
pub fn correction_message(attempt: u32, limit: u32, reasons: &[String]) -> String {
    let mut text = format!(
        "A verificação determinística falhou (correção {attempt} de {limit}). \
         O sistema julga por evidência, não pela afirmação de que a tarefa terminou.\n"
    );
    for reason in reasons {
        text.push_str(&format!("- {reason}\n"));
    }
    text.push_str(
        "Não repita a mesma chamada. Declare a hipótese, corrija e só responda sem tools \
         quando os checks tiverem passado.",
    );
    text
}

/// Isolated review prompt: requirement + diff + deterministic results. No coder transcript.
pub fn review_prompt(
    request: &str,
    files: &[FileChange],
    diffs: &HashMap<String, String>,
    evidence: &[String],
) -> String {
    let mut prompt = String::from(
        "You are reviewing a change made by a coding agent. You did not write this code.\n\
         Do not comment on style. Only functional issues, missing edge cases, error handling \
         or obvious regressions.\n\nRequirement:\n",
    );
    prompt.push_str(request.trim());
    prompt.push_str("\n\nChanged files:\n");
    for change in files {
        prompt.push_str(&format!("- {}\n", change.path));
    }
    prompt.push_str("\nDeterministic checks:\n");
    for line in evidence {
        prompt.push_str(&format!("- {line}\n"));
    }
    prompt.push_str("\nDiff:\n");
    let mut used = 0;
    for change in files {
        let Some(diff) = diffs.get(&change.path) else {
            continue;
        };
        let remaining = MAX_REVIEW_DIFF_CHARS.saturating_sub(used);
        if remaining == 0 {
            prompt.push_str("\n[diff truncated]\n");
            break;
        }
        let take = diff.chars().take(remaining).collect::<String>();
        used += take.chars().count();
        prompt.push_str(&format!("--- {}\n{take}\n", change.path));
    }
    prompt.push_str(
        "\nReply with exactly:\nPASS\nor\nCHANGES_REQUIRED\n- reason 1\n\
         Nothing else.",
    );
    prompt
}

/// First non-empty line decides. Anything else is a skip, so a scripted empty turn never blocks.
pub fn parse_review(text: &str) -> Review {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return Review::Skip;
    };
    let head = first
        .trim_matches(|ch: char| ch == '*' || ch == '_' || ch == '`')
        .to_ascii_uppercase();
    if head == "PASS" || head.starts_with("PASS ") || head.starts_with("PASS.") {
        return Review::Pass;
    }
    if head == "CHANGES_REQUIRED" || head.starts_with("CHANGES_REQUIRED") {
        let reasons: Vec<String> = lines
            .map(|line| line.trim_start_matches(['-', '*', ' ']).trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        return Review::ChangesRequired {
            reasons: if reasons.is_empty() {
                vec!["o review pediu mudanças sem listar o motivo".to_string()]
            } else {
                reasons
            },
        };
    }
    Review::Skip
}

/// Formats a failed (or still-pending) validation command for the correction message.
pub fn command_failure_reason(argv: &[String], exit_code: Option<i32>, output: &str) -> String {
    let cmd = argv.join(" ");
    let code = match exit_code {
        Some(code) => format!("exit {code}"),
        None => "sem código de saída".to_string(),
    };
    let trimmed = output.trim();
    if trimmed.is_empty() {
        format!("{cmd}: {code}")
    } else {
        let cut: String = trimmed.chars().take(MAX_FAIL_OUTPUT_CHARS).collect();
        format!("{cmd}: {code}\n{cut}")
    }
}

fn parse_changed_files(workspace: &Workspace, files: &[FileChange]) -> Vec<String> {
    let mut reasons = Vec::new();
    for change in files {
        let Ok(path) = workspace.resolve(&change.path) else {
            reasons.push(format!("{}: caminho fora do workspace", change.path));
            continue;
        };
        let content = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                reasons.push(format!("{}: não deu para ler ({error})", change.path));
                continue;
            }
        };
        if let Err(error) = parse_check(&path, &content) {
            reasons.push(format!("{}: {error}", change.path));
        }
    }
    reasons
}

fn diff_checks(files: &[FileChange], diffs: &HashMap<String, String>) -> Vec<String> {
    let mut reasons = Vec::new();
    let mut diff_lines = 0;
    for change in files {
        let path = Path::new(&change.path);
        if redactor::detect_path_secret(path).is_some() {
            reasons.push(format!("{}: arquivo de secret foi alterado", change.path));
        }
        if is_lockfile(&change.path) {
            reasons.push(format!(
                "{}: lockfile ou arquivo gerado foi alterado",
                change.path
            ));
        }
        if let Some(diff) = diffs.get(&change.path) {
            diff_lines += count_diff_lines(diff);
            if let Some(marker) = introduced_debug_marker(diff) {
                reasons.push(format!("{}: introduziu {marker} no diff", change.path));
            }
        }
    }
    if diff_lines > MAX_DIFF_LINES {
        reasons.push(format!(
            "diff desproporcional: {diff_lines} linhas (limite {MAX_DIFF_LINES})"
        ));
    }
    reasons
}

fn is_lockfile(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    LOCKFILE_NAMES.contains(&name)
}

fn count_diff_lines(diff: &str) -> usize {
    diff.lines()
        .filter(|line| {
            (line.starts_with('+') && !line.starts_with("+++"))
                || (line.starts_with('-') && !line.starts_with("---"))
        })
        .count()
}

fn introduced_debug_marker(diff: &str) -> Option<&'static str> {
    for line in diff.lines() {
        if !line.starts_with('+') || line.starts_with("+++") {
            continue;
        }
        let body = &line[1..];
        if body.contains("TODO") {
            return Some("TODO");
        }
        if body.contains("FIXME") {
            return Some("FIXME");
        }
        if body
            .split_whitespace()
            .any(|token| token == "debugger" || token.starts_with("debugger;"))
        {
            return Some("debugger");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;
    use std::fs;
    use tempfile::tempdir;

    fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace) {
        let dir = tempdir().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
        let workspace = Workspace::open(dir.path()).unwrap();
        (dir, workspace)
    }

    fn change(path: &str) -> FileChange {
        FileChange {
            path: path.to_string(),
            hash_after: "abc".to_string(),
        }
    }

    #[test]
    fn nothing_changed_is_unvalidated() {
        let (_dir, ws) = workspace(&[("src/a.ts", "export const a = 1;\n")]);
        let diffs = HashMap::new();
        let verdict = judge_files(&VerifyInput {
            workspace: &ws,
            request: "olhe",
            files_changed: &[],
            diffs: &diffs,
            passed_validate_since_edit: &[],
        });
        assert!(matches!(verdict, Verdict::Unvalidated { .. }));
    }

    #[test]
    fn changed_files_without_validation_commands_are_unvalidated() {
        let (_dir, ws) = workspace(&[("src/a.ts", "export const a = 1;\n")]);
        let diffs = HashMap::new();
        let files = [change("src/a.ts")];
        let verdict = judge_files(&VerifyInput {
            workspace: &ws,
            request: "crie a",
            files_changed: &files,
            diffs: &diffs,
            passed_validate_since_edit: &[],
        });
        match verdict {
            Verdict::Unvalidated { why } => assert!(why.contains("comandos de validação"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_successful_validate_after_the_edit_is_a_pass() {
        let (_dir, ws) = workspace(&[
            ("package.json", r#"{ "scripts": { "test": "bun test" } }"#),
            ("src/a.ts", "export const a = 1;\n"),
        ]);
        let diffs = HashMap::new();
        let files = [change("src/a.ts")];
        let passed = vec![vec!["bun".to_string(), "test".to_string()]];
        let verdict = judge_files(&VerifyInput {
            workspace: &ws,
            request: "crie a",
            files_changed: &files,
            diffs: &diffs,
            passed_validate_since_edit: &passed,
        });
        assert!(matches!(verdict, Verdict::Pass { .. }), "{verdict:?}");
    }

    #[test]
    fn missing_validate_evidence_asks_the_engine_to_run() {
        let (_dir, ws) = workspace(&[
            ("package.json", r#"{ "scripts": { "test": "bun test" } }"#),
            ("src/a.ts", "export const a = 1;\n"),
        ]);
        let diffs = HashMap::new();
        let files = [change("src/a.ts")];
        let verdict = judge_files(&VerifyInput {
            workspace: &ws,
            request: "crie a",
            files_changed: &files,
            diffs: &diffs,
            passed_validate_since_edit: &[],
        });
        assert_eq!(
            verdict,
            Verdict::Run {
                argv: vec![vec![
                    "bun".to_string(),
                    "run".to_string(),
                    "test".to_string()
                ]]
            }
        );
    }

    #[test]
    fn broken_typescript_fails_parse() {
        let (_dir, ws) = workspace(&[("src/a.ts", "export const a = (\n")]);
        let diffs = HashMap::new();
        let files = [change("src/a.ts")];
        let verdict = judge_files(&VerifyInput {
            workspace: &ws,
            request: "crie a",
            files_changed: &files,
            diffs: &diffs,
            passed_validate_since_edit: &[],
        });
        match verdict {
            Verdict::Fail { reasons } => {
                assert!(
                    reasons.iter().any(|reason| reason.contains("src/a.ts")),
                    "{reasons:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_touched_lockfile_fails() {
        let (_dir, ws) = workspace(&[("package-lock.json", "{}\n")]);
        let diffs = HashMap::new();
        let files = [change("package-lock.json")];
        let verdict = judge_files(&VerifyInput {
            workspace: &ws,
            request: "ignore",
            files_changed: &files,
            diffs: &diffs,
            passed_validate_since_edit: &[],
        });
        match verdict {
            Verdict::Fail { reasons } => {
                assert!(
                    reasons.iter().any(|reason| reason.contains("lockfile")),
                    "{reasons:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn todo_introduced_in_the_diff_fails() {
        let (_dir, ws) = workspace(&[("src/a.ts", "export const a = 1;\n")]);
        let mut diffs = HashMap::new();
        diffs.insert(
            "src/a.ts".to_string(),
            "--- before\n+++ after\n-export const a = 1;\n+export const a = 1; // TODO remove\n"
                .to_string(),
        );
        let files = [change("src/a.ts")];
        let verdict = judge_files(&VerifyInput {
            workspace: &ws,
            request: "crie a",
            files_changed: &files,
            diffs: &diffs,
            passed_validate_since_edit: &[],
        });
        match verdict {
            Verdict::Fail { reasons } => {
                assert!(
                    reasons.iter().any(|reason| reason.contains("TODO")),
                    "{reasons:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_review_reads_pass_and_changes() {
        assert_eq!(parse_review("PASS\n"), Review::Pass);
        assert_eq!(parse_review(""), Review::Skip);
        assert_eq!(parse_review("looks fine to me"), Review::Skip);
        match parse_review("CHANGES_REQUIRED\n- falta o caso zero\n- erros não tratados") {
            Review::ChangesRequired { reasons } => {
                assert_eq!(reasons.len(), 2);
                assert!(reasons[0].contains("zero"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_validate_command_with_exit_zero_counts() {
        assert!(is_successful_validate(
            &["bun".to_string(), "test".to_string()],
            Some(0)
        ));
        assert!(!is_successful_validate(
            &["bun".to_string(), "test".to_string()],
            Some(1)
        ));
        assert!(!is_successful_validate(
            &["echo".to_string(), "oi".to_string()],
            Some(0)
        ));
    }

    #[test]
    fn correction_message_names_the_attempt() {
        let text = correction_message(1, 3, &["bun test: exit 1".to_string()]);
        assert!(text.contains("1 de 3"));
        assert!(text.contains("bun test: exit 1"));
        assert!(text.contains("evidência"));
    }
}
