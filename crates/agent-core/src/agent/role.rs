//! Roles of the v1 agent (SPEC §12, plan 020): Explorer (read-only) and Coder.
//!
//! The Verifier is not a model role — it is deterministic code (plan 018). Skills are knowledge
//! selected by `crate::skills` (plan 021), not extra roles.
//! Classification is a pure function over the request and the size of the repo map.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::tool_calls::{TOOL_NAMES, tool_specs};
use crate::ollama::ToolSpec;
use crate::tools::ToolRequest;

/// How the orchestrator labelled this task (SPEC §11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Question,
    Trivial,
    #[default]
    Normal,
    Complex,
}

/// Which prompt and tool set the model is running under. Default `Coder` so states written
/// before Fase 10 resume with write tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Explorer,
    #[default]
    Coder,
}

/// Tools the Explorer is allowed to call. Write/command tools are refused even if emitted.
pub const EXPLORER_TOOLS: [&str; 7] = [
    "read_file",
    "list_directory",
    "search",
    "git_status",
    "git_diff",
    "git_log",
    "git_branch",
];

/// After this many Explorer turns without a handoff, the loop promotes to Coder (D7).
pub const MAX_EXPLORER_ITERATIONS: u32 = 8;

/// Deterministic label for a request (plan 020, D8). `source_files` is the repo-map size.
pub fn classify_task(request: &str, source_files: usize) -> TaskKind {
    let lower = request.to_ascii_lowercase();
    if is_question(&lower) && !has_edit_verb(&lower) {
        return TaskKind::Question;
    }
    if is_complex(&lower, request.chars().count()) {
        return TaskKind::Complex;
    }
    if source_files <= 12 || names_existing_path(&lower) {
        return TaskKind::Trivial;
    }
    TaskKind::Normal
}

/// Starting role for a kind (D7): only questions and complex tasks spend a model turn exploring.
pub fn starting_role(kind: TaskKind) -> AgentRole {
    match kind {
        TaskKind::Question | TaskKind::Complex => AgentRole::Explorer,
        TaskKind::Trivial | TaskKind::Normal => AgentRole::Coder,
    }
}

pub fn tool_specs_for(role: AgentRole) -> Vec<ToolSpec> {
    match role {
        AgentRole::Coder => tool_specs(),
        AgentRole::Explorer => tool_specs()
            .into_iter()
            .filter(|spec| EXPLORER_TOOLS.contains(&spec.function.name.as_str()))
            .collect(),
    }
}

pub fn offered_names(role: AgentRole) -> Vec<&'static str> {
    match role {
        AgentRole::Coder => TOOL_NAMES.to_vec(),
        AgentRole::Explorer => EXPLORER_TOOLS.to_vec(),
    }
}

/// Whether the engine may run this request for the current role. The trust boundary is here,
/// not in the list of specs the model was shown.
pub fn permits(role: AgentRole, request: &ToolRequest) -> bool {
    match role {
        AgentRole::Coder => true,
        AgentRole::Explorer => matches!(
            request,
            ToolRequest::ReadFile(_)
                | ToolRequest::ListDirectory(_)
                | ToolRequest::Search(_)
                | ToolRequest::GitStatus(_)
                | ToolRequest::GitDiff(_)
                | ToolRequest::GitLog(_)
                | ToolRequest::GitBranch(_)
        ),
    }
}

pub fn explorer_refusal(tool: &str) -> String {
    format!(
        "erro: o role Explorer é somente leitura; {tool} não está disponível. \
         Use read_file, list_directory, search ou as tools git. Quando souber o bastante, \
         responda sem tools."
    )
}

/// Extra rules appended to the shared system prompt for this role.
pub fn role_addendum(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Coder => {
            "Role: Coder. Read before you change anything. Prefer edit_file on existing files. \
             When you are done, answer without tool calls."
        }
        AgentRole::Explorer => {
            "Role: Explorer. You may only read, list, search and inspect git. Never edit, write \
             or run commands. Start from the repo map. Never read the whole project. When you \
             know enough, answer WITHOUT tool calls: the relevant files, short excerpts, related \
             tests, validation commands you noticed, and risks. Do not claim you changed anything."
        }
    }
}

pub fn coder_handoff_message() -> &'static str {
    // Short on purpose: internal role handoff, not user-facing (plan 021 D8).
    "Explorer done. You are Coder: edit/write/run allowed. Use the repo map and files already \
     read. Re-read before edit. Do not re-explore unless a path is missing."
}

fn is_question(lower: &str) -> bool {
    if lower.contains('?') {
        return true;
    }
    const HEADS: &[&str] = &[
        "o que ",
        "onde ",
        "como ",
        "qual ",
        "quais ",
        "por que ",
        "porque ",
        "why ",
        "what ",
        "where ",
        "how ",
        "explique ",
        "explain ",
    ];
    let trimmed = lower.trim_start();
    HEADS.iter().any(|head| trimmed.starts_with(head))
}

fn has_edit_verb(lower: &str) -> bool {
    const VERBS: &[&str] = &[
        "corrija",
        "corrigir",
        "conserte",
        "consertar",
        "crie",
        "criar",
        "implemente",
        "implementar",
        "adicion",
        "remova",
        "remover",
        "altere",
        "alterar",
        "edite",
        "editar",
        "escreva",
        "escrever",
        "ajuste",
        "fix ",
        "fix.",
        "create ",
        "implement ",
        "add ",
        "remove ",
        "write ",
        "update ",
        "patch ",
    ];
    VERBS.iter().any(|verb| lower.contains(verb))
}

fn is_complex(lower: &str, chars: usize) -> bool {
    chars > 800
        || lower.contains("arquitetura")
        || lower.contains("architecture")
        || lower.contains("refator")
        || lower.contains("refactor")
        || lower.contains("migrar")
        || lower.contains("migrate")
}

fn names_existing_path(lower: &str) -> bool {
    lower.contains('/')
        || lower.contains(".ts")
        || lower.contains(".rs")
        || lower.contains(".js")
        || lower.contains(".tsx")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_style_requests_are_trivial_on_a_small_repo() {
        let kind = classify_task("O teste de soma falha. Corrija e rode os testes.", 3);
        assert_eq!(kind, TaskKind::Trivial);
        assert_eq!(starting_role(kind), AgentRole::Coder);

        let kind = classify_task(
            "O teste de dobro falha porque o módulo não existe. Crie src/dobro.ts e rode os testes.",
            2,
        );
        assert_eq!(kind, TaskKind::Trivial);
    }

    #[test]
    fn a_question_without_edit_verbs_is_explorer() {
        let kind = classify_task("O que a função soma faz?", 3);
        assert_eq!(kind, TaskKind::Question);
        assert_eq!(starting_role(kind), AgentRole::Explorer);
        assert_eq!(offered_names(AgentRole::Explorer).len(), 7);
        assert!(!offered_names(AgentRole::Explorer).contains(&"edit_file"));
        assert!(!offered_names(AgentRole::Explorer).contains(&"run_command"));
    }

    #[test]
    fn architecture_requests_are_complex() {
        let kind = classify_task("refatore a arquitetura do módulo de pagamentos", 40);
        assert_eq!(kind, TaskKind::Complex);
        assert_eq!(starting_role(kind), AgentRole::Explorer);
    }

    #[test]
    fn explorer_permits_only_reads() {
        let read = ToolRequest::ReadFile(crate::tools::ReadFileArgs {
            path: "a.ts".to_string(),
            start_line: None,
            end_line: None,
        });
        let write = ToolRequest::WriteFile(crate::tools::WriteFileArgs {
            path: "a.ts".to_string(),
            content: "x".to_string(),
            if_exists: crate::tools::IfExists::Error,
        });
        assert!(permits(AgentRole::Explorer, &read));
        assert!(!permits(AgentRole::Explorer, &write));
        assert!(permits(AgentRole::Coder, &write));
    }

    #[test]
    fn look_at_the_project_is_not_a_question() {
        assert_eq!(classify_task("olhe o projeto", 1), TaskKind::Trivial);
    }
}
