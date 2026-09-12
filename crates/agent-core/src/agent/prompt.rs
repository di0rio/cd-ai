//! The basic context of a task (plan 015, D10): a short system prompt in English, the
//! deterministic workspace profile, and an explicit way to cut the window when it fills up.
//!
//! There is no automatic compaction here. Trimming is visible (the loop emits `ContextTrimmed`)
//! and, past the hard ceiling, the task stops instead of silently losing the beginning of the
//! conversation — summarising is phase 10 (decision 0008).

use crate::ollama::ChatMessage;

/// Above this share of `num_ctx` the oldest tool results are dropped.
const TRIM_THRESHOLD_PERCENT: u64 = 75;
/// Above this share, after trimming, the task cannot continue honestly.
const EXHAUSTED_PERCENT: u64 = 90;
/// Recent tool results the model still needs verbatim.
const KEEP_RECENT_TOOL_RESULTS: usize = 2;
/// What an omitted tool result says. In pt-BR: the model reads it, like every tool result.
pub const OMITTED_RESULT: &str = "[resultado antigo omitido; chame a tool de novo se precisar]";

/// The system prompt, with the workspace profile appended (D10).
pub fn system_prompt(profile: &str) -> String {
    format!(
        "You are cd-ai, a coding agent working inside one local project folder (the workspace).\n\
         Work in small steps and look before you change anything.\n\
         Rules:\n\
         - Paths are relative to the workspace root. Never try to leave it.\n\
         - run_command takes argv as an array of strings and runs without a shell: no pipes, &&, \
         redirects or globs. One command per call.\n\
         - For existing files prefer edit_file (exact old_text -> new_text) over write_file.\n\
         - Never write content you already wrote into a second file to make it \"simpler\". If a \
         file already holds what you meant, that step is done: improve it with edit_file, or \
         finish.\n\
         - File changes and most commands need the user's approval. If something is denied, adapt; \
         do not repeat the same call.\n\
         - When the task is done, or you cannot continue, answer WITHOUT tool calls: a short \
         summary in Brazilian Portuguese of what changed and how it was checked.\n\
         \n\
         Workspace profile:\n\
         {profile}"
    )
}

/// Rough size of the conversation, in tokens: `chars / 4` (D10). A real tokenizer would mean a
/// dependency and a per-model vocabulary; this is an estimate used only to decide when to cut.
pub fn estimate_tokens(messages: &[ChatMessage]) -> u64 {
    let chars: usize = messages.iter().map(message_chars).sum();
    (chars / 4) as u64
}

fn message_chars(message: &ChatMessage) -> usize {
    message.content.chars().count()
        + message
            .tool_name
            .as_ref()
            .map_or(0, |name| name.chars().count())
        + message
            .tool_calls
            .iter()
            .map(|call| {
                call.function.name.chars().count()
                    + call.function.arguments.to_string().chars().count()
            })
            .sum::<usize>()
}

/// Replaces the content of the oldest tool results until the conversation is back under 75% of
/// `num_ctx`, keeping the two most recent ones (D10). The system prompt and the user messages are
/// never candidates, so the task and its rules always survive.
///
/// Returns how many results were omitted and the new estimate, or `None` if nothing was needed.
///
/// Takes a slice, not a `&mut Vec`: no message is ever added or dropped here, only emptied, so the
/// conversation keeps its shape and the model never sees a hole where a result used to be.
pub fn trim_for_budget(messages: &mut [ChatMessage], num_ctx: u32) -> Option<(u32, u64)> {
    let limit = budget(num_ctx, TRIM_THRESHOLD_PERCENT);
    if estimate_tokens(messages) <= limit {
        return None;
    }

    let mut candidates: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == "tool" && message.content != OMITTED_RESULT)
        .map(|(index, _)| index)
        .collect();
    let keep = KEEP_RECENT_TOOL_RESULTS.min(candidates.len());
    candidates.truncate(candidates.len() - keep);

    let mut removed = 0;
    for index in candidates {
        messages[index].content = OMITTED_RESULT.to_string();
        removed += 1;
        if estimate_tokens(messages) <= limit {
            break;
        }
    }
    (removed > 0).then(|| (removed, estimate_tokens(messages)))
}

/// Whether the conversation is past the hard ceiling even after trimming (D10).
pub fn is_exhausted(messages: &[ChatMessage], num_ctx: u32) -> bool {
    estimate_tokens(messages) > budget(num_ctx, EXHAUSTED_PERCENT)
}

fn budget(num_ctx: u32, percent: u64) -> u64 {
    u64::from(num_ctx) * percent / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn system_prompt_is_english_and_carries_the_profile() {
        let prompt = system_prompt("Languages: Rust\n");
        assert!(prompt.starts_with("You are cd-ai"));
        assert!(prompt.contains("argv as an array of strings"));
        assert!(prompt.contains("Brazilian Portuguese"));
        assert!(prompt.ends_with("Workspace profile:\nLanguages: Rust\n"));
    }

    #[test]
    fn estimate_counts_content_and_tool_calls() {
        let messages = vec![message("user", "12345678")];
        assert_eq!(estimate_tokens(&messages), 2);

        let mut with_call = message("assistant", "");
        with_call.tool_calls = vec![crate::ollama::ModelToolCall {
            function: crate::ollama::ModelFunctionCall {
                name: "read_file".to_string(),
                arguments: serde_json::json!({ "path": "a.rs" }),
            },
        }];
        assert!(estimate_tokens(&[with_call]) > 0);
    }

    #[test]
    fn nothing_is_trimmed_below_the_threshold() {
        let mut messages = vec![message("system", "curto"), message("user", "pedido")];
        assert_eq!(trim_for_budget(&mut messages, 16_384), None);
        assert_eq!(messages[0].content, "curto");
    }

    #[test]
    fn oldest_tool_results_are_omitted_and_the_last_two_survive() {
        let big = "x".repeat(400);
        let mut messages = vec![
            message("system", "regras"),
            message("user", "pedido"),
            message("tool", &big),
            message("tool", &big),
            message("tool", &big),
            message("tool", &big),
        ];
        // 1600 chars ≈ 400 tokens, well past 75% of 256.
        let (removed, tokens) = trim_for_budget(&mut messages, 256).expect("precisa cortar");
        assert!(removed >= 1, "cortou {removed}");
        assert_eq!(messages[2].content, OMITTED_RESULT);
        // The two most recent results are never touched.
        assert_eq!(messages[5].content, big);
        assert_eq!(messages[4].content, big);
        // The task and its rules survive.
        assert_eq!(messages[0].content, "regras");
        assert_eq!(messages[1].content, "pedido");
        assert_eq!(tokens, estimate_tokens(&messages));
    }

    #[test]
    fn trimming_stops_as_soon_as_it_fits() {
        let big = "x".repeat(4_000);
        let small = "y".repeat(40);
        let mut messages = vec![
            message("system", "regras"),
            message("user", "pedido"),
            message("tool", &big),
            message("tool", &small),
            message("tool", &small),
            message("tool", &small),
        ];
        let (removed, _) = trim_for_budget(&mut messages, 1_024).expect("precisa cortar");
        assert_eq!(removed, 1, "um resultado grande já resolve");
        assert_eq!(messages[3].content, small);
    }

    #[test]
    fn a_conversation_with_nothing_to_trim_is_exhausted() {
        let mut messages = vec![message("system", &"x".repeat(4_000))];
        assert_eq!(trim_for_budget(&mut messages, 256), None);
        assert!(is_exhausted(&messages, 256));
        assert!(!is_exhausted(&messages, 16_384));
    }

    #[test]
    fn an_already_omitted_result_is_not_counted_again() {
        let big = "x".repeat(4_000);
        let mut messages = vec![
            message("system", "regras"),
            message("tool", OMITTED_RESULT),
            message("tool", &big),
            message("tool", "recente"),
            message("tool", "recente"),
        ];
        let (removed, _) = trim_for_budget(&mut messages, 256).expect("precisa cortar");
        assert_eq!(removed, 1);
        assert_eq!(messages[2].content, OMITTED_RESULT);
    }
}
