//! What a task is, as persisted state (plan 015, Part C contracts; SPEC §23).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::timestamp;

/// First characters of the request used as a task title.
const TITLE_CHARS: usize = 60;

/// Status of a task (SPEC §23). `completed` is only emitted when the Verifier has evidence
/// (plan 018); a finish without that evidence is `completed_unvalidated`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    WaitingApproval,
    Completed,
    CompletedUnvalidated,
    Failed,
    Cancelled,
}

/// Why the loop stopped. Every terminal status carries one (SPEC §11.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StopReason {
    /// The model answered without tool calls, and nothing was validated.
    Finished,
    /// Deterministic checks passed after the last edit (plan 018).
    Verified,
    MaxIterations,
    TaskTimeout,
    LoopDetected {
        detail: String,
    },
    InvalidToolCalls,
    ModelError {
        message: String,
    },
    ContextExhausted,
    Cancelled,
    /// The app died while the task was running; found again on the next open (D9).
    Interrupted,
}

/// Operational limits of the loop (plan 015, D7; SPEC §11.1). Not a token budget:
/// inference is local and has no cost (decision 0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AgentLimits {
    pub max_iterations: u32,
    pub max_model_retries: u32,
    /// Consecutive invalid tool calls tolerated before the task fails.
    pub max_invalid_tool_calls: u32,
    /// How many times a failed verification may send the model back to fix (plan 018, D7).
    pub max_correction_retries: u32,
    /// Isolated LLM review after a deterministic pass (SPEC §13.2). Empty/unparseable = skip.
    pub llm_review: bool,
    #[ts(type = "number")]
    pub model_turn_timeout_ms: u64,
    #[ts(type = "number")]
    pub task_timeout_ms: u64,
}

impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            max_iterations: 30,
            max_model_retries: 2,
            max_invalid_tool_calls: 3,
            max_correction_retries: 3,
            llm_review: true,
            // 300 s covers a 16k prefill at ~100 tok/s plus generation at ~20 tok/s (bench 2026-09-11).
            model_turn_timeout_ms: 300_000,
            task_timeout_ms: 45 * 60 * 1_000,
        }
    }
}

/// One file the task changed, with the hash that proves the content afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub hash_after: String,
}

/// One file the safe rollback refused to touch, with the diff the user needs to decide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RollbackSkip {
    pub path: String,
    pub reason: String,
    pub diff: String,
}

/// What a rollback did. Paths not listed here were never candidates (the agent did not write them).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RollbackResult {
    pub restored: Vec<String>,
    pub skipped: Vec<RollbackSkip>,
    pub already_clean: Vec<String>,
}

/// Why a shadow-repo commit was taken (plan 019, SPEC §21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum CheckpointKind {
    Baseline,
    AfterWrite,
    BeforeDestructive,
}

/// One commit in the shadow repo, hanging off the task that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub commit: String,
    pub kind: CheckpointKind,
    pub created_at: String,
}

/// One command the task ran. Evidence for the report (SPEC §2).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CommandRecord {
    pub argv: Vec<String>,
    /// `None` when the command timed out or was cancelled (plan 015, maintenance notes).
    pub exit_code: Option<i32>,
    #[ts(type = "number")]
    pub duration_ms: u64,
}

/// Counters worth showing in the UI; never a billing figure (decision 0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskMetrics {
    #[ts(type = "number")]
    pub prompt_tokens: u64,
    #[ts(type = "number")]
    pub gen_tokens: u64,
    #[ts(type = "number")]
    pub model_ms: u64,
    /// Time the tools actually ran. Waiting for a human to approve is not tool work, so it is
    /// counted apart in `approval_wait_ms`.
    #[ts(type = "number")]
    pub tool_ms: u64,
    /// Time the task sat blocked on an approval prompt. Absent from states written before this
    /// field existed, hence the default.
    #[serde(default)]
    #[ts(type = "number")]
    pub approval_wait_ms: u64,
}

/// Everything about a task that survives a restart. Written to `tasks/<id>/state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskState {
    pub id: String,
    /// Workspace root, for display and for filtering the task list.
    pub workspace: String,
    pub request: String,
    /// Task this one continues, when the request came right after another one in the same
    /// workspace. Only the previous report is inherited, never its transcript: a task still has
    /// one request, and continuity is a chain of tasks, not a task with many turns.
    /// Absent from states written before chaining existed, hence the default.
    #[serde(default)]
    pub continues: Option<String>,
    pub model: String,
    pub num_ctx: u32,
    pub status: TaskStatus,
    pub stop_reason: Option<StopReason>,
    pub created_at: String,
    pub updated_at: String,
    pub iterations: u32,
    pub files_read: Vec<String>,
    pub files_changed: Vec<FileChange>,
    pub commands: Vec<CommandRecord>,
    /// Shadow-repo commits of this task (plan 019). Absent from states written before Fase 9.
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    /// Set when the user rolled this task back (plan 019). Absent from older states.
    #[serde(default)]
    pub rolled_back: bool,
    /// Redacted before reaching disk (D8).
    pub errors: Vec<String>,
    pub retries: u32,
    pub metrics: TaskMetrics,
}

impl TaskState {
    pub fn new(
        id: impl Into<String>,
        workspace: impl Into<String>,
        request: impl Into<String>,
        model: impl Into<String>,
        num_ctx: u32,
    ) -> Self {
        let at = timestamp();
        Self {
            id: id.into(),
            workspace: workspace.into(),
            request: request.into(),
            continues: None,
            model: model.into(),
            num_ctx,
            status: TaskStatus::Running,
            stop_reason: None,
            created_at: at.clone(),
            updated_at: at,
            iterations: 0,
            files_read: Vec::new(),
            files_changed: Vec::new(),
            commands: Vec::new(),
            checkpoints: Vec::new(),
            rolled_back: false,
            errors: Vec::new(),
            retries: 0,
            metrics: TaskMetrics::default(),
        }
    }

    /// Marks the state as changed now. Every write to disk should follow one.
    pub fn touch(&mut self) {
        self.updated_at = timestamp();
    }

    pub fn finish(&mut self, status: TaskStatus, reason: StopReason) {
        self.status = status;
        self.stop_reason = Some(reason);
        self.touch();
    }

    pub fn summary(&self) -> TaskSummary {
        TaskSummary {
            id: self.id.clone(),
            title: title_from_request(&self.request),
            status: self.status,
            updated_at: self.updated_at.clone(),
            model: self.model.clone(),
            continues: self.continues.clone(),
        }
    }

    pub fn history_entry(&self) -> TaskHistoryEntry {
        TaskHistoryEntry {
            summary: self.summary(),
            files_changed: self
                .files_changed
                .iter()
                .map(|change| change.path.clone())
                .collect(),
            command_count: self.commands.len() as u32,
            checkpoint: self
                .checkpoints
                .iter()
                .find(|checkpoint| checkpoint.kind == CheckpointKind::Baseline)
                .map(|checkpoint| checkpoint.commit.clone()),
            rolled_back: self.rolled_back,
            created_at: self.created_at.clone(),
            metrics: self.metrics,
        }
    }
}

/// A task as listed in the sidebar and by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub updated_at: String,
    pub model: String,
    /// Id of the task this one continues, so a list can show a chain instead of loose fragments.
    pub continues: Option<String>,
}

/// A task as listed in the workspace history (plan 019, SPEC §24.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskHistoryEntry {
    pub summary: TaskSummary,
    pub files_changed: Vec<String>,
    #[ts(type = "number")]
    pub command_count: u32,
    pub checkpoint: Option<String>,
    pub rolled_back: bool,
    pub created_at: String,
    pub metrics: TaskMetrics,
}

/// What the task delivered. `validated` is true only when the Verifier had evidence (plan 018).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskReport {
    /// Final model text, redacted.
    pub summary: String,
    pub validated: bool,
    pub evidence: Vec<CommandRecord>,
    pub files_changed: Vec<String>,
}

/// Title shown in lists: the first characters of the request, on a char boundary.
pub fn title_from_request(request: &str) -> String {
    let trimmed = request.trim();
    let title: String = trimmed.chars().take(TITLE_CHARS).collect();
    if title.chars().count() < trimmed.chars().count() {
        format!("{title}…")
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_default_to_the_plan_values() {
        let limits = AgentLimits::default();
        assert_eq!(limits.max_iterations, 30);
        assert_eq!(limits.max_model_retries, 2);
        assert_eq!(limits.max_invalid_tool_calls, 3);
        assert_eq!(limits.max_correction_retries, 3);
        assert!(limits.llm_review);
        assert_eq!(limits.model_turn_timeout_ms, 300_000);
        assert_eq!(limits.task_timeout_ms, 2_700_000);
    }

    #[test]
    fn status_serializes_in_snake_case() {
        let value = serde_json::to_value(TaskStatus::CompletedUnvalidated).unwrap();
        assert_eq!(value, "completed_unvalidated");
        let value = serde_json::to_value(TaskStatus::Completed).unwrap();
        assert_eq!(value, "completed");
        let value = serde_json::to_value(TaskStatus::WaitingApproval).unwrap();
        assert_eq!(value, "waiting_approval");
    }

    #[test]
    fn stop_reason_is_tagged_by_kind() {
        let value = serde_json::to_value(StopReason::LoopDetected {
            detail: "read_file src/a.rs".to_string(),
        })
        .unwrap();
        assert_eq!(value["kind"], "loopDetected");
        assert_eq!(value["detail"], "read_file src/a.rs");
        let value = serde_json::to_value(StopReason::Finished).unwrap();
        assert_eq!(value["kind"], "finished");
        let value = serde_json::to_value(StopReason::Verified).unwrap();
        assert_eq!(value["kind"], "verified");
    }

    #[test]
    fn new_state_starts_running_without_reason() {
        let state = TaskState::new("task_1", "C:/projeto", "arrume o teste", "qwen3", 16_384);
        assert_eq!(state.status, TaskStatus::Running);
        assert!(state.stop_reason.is_none());
        assert_eq!(state.created_at, state.updated_at);
        assert_eq!(state.iterations, 0);
    }

    #[test]
    fn summary_title_is_cut_at_sixty_chars() {
        let request = "á".repeat(80);
        let state = TaskState::new("task_1", "C:/projeto", &request, "qwen3", 16_384);
        let summary = state.summary();
        // 60 chars plus the ellipsis; counted in chars so multibyte text never splits.
        assert_eq!(summary.title.chars().count(), 61);
        assert!(summary.title.ends_with('…'));
    }

    #[test]
    fn short_title_has_no_ellipsis() {
        assert_eq!(title_from_request("  liste a raiz  "), "liste a raiz");
    }

    #[test]
    fn metrics_written_before_approval_wait_existed_still_load() {
        let metrics: TaskMetrics = serde_json::from_str(
            r#"{"promptTokens":7543,"genTokens":289,"modelMs":97952,"toolMs":3532469}"#,
        )
        .unwrap();
        assert_eq!(metrics.tool_ms, 3_532_469);
        assert_eq!(metrics.approval_wait_ms, 0);
    }

    #[test]
    fn a_state_written_before_chaining_still_loads() {
        // Shape of a real `state.json` from before `continues` existed: it must keep loading, or
        // the app breaks on every task already on disk.
        let state: TaskState = serde_json::from_str(
            r#"{"id":"task_1","workspace":"C:/projeto","request":"pedido","model":"qwen3",
                "numCtx":16384,"status":"completed_unvalidated","stopReason":{"kind":"finished"},
                "createdAt":"2026-09-11T10:00:00Z","updatedAt":"2026-09-11T10:05:00Z",
                "iterations":3,"filesRead":[],"filesChanged":[],"commands":[],"errors":[],
                "retries":0,"metrics":{"promptTokens":0,"genTokens":0,"modelMs":0,"toolMs":0}}"#,
        )
        .unwrap();
        assert_eq!(state.continues, None);
        assert_eq!(state.summary().continues, None);
        assert!(state.checkpoints.is_empty());
        assert!(!state.rolled_back);
    }

    #[test]
    fn state_round_trips_through_json() {
        let mut state = TaskState::new("task_1", "C:/projeto", "pedido", "qwen3", 16_384);
        state.files_changed.push(FileChange {
            path: "src/a.rs".to_string(),
            hash_after: "abc".to_string(),
        });
        state.commands.push(CommandRecord {
            argv: vec!["cargo".to_string(), "test".to_string()],
            exit_code: Some(0),
            duration_ms: 1_200,
        });
        state.finish(TaskStatus::CompletedUnvalidated, StopReason::Finished);
        let text = serde_json::to_string(&state).unwrap();
        let back: TaskState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, state);
    }
}
