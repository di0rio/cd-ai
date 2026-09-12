//! One channel per task (plan 015, D12; SPEC §22). Same `{ event, data }` shape as plan 005,
//! so the UI and the CLI read agent and tool activity from a single stream.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::agent::state::{StopReason, TaskReport, TaskStatus, TaskSummary};
use crate::events::{ToolEvent, timestamp};

/// Everything the loop tells the outside world. Free text here is already redacted.
///
/// Read back as well as written: `TaskStore::load_events` replays a task as
/// `AgentEventMessage`, not as raw JSON (plan 015, Part D, step 0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
#[allow(clippy::large_enum_variant)]
pub enum AgentEvent {
    TaskStarted {
        summary: TaskSummary,
    },
    StatusChanged {
        status: TaskStatus,
        reason: Option<String>,
    },
    /// The request, a course correction, or the resume note (D9).
    UserMessage {
        text: String,
    },
    ModelTurnStarted {
        iteration: u32,
        model: String,
    },
    Token {
        content: String,
    },
    Thinking {
        content: String,
    },
    ModelTurnCompleted {
        #[ts(type = "number")]
        prompt_tokens: u64,
        #[ts(type = "number")]
        gen_tokens: u64,
        #[ts(type = "number")]
        prompt_ms: u64,
        #[ts(type = "number")]
        gen_ms: u64,
    },
    /// Full text of the turn.
    AssistantMessage {
        content: String,
    },
    ToolCallRequested {
        tool: String,
        #[ts(type = "Record<string, unknown>")]
        input: serde_json::Value,
    },
    ToolCallFinished {
        tool: String,
        ok: bool,
        detail: String,
        output: Option<String>,
    },
    /// Everything the `ToolEngine` emits, forwarded as-is.
    Tool(ToolEvent),
    ContextTrimmed {
        removed_messages: u32,
        #[ts(type = "number")]
        estimated_tokens: u64,
    },
    Retrying {
        attempt: u32,
        reason: String,
    },
    TaskFinished {
        status: TaskStatus,
        stop_reason: StopReason,
        report: TaskReport,
    },
}

/// What actually travels over the channel: task identity, order and an instant on every event,
/// flattened so a consumer reads `{ event, data, taskId, sequence, at }` (D12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventMessage {
    pub task_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub at: String,
    #[serde(flatten)]
    #[ts(flatten)]
    pub event: AgentEvent,
}

impl AgentEventMessage {
    pub fn new(task_id: impl Into<String>, sequence: u64, event: AgentEvent) -> Self {
        Self {
            task_id: task_id.into(),
            sequence,
            at: timestamp(),
            event,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::{TaskState, TaskStatus};

    #[test]
    fn message_serializes_in_event_data_shape() {
        let state = TaskState::new("task_1", "C:/projeto", "liste a raiz", "qwen3", 16_384);
        let message = AgentEventMessage::new(
            "task_1",
            0,
            AgentEvent::TaskStarted {
                summary: state.summary(),
            },
        );
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["event"], "taskStarted");
        assert_eq!(value["data"]["summary"]["title"], "liste a raiz");
        assert_eq!(value["taskId"], "task_1");
        assert_eq!(value["sequence"], 0);
        assert!(value["at"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn tool_events_travel_nested_under_tool() {
        let message = AgentEventMessage::new(
            "task_1",
            3,
            AgentEvent::Tool(ToolEvent::FileRead {
                path: "src/main.rs".to_string(),
                start_line: 1,
                line_count: 10,
                total_lines: 40,
                truncated: false,
                redacted: 0,
            }),
        );
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["event"], "tool");
        assert_eq!(value["data"]["event"], "fileRead");
        assert_eq!(value["data"]["data"]["path"], "src/main.rs");
    }

    #[test]
    fn model_turn_counters_serialize_as_numbers() {
        let message = AgentEventMessage::new(
            "task_1",
            1,
            AgentEvent::ModelTurnCompleted {
                prompt_tokens: 1_200,
                gen_tokens: 80,
                prompt_ms: 900,
                gen_ms: 4_000,
            },
        );
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["data"]["promptTokens"], 1_200);
        assert_eq!(value["data"]["genMs"], 4_000);
    }

    #[test]
    fn message_round_trips_through_json() {
        // The replay of a stored task parses these lines back (Part D, step 0), so the flattened
        // envelope has to survive a round trip — including a nested `ToolEvent`.
        let original = AgentEventMessage::new(
            "task_1",
            7,
            AgentEvent::Tool(ToolEvent::CommandCompleted {
                id: 0,
                exit_code: Some(0),
                duration_ms: 12,
                truncated: false,
                output_len: 3,
            }),
        );
        let text = serde_json::to_string(&original).unwrap();
        let back: AgentEventMessage = serde_json::from_str(&text).unwrap();
        assert_eq!(back, original);

        let original = AgentEventMessage::new(
            "task_1",
            8,
            AgentEvent::ToolCallRequested {
                tool: "read_file".to_string(),
                input: serde_json::json!({ "path": "src/a.rs" }),
            },
        );
        let text = serde_json::to_string(&original).unwrap();
        let back: AgentEventMessage = serde_json::from_str(&text).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn finished_event_carries_status_and_reason() {
        let message = AgentEventMessage::new(
            "task_1",
            9,
            AgentEvent::TaskFinished {
                status: TaskStatus::CompletedUnvalidated,
                stop_reason: StopReason::Finished,
                report: TaskReport {
                    summary: "arrumei a soma".to_string(),
                    validated: false,
                    evidence: Vec::new(),
                    files_changed: vec!["src/soma.ts".to_string()],
                },
            },
        );
        let value = serde_json::to_value(&message).unwrap();
        assert_eq!(value["event"], "taskFinished");
        assert_eq!(value["data"]["status"], "completed_unvalidated");
        assert_eq!(value["data"]["stopReason"]["kind"], "finished");
        assert_eq!(value["data"]["report"]["validated"], false);
    }
}
