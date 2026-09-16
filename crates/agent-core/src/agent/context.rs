//! Context Manager (SPEC §16, plan 020): section budgets, prompt assembly, hygiene and
//! automatic compaction (decision 0008).
//!
//! Token estimates stay `chars / 4` (plan 015 D10). Cuts are explicit: a marker in the text
//! and an event from the loop. The current user request is never trimmed.

use crate::agent::memory::{self, MemoryStore};
use crate::agent::profile::workspace_profile;
use crate::agent::prompt::{
    OMITTED_RESULT, UNTRUSTED_TOOL_BEGIN, UNTRUSTED_TOOL_END, estimate_tokens, system_prompt,
    trim_for_budget, wrap_untrusted_tool_result,
};
use crate::agent::repo_map::{RepoMap, cached_profile_render, load_repo_map, store_profile_render};
use crate::agent::role::{AgentRole, TaskKind, classify_task, role_addendum, starting_role};
use crate::agent::state::{CommandRecord, FileChange};
use crate::ollama::ChatMessage;
use crate::skills::{self, RouteInput, SelectedSkill, SkillSkip};
use crate::workspace::Workspace;

/// Share of `num_ctx` reserved for the model's reply (SPEC §16.2).
pub const RESPONSE_PERCENT: u64 = 20;
const ROLE_PERCENT: u64 = 12;
const RULES_PERCENT: u64 = 8;
const MAP_PERCENT: u64 = 15;
const SKILLS_PERCENT: u64 = 10;
const MEMORY_PERCENT: u64 = 5;
const TRIM_THRESHOLD_PERCENT: u64 = 75;
const EXHAUSTED_PERCENT: u64 = 90;
const KEEP_RECENT_TOOL_RESULTS: usize = 2;
/// Command output larger than this is reduced to the decisive lines.
const LOG_COMPACT_CHARS: usize = 2_000;

/// One section that did not fit and was cut (SPEC §16.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetCut {
    pub section: String,
    pub tokens_before: u64,
    pub tokens_after: u64,
}

/// What [`assemble`] built for this turn.
#[derive(Debug, Clone)]
pub struct AssembledPrompt {
    pub system: String,
    pub role: AgentRole,
    pub kind: TaskKind,
    pub cuts: Vec<BudgetCut>,
    pub map: RepoMap,
    pub skills: Vec<SelectedSkill>,
    pub skipped: Vec<SkillSkip>,
    pub detected: Vec<String>,
    pub memory_ids: Vec<String>,
}

/// Result of preparing the conversation before a model turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrepareResult {
    pub omitted_tools: u32,
    pub compacted: bool,
    pub estimated_tokens: u64,
    pub cuts: Vec<BudgetCut>,
}

/// Builds the system prompt: role + profile + skills + repo map + optional inherited report,
/// each clipped to its section budget.
pub fn assemble(
    workspace: &Workspace,
    data_dir: Option<&std::path::Path>,
    request: &str,
    num_ctx: u32,
    inherited: Option<&str>,
    role: AgentRole,
    selected: Option<&[SelectedSkill]>,
) -> AssembledPrompt {
    let map = load_repo_map(workspace, data_dir, request);
    let kind = classify_task(request, map.indexed);
    let budgets = SectionBudgets::from_num_ctx(num_ctx);

    let profile = workspace_profile(workspace);
    let profile_text = match cached_profile_render(workspace, data_dir) {
        Some(text) => text,
        None => {
            let rendered = profile.render();
            if let Some(dir) = data_dir {
                store_profile_render(workspace, dir, &rendered);
            }
            rendered
        }
    };

    let paths: Vec<String> = map.files.iter().map(|file| file.path.clone()).collect();
    let skill_decision = match selected {
        Some(already) => skills::RouteDecision {
            detected: already.iter().map(|item| item.name.clone()).collect(),
            loaded: already.to_vec(),
            skipped: Vec::new(),
        },
        None => skills::route_builtin(&RouteInput {
            request,
            kind,
            languages: &profile.languages,
            frameworks: &profile.frameworks,
            paths: &paths,
            budget_chars: budgets.skills_chars,
        }),
    };
    let skills_body = skills::render(&skill_decision.loaded, budgets.skills_chars);

    let memory_entries = data_dir
        .and_then(|dir| MemoryStore::open(dir, workspace).ok())
        .map(|store| store.load())
        .unwrap_or_default();
    let relevant = memory::select_relevant(&memory_entries, request, budgets.memory_chars);
    let memory_ids: Vec<String> = relevant.iter().map(|entry| entry.id.clone()).collect();
    let memory_body = memory::render(&relevant);

    let mut cuts = Vec::new();
    let role_text = cut_section(
        "role",
        &format!("{}\n{}", base_role_block(), role_addendum(role)),
        budgets.role_chars,
        &mut cuts,
    );
    let rules = cut_section("rules", &profile_text, budgets.rules_chars, &mut cuts);
    let skills_text = cut_section("skills", &skills_body, budgets.skills_chars, &mut cuts);
    let memory_text = cut_section("memory", &memory_body, budgets.memory_chars, &mut cuts);
    let (map_text, map_cut) = map.render(budgets.map_chars);
    if map_cut {
        cuts.push(BudgetCut {
            section: "repo_map".to_string(),
            tokens_before: tokens_of(&map_text) + 8,
            tokens_after: tokens_of(&map_text),
        });
    }
    let inherited_text = inherited
        .map(|block| cut_section("inherited", block, budgets.inherited_chars, &mut cuts))
        .filter(|text| !text.is_empty());

    let mut system = String::new();
    system.push_str(&role_text);
    if !rules.is_empty() {
        system.push_str("\n\nWorkspace profile:\n");
        system.push_str(&rules);
    }
    if !skills_text.is_empty() {
        system.push_str("\nSkills:\n");
        system.push_str(&skills_text);
        system.push('\n');
    }
    if !memory_text.is_empty() {
        system.push('\n');
        system.push_str(&memory_text);
        system.push('\n');
    }
    if !map_text.is_empty() {
        system.push_str("\nRepo map:\n");
        system.push_str(&map_text);
        system.push('\n');
    }
    if let Some(block) = inherited_text {
        system.push('\n');
        system.push_str(&block);
    }

    AssembledPrompt {
        system,
        role,
        kind,
        cuts,
        map,
        skills: skill_decision.loaded,
        skipped: skill_decision.skipped,
        detected: skill_decision.detected,
        memory_ids,
    }
}

/// First assembly of a new task: classify, pick the starting role, render the prompt.
pub fn assemble_new(
    workspace: &Workspace,
    data_dir: Option<&std::path::Path>,
    request: &str,
    num_ctx: u32,
    inherited: Option<&str>,
) -> AssembledPrompt {
    let probe = load_repo_map(workspace, data_dir, request);
    let kind = classify_task(request, probe.indexed);
    let role = starting_role(kind);
    let mut assembled = assemble(workspace, data_dir, request, num_ctx, inherited, role, None);
    assembled.kind = kind;
    assembled.role = role;
    assembled
}

/// Hygiene + trim + compaction. Mutates `messages` in place. Never drops the system prompt
/// or the original user request.
pub fn prepare_messages(
    messages: &mut Vec<ChatMessage>,
    num_ctx: u32,
    files_changed: &[FileChange],
    commands: &[CommandRecord],
    request: &str,
) -> PrepareResult {
    let mut result = PrepareResult {
        estimated_tokens: estimate_tokens(messages),
        ..PrepareResult::default()
    };

    result.omitted_tools += apply_hygiene(messages, files_changed);

    if let Some((removed, tokens)) = trim_for_budget(messages, num_ctx) {
        result.omitted_tools += removed;
        result.estimated_tokens = tokens;
    }

    let limit = budget(num_ctx, TRIM_THRESHOLD_PERCENT);
    if estimate_tokens(messages) > limit {
        if compact_history(messages, request, files_changed, commands) {
            result.compacted = true;
        }
        result.omitted_tools += shrink_large_tools(messages);
        if let Some((removed, _)) = trim_for_budget(messages, num_ctx) {
            result.omitted_tools += removed;
        }
        result.estimated_tokens = estimate_tokens(messages);
    } else {
        result.estimated_tokens = estimate_tokens(messages);
    }
    result
}

pub fn is_exhausted(messages: &[ChatMessage], num_ctx: u32) -> bool {
    estimate_tokens(messages) > budget(num_ctx, EXHAUSTED_PERCENT)
}

fn base_role_block() -> String {
    // Shared rules, identical to the pre-Fase-10 prompt, minus the trailing "Workspace profile"
    // header — that section is appended separately so it can take its own budget.
    let full = system_prompt("", None);
    full.trim_end_matches("Workspace profile:\n")
        .trim_end()
        .to_string()
}

struct SectionBudgets {
    role_chars: usize,
    rules_chars: usize,
    map_chars: usize,
    skills_chars: usize,
    memory_chars: usize,
    inherited_chars: usize,
}

impl SectionBudgets {
    fn from_num_ctx(num_ctx: u32) -> Self {
        let chars = |percent: u64| (u64::from(num_ctx) * percent / 100 * 4) as usize;
        Self {
            role_chars: chars(ROLE_PERCENT).max(64),
            rules_chars: chars(RULES_PERCENT).max(32),
            map_chars: chars(MAP_PERCENT).max(32),
            skills_chars: chars(SKILLS_PERCENT).max(32),
            memory_chars: chars(MEMORY_PERCENT).max(32),
            inherited_chars: chars(RULES_PERCENT).max(32),
        }
    }
}

fn cut_section(name: &str, text: &str, max_chars: usize, cuts: &mut Vec<BudgetCut>) -> String {
    let before = tokens_of(text);
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let marker = "\n[truncated]";
    let keep = max_chars.saturating_sub(marker.len());
    let clipped: String = text.chars().take(keep).collect();
    let out = format!("{clipped}{marker}");
    cuts.push(BudgetCut {
        section: name.to_string(),
        tokens_before: before,
        tokens_after: tokens_of(&out),
    });
    out
}

fn tokens_of(text: &str) -> u64 {
    (text.chars().count() / 4) as u64
}

fn budget(num_ctx: u32, percent: u64) -> u64 {
    u64::from(num_ctx) * percent / 100
}

fn apply_hygiene(messages: &mut [ChatMessage], files_changed: &[FileChange]) -> u32 {
    let edited: Vec<&str> = files_changed
        .iter()
        .map(|change| change.path.as_str())
        .collect();
    let mut omitted = 0;
    // Paths edited later in the conversation, found by scanning tool names.
    let edited_in_transcript = edited_paths(messages);
    for (index, message) in messages.iter_mut().enumerate() {
        if message.role != "tool" || message.content == OMITTED_RESULT {
            continue;
        }
        let name = message.tool_name.as_deref().unwrap_or("");
        let body = unwrap_tool_body(&message.content);
        if name == "read_file"
            && let Some(path) = read_path(&body)
            && (edited.contains(&path) || later_edit(&edited_in_transcript, index, path))
        {
            message.content = OMITTED_RESULT.to_string();
            omitted += 1;
            continue;
        }
        if name == "run_command" && body.chars().count() > LOG_COMPACT_CHARS {
            let compact = compact_log(&body);
            if compact.chars().count() < body.chars().count() {
                message.content = wrap_untrusted_tool_result(&compact);
                omitted += 1;
            }
        }
    }
    omitted
}

struct EditAt {
    index: usize,
    path: String,
}

fn edited_paths(messages: &[ChatMessage]) -> Vec<EditAt> {
    let mut found = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role != "tool" {
            continue;
        }
        let name = message.tool_name.as_deref().unwrap_or("");
        if name != "edit_file" && name != "write_file" {
            continue;
        }
        let body = unwrap_tool_body(&message.content);
        if let Some(path) = written_path(&body) {
            found.push(EditAt {
                index,
                path: path.to_string(),
            });
        }
    }
    found
}

fn later_edit(edits: &[EditAt], read_index: usize, path: &str) -> bool {
    edits
        .iter()
        .any(|edit| edit.index > read_index && edit.path == path)
}

fn unwrap_tool_body(content: &str) -> String {
    let trimmed = content.trim();
    if let Some(rest) = trimmed.strip_prefix(UNTRUSTED_TOOL_BEGIN) {
        return rest
            .trim_start_matches('\n')
            .strip_suffix(UNTRUSTED_TOOL_END)
            .unwrap_or(rest)
            .trim_end_matches('\n')
            .to_string();
    }
    trimmed.to_string()
}

fn read_path(body: &str) -> Option<&str> {
    let line = body.lines().next()?;
    let path = line
        .split(" (linhas")
        .next()?
        .split(" (arquivo")
        .next()?
        .trim();
    (!path.is_empty()).then_some(path)
}

fn written_path(body: &str) -> Option<&str> {
    let line = body.lines().next()?.trim();
    line.strip_prefix("ok: ").map(|rest| {
        rest.split(" (+")
            .next()
            .unwrap_or(rest)
            .split(" (")
            .next()
            .unwrap_or(rest)
            .trim()
    })
}

fn compact_log(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    if lines.len() <= 16 {
        return body.to_string();
    }
    let mut kept: Vec<&str> = Vec::new();
    let head = lines.first().copied();
    if let Some(first) = head {
        kept.push(first);
    }
    for line in &lines {
        if is_decisive(line) {
            kept.push(line);
        }
    }
    let tail_start = lines.len().saturating_sub(8);
    for line in &lines[tail_start..] {
        if !kept.contains(line) {
            kept.push(line);
        }
    }
    kept.truncate(80);
    if kept.len() + 2 >= lines.len() {
        return body.to_string();
    }
    format!(
        "{}\n[log compactado: {} → {} linhas]",
        kept.join("\n"),
        lines.len(),
        kept.len()
    )
}

fn is_decisive(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("erro")
        || lower.contains("fail")
        || lower.contains("panic")
        || lower.contains("assert")
        || lower.contains(".ts:")
        || lower.contains(".rs:")
        || lower.contains(".js:")
}

fn shrink_large_tools(messages: &mut [ChatMessage]) -> u32 {
    let mut omitted = 0;
    for message in messages.iter_mut() {
        if message.role != "tool" || message.content == OMITTED_RESULT {
            continue;
        }
        let body = unwrap_tool_body(&message.content);
        if body.chars().count() <= LOG_COMPACT_CHARS {
            continue;
        }
        let head = body.lines().next().unwrap_or("").to_string();
        let stub = format!("{head}\n[conteúdo omitido; chame a tool de novo se precisar]");
        message.content = wrap_untrusted_tool_result(&stub);
        omitted += 1;
    }
    omitted
}

fn compact_history(
    messages: &mut Vec<ChatMessage>,
    request: &str,
    files_changed: &[FileChange],
    commands: &[CommandRecord],
) -> bool {
    if messages.len() < 6 {
        return false;
    }
    let system = messages[0].clone();
    let first_user = messages
        .iter()
        .find(|message| message.role == "user")
        .cloned();
    let recent = recent_tail(messages, KEEP_RECENT_TOOL_RESULTS);
    let summary = structured_summary(request, files_changed, commands);
    let mut rebuilt = vec![system];
    if let Some(user) = first_user
        && !recent
            .iter()
            .any(|message| message.role == "user" && message.content == user.content)
    {
        rebuilt.push(user);
    }
    rebuilt.push(ChatMessage {
        role: "user".to_string(),
        content: summary,
        ..Default::default()
    });
    rebuilt.extend(recent);
    if rebuilt.len() >= messages.len() {
        return false;
    }
    *messages = rebuilt;
    true
}

fn recent_tail(messages: &[ChatMessage], keep_tools: usize) -> Vec<ChatMessage> {
    let tool_indexes: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == "tool" && message.content != OMITTED_RESULT)
        .map(|(index, _)| index)
        .collect();
    let start = if tool_indexes.len() <= keep_tools {
        messages.len().saturating_sub(4)
    } else {
        let from = tool_indexes[tool_indexes.len() - keep_tools];
        // Include the assistant turn that produced those tools.
        (0..from)
            .rev()
            .find(|&index| messages[index].role == "assistant")
            .unwrap_or(from)
    };
    messages[start.min(messages.len())..].to_vec()
}

fn structured_summary(
    request: &str,
    files_changed: &[FileChange],
    commands: &[CommandRecord],
) -> String {
    let mut block = String::from(
        "[context compacted] Earlier tool results were dropped. Re-read a file instead of \
         assuming its old contents. Facts from the engine:\n",
    );
    block.push_str(&format!("Request: {}\n", one_line(request, 400)));
    if !files_changed.is_empty() {
        let files: Vec<&str> = files_changed
            .iter()
            .map(|change| change.path.as_str())
            .collect();
        block.push_str(&format!("Files already changed: {}\n", files.join(", ")));
    }
    if !commands.is_empty() {
        block.push_str("Commands already run:\n");
        for command in commands.iter().take(20) {
            let result = match command.exit_code {
                Some(code) => format!("exit {code}"),
                None => "no exit code".to_string(),
            };
            block.push_str(&format!("- {} -> {result}\n", command.argv.join(" ")));
        }
    }
    block
}

fn one_line(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    line.chars().take(max).collect()
}

/// Re-export so the loop can keep calling one function for inherited reports.
pub use crate::agent::prompt::inherited_context as inherited_report;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::prompt::{UNTRUSTED_TOOL_BEGIN, wrap_untrusted_tool_result};
    use crate::agent::state::FileChange;
    use crate::workspace::Workspace;
    use std::fs;
    use tempfile::tempdir;

    fn message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn tool(name: &str, body: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_name: Some(name.to_string()),
            content: wrap_untrusted_tool_result(body),
            ..Default::default()
        }
    }

    fn project(files: &[(&str, &str)]) -> (tempfile::TempDir, Workspace) {
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

    #[test]
    fn assemble_puts_the_map_and_the_role_in_the_system_prompt() {
        let (_dir, workspace) = project(&[(
            "src/soma.ts",
            "export function soma(a: number, b: number): number { return a - b; }\n",
        )]);
        let assembled = assemble_new(&workspace, None, "corrija a soma", 16_384, None);
        assert_eq!(assembled.kind, TaskKind::Trivial);
        assert_eq!(assembled.role, AgentRole::Coder);
        assert!(assembled.system.contains("You are cd-ai"));
        assert!(assembled.system.contains("Role: Coder"));
        assert!(assembled.system.contains("Repo map:"));
        assert!(assembled.system.contains("src/soma.ts"));
        assert!(assembled.system.contains("soma"));
        assert!(assembled.system.contains("Workspace profile:"));
        assert!(assembled.system.contains("Skills:"));
        assert!(assembled.system.contains("### typescript"));
        assert!(assembled.skills.iter().any(|s| s.name == "typescript"));
    }

    #[test]
    fn eval_style_request_selects_testing_typescript_debugging() {
        let (_dir, workspace) = project(&[
            ("package.json", r#"{ "scripts": { "test": "bun test" } }"#),
            (
                "src/soma.ts",
                "export function soma(a: number, b: number): number { return a - b; }\n",
            ),
        ]);
        let assembled = assemble_new(
            &workspace,
            None,
            "O teste de soma falha. Corrija e rode os testes.",
            16_384,
            None,
        );
        let names: Vec<&str> = assembled.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"typescript"), "{names:?}");
        assert!(names.contains(&"testing"), "{names:?}");
        assert!(names.contains(&"debugging"), "{names:?}");
        assert!(!names.contains(&"security"));
        assert!(assembled.system.contains("### testing"));
        assert!(assembled.system.contains("reproduce the failing test"));
    }

    #[test]
    fn a_question_starts_as_explorer() {
        let (_dir, workspace) = project(&[("src/a.ts", "export function a() {}\n")]);
        let assembled = assemble_new(&workspace, None, "o que a função a faz?", 16_384, None);
        assert_eq!(assembled.kind, TaskKind::Question);
        assert_eq!(assembled.role, AgentRole::Explorer);
        assert!(assembled.system.contains("Role: Explorer"));
        assert!(assembled.system.contains("Never edit"));
    }

    #[test]
    fn section_budget_cuts_an_oversized_map() {
        let files: Vec<(String, String)> = (0..30)
            .map(|i| {
                (
                    format!("src/f{i}.ts"),
                    format!("export function f{i}() {{ return {i}; }}\n"),
                )
            })
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, content)| (path.as_str(), content.as_str()))
            .collect();
        let (_dir, workspace) = project(&refs);
        let assembled = assemble_new(&workspace, None, "liste tudo", 256, None);
        assert!(
            assembled.cuts.iter().any(|cut| cut.section == "repo_map"
                || cut.section == "role"
                || cut.section == "rules"),
            "{:?}",
            assembled.cuts
        );
        // Role + map + profile must leave room for the reserved reply (20% of num_ctx).
        let max_system_chars = (256 * (100 - RESPONSE_PERCENT) / 100 * 4) as usize;
        assert!(
            assembled.system.chars().count() <= max_system_chars + 64,
            "system {} > {max_system_chars}",
            assembled.system.chars().count()
        );
    }

    #[test]
    fn hygiene_drops_a_stale_read_after_an_edit() {
        let mut messages = vec![
            message("system", "regras"),
            message("user", "corrija"),
            tool(
                "read_file",
                "src/a.ts (linhas 1-2 de 2)\nexport const x = 1;\n",
            ),
            tool("edit_file", "ok: src/a.ts (+1 −1)"),
        ];
        let changed = vec![FileChange {
            path: "src/a.ts".to_string(),
            hash_after: "x".to_string(),
        }];
        let omitted = apply_hygiene(&mut messages, &changed);
        assert!(omitted >= 1);
        assert_eq!(messages[2].content, OMITTED_RESULT);
        assert!(messages[3].content.contains("ok: src/a.ts"));
        assert!(messages[3].content.contains(UNTRUSTED_TOOL_BEGIN));
    }

    #[test]
    fn hygiene_compacts_a_huge_command_log() {
        let mut log = String::from("exit 1\n");
        for i in 0..200 {
            log.push_str(&format!("info line {i}\n"));
        }
        log.push_str("error: boom at src/a.ts:3\n");
        let mut messages = vec![message("system", "regras"), tool("run_command", &log)];
        let omitted = apply_hygiene(&mut messages, &[]);
        assert!(omitted >= 1);
        assert!(messages[1].content.contains("error: boom"));
        assert!(messages[1].content.contains("[log compactado"));
        assert!(messages[1].content.len() < log.len());
    }

    #[test]
    fn compaction_replaces_the_middle_and_keeps_the_request() {
        let big = "x".repeat(800);
        let mut messages = vec![message("system", "regras")];
        messages.push(message("user", "pedido original"));
        for i in 0..6 {
            messages.push(message("assistant", "ok"));
            messages.push(tool(
                "read_file",
                &format!("f{i}.ts (linhas 1-1 de 1)\n{big}"),
            ));
        }
        let before = estimate_tokens(&messages);
        let changed = vec![FileChange {
            path: "f0.ts".to_string(),
            hash_after: "h".to_string(),
        }];
        let ok = compact_history(&mut messages, "pedido original", &changed, &[]);
        assert!(ok);
        assert_eq!(messages[0].content, "regras");
        assert!(messages.iter().any(|m| m.content == "pedido original"));
        assert!(
            messages
                .iter()
                .any(|m| m.content.contains("[context compacted]"))
        );
        assert!(estimate_tokens(&messages) < before);
    }

    #[test]
    fn assemble_injects_relevant_memory_and_skips_stale() {
        use crate::agent::memory::MemoryKind;
        let dir = tempdir().unwrap();
        let project = dir.path().join("proj");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(
            project.join("src/soma.ts"),
            "export function soma(a: number, b: number): number { return a - b; }\n",
        )
        .unwrap();
        let workspace = Workspace::open(&project).unwrap();
        let data = dir.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let store = MemoryStore::open(&data, &workspace).unwrap();
        store
            .add(MemoryKind::Rule, "testes deste projeto usam bun")
            .unwrap();
        let stale = store
            .add(MemoryKind::Learned, "o staging cai sexta")
            .unwrap();
        store.set_stale(&stale.id, true).unwrap();

        let assembled = assemble_new(
            &workspace,
            Some(&data),
            "corrija a soma e rode os testes",
            16_384,
            None,
        );
        assert!(assembled.system.contains("Memory:"));
        assert!(
            assembled
                .system
                .contains("[rule] testes deste projeto usam bun")
        );
        assert!(!assembled.system.contains("staging"));
        assert_eq!(assembled.memory_ids, vec!["mem_1".to_string()]);
    }
}
