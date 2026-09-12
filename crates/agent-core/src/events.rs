use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::permissions::{ApprovalAction, CommandClass, PermissionDecision};

/// Structured tool activity routed to the UI and CLI (design §7), in the style of plan 005
/// (`#[serde(tag = "event", content = "data")]`).
///
/// `Deserialize` as well as `Serialize`: a stored event is read back from `events.jsonl` to
/// replay a task, and the replay is typed (plan 015, Part D, step 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "event",
    content = "data"
)]
#[allow(clippy::large_enum_variant)]
pub enum ToolEvent {
    ToolStarted {
        tool: String,
        #[ts(type = "number")]
        request_id: u64,
    },
    ToolCompleted {
        tool: String,
        #[ts(type = "number")]
        request_id: u64,
        #[ts(type = "number")]
        duration_ms: u64,
        truncated: bool,
        decision: PermissionDecision,
    },
    /// Message already redacted.
    ToolFailed {
        tool: String,
        #[ts(type = "number")]
        request_id: u64,
        message: String,
    },
    FileRead {
        path: String,
        #[ts(type = "number")]
        start_line: u64,
        #[ts(type = "number")]
        line_count: u64,
        #[ts(type = "number")]
        total_lines: u64,
        truncated: bool,
        redacted: usize,
    },
    FileChanged {
        path: String,
        diff: String,
        fuzzy: bool,
        hash_before: String,
        hash_after: String,
    },
    CommandStarted {
        #[ts(type = "number")]
        id: u64,
        argv: Vec<String>,
        class: CommandClass,
    },
    CommandCompleted {
        #[ts(type = "number")]
        id: u64,
        exit_code: Option<i32>,
        #[ts(type = "number")]
        duration_ms: u64,
        truncated: bool,
        output_len: usize,
    },
    ApprovalRequired {
        id: String,
        task_id: String,
        action: ApprovalAction,
    },
    ApprovalGranted {
        id: String,
    },
    ApprovalDenied {
        id: String,
        reason: Option<String>,
    },
    CheckpointCreated {
        hash: String,
    },
}

/// What actually travels over the channel: every event carries task identity and an instant
/// (design §7 — "todo evento carrega task_id e timestamp").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ToolEventMessage {
    pub task_id: String,
    #[ts(type = "number")]
    pub sequence: u64,
    pub at: String,
    /// Flattened so every event travels as `{ event, data, taskId, sequence, at }`,
    /// matching the chat channel shape from plan 005.
    #[serde(flatten)]
    #[ts(flatten)]
    pub event: ToolEvent,
}

use std::time::{SystemTime, UNIX_EPOCH};

pub fn timestamp() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let (h, m, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let days = (secs / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    (year + if month <= 2 { 1 } else { 0 }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_serializes_in_event_data_shape() {
        let message = ToolEventMessage {
            task_id: "task_1".to_string(),
            sequence: 0,
            at: "2026-09-11T10:00:00Z".to_string(),
            event: ToolEvent::FileRead {
                path: "src/main.rs".to_string(),
                start_line: 1,
                line_count: 10,
                total_lines: 40,
                truncated: false,
                redacted: 0,
            },
        };
        let value = serde_json::to_value(&message).unwrap();
        // The channel contract used by plan 005: `{ event: "...", data: { ... } }`.
        assert_eq!(value["event"], "fileRead");
        assert_eq!(value["data"]["path"], "src/main.rs");
        assert_eq!(value["data"]["totalLines"], 40);
        assert_eq!(value["taskId"], "task_1");
    }

    #[test]
    fn approval_event_carries_exact_action() {
        let event = ToolEvent::ApprovalRequired {
            id: "aprv_0001".to_string(),
            task_id: "task_1".to_string(),
            action: crate::permissions::ApprovalAction::RunCommand {
                argv: vec!["rm".to_string(), "-rf".to_string(), "dist".to_string()],
                class: crate::permissions::CommandClass::Destructive,
                cwd: "/home/user/projeto".to_string(),
            },
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["event"], "approvalRequired");
        assert_eq!(value["data"]["action"]["type"], "runCommand");
        assert_eq!(value["data"]["action"]["argv"][0], "rm");
    }

    #[test]
    fn timestamp_is_rfc3339_utc() {
        let value = timestamp();
        assert!(value.ends_with('Z'));
        assert_eq!(value.len(), 20); // YYYY-MM-DDTHH:MM:SSZ
    }
}
