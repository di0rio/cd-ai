//! Opt-in trajectories (SPEC §25, plan 022): local JSONL, off by default, never uploaded.
//!
//! This is a dataset for a future model, not training. Secrets go through the redactor.
//! Tool output and full file diffs stay out: paths, status and routing are enough to replay
//! the skeleton of a task without copying the workspace into the log.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::agent::events::AgentEvent;
use crate::agent::role::{AgentRole, TaskKind};
use crate::agent::router::ModelCategory;
use crate::agent::state::{StopReason, TaskState, TaskStatus};
use crate::agent::storage::{StorageError, io_error};
use crate::redactor;
use crate::tools::sha256_hex;
use crate::workspace::Workspace;

const TRAJECTORIES_DIR: &str = "trajectories";
const SCHEMA_VERSION: u32 = 1;

/// Append-only JSONL for one task. Created only when the user opted in.
#[derive(Debug, Clone)]
pub struct TrajectoryLog {
    path: PathBuf,
}

impl TrajectoryLog {
    pub fn create(
        data_dir: impl AsRef<Path>,
        workspace: &Workspace,
        task_id: &str,
    ) -> Result<Self, StorageError> {
        let ws = sha256_hex(workspace.root().to_string_lossy().as_bytes());
        let dir = data_dir.as_ref().join(TRAJECTORIES_DIR).join(ws);
        fs::create_dir_all(&dir).map_err(io_error)?;
        let path = dir.join(format!("{task_id}.jsonl"));
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(io_error)?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, record: &TrajectoryRecord) -> Result<(), StorageError> {
        let mut json = serde_json::to_value(record)
            .map_err(|error| StorageError::Format(error.to_string()))?;
        redact_value(&mut json);
        let mut line = serde_json::to_string(&json)
            .map_err(|error| StorageError::Format(error.to_string()))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(io_error)?;
        file.write_all(line.as_bytes()).map_err(io_error)
    }

    pub fn write_header(&self, state: &TaskState) {
        let _ = self.append(&TrajectoryRecord::Header {
            schema_version: SCHEMA_VERSION,
            task_id: state.id.clone(),
            request: state.request.clone(),
            task_kind: state.task_kind,
            model: state.model.clone(),
            category: state.model_category,
            num_ctx: state.num_ctx,
            skills: state
                .selected_skills
                .iter()
                .map(|skill| skill.name.clone())
                .collect(),
        });
    }

    /// Maps a loop event onto a compact record. Tokens and tool output are dropped.
    pub fn observe(&self, event: &AgentEvent) {
        let record = match event {
            AgentEvent::ModelRouted {
                category,
                model,
                num_ctx,
                reason,
            } => Some(TrajectoryRecord::Route {
                category: *category,
                model: model.clone(),
                num_ctx: *num_ctx,
                reason: reason.clone(),
            }),
            AgentEvent::ModelTurnStarted { iteration, model } => Some(TrajectoryRecord::Turn {
                iteration: *iteration,
                model: model.clone(),
                role: AgentRole::Coder,
                prompt_tokens: 0,
                gen_tokens: 0,
            }),
            AgentEvent::ToolCallFinished {
                tool, ok, detail, ..
            } => Some(TrajectoryRecord::Tool {
                tool: tool.clone(),
                ok: *ok,
                detail: detail.clone(),
            }),
            AgentEvent::UserMessage { text } => {
                Some(TrajectoryRecord::Human { text: text.clone() })
            }
            AgentEvent::TaskFinished {
                status,
                stop_reason,
                report,
            } => Some(TrajectoryRecord::Outcome {
                status: *status,
                stop_reason: stop_reason.clone(),
                validated: report.validated,
                files_changed: report.files_changed.clone(),
            }),
            _ => None,
        };
        if let Some(record) = record {
            let _ = self.append(&record);
        }
    }
}

/// One JSONL line. Stable tag names; bump [`SCHEMA_VERSION`] if the shape changes.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TrajectoryRecord {
    Header {
        schema_version: u32,
        task_id: String,
        request: String,
        task_kind: TaskKind,
        model: String,
        category: ModelCategory,
        num_ctx: u32,
        skills: Vec<String>,
    },
    Route {
        category: ModelCategory,
        model: String,
        num_ctx: u32,
        reason: String,
    },
    Turn {
        iteration: u32,
        model: String,
        role: AgentRole,
        prompt_tokens: u64,
        gen_tokens: u64,
    },
    Tool {
        tool: String,
        ok: bool,
        detail: String,
    },
    Human {
        text: String,
    },
    Outcome {
        status: TaskStatus,
        stop_reason: StopReason,
        validated: bool,
        files_changed: Vec<String>,
    },
}

fn redact_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            let redacted = redactor::redact(text);
            if redacted.count() > 0 {
                *text = redacted.text;
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_value),
        serde_json::Value::Object(entries) => entries.values_mut().for_each(redact_value),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::TaskState;
    use crate::workspace::Workspace;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, Workspace) {
        let data = tempdir().unwrap();
        let project = tempdir().unwrap();
        let workspace = Workspace::open(project.path()).unwrap();
        (data, project, workspace)
    }

    #[test]
    fn opt_in_writes_redacted_jsonl() {
        let (data, _project, workspace) = setup();
        let log = TrajectoryLog::create(data.path(), &workspace, "task_1").unwrap();
        let token = "ghp_abcDEF1234567890abcDEF1234567890";
        log.append(&TrajectoryRecord::Human {
            text: format!("use {token}"),
        })
        .unwrap();
        let text = fs::read_to_string(log.path()).unwrap();
        assert!(!text.contains(token), "{text}");
        assert!(text.contains("\"kind\":\"human\""));
        assert!(log.path().starts_with(data.path().join(TRAJECTORIES_DIR)));
    }

    #[test]
    fn observe_ignores_tokens() {
        let (data, _project, workspace) = setup();
        let log = TrajectoryLog::create(data.path(), &workspace, "task_1").unwrap();
        let state = TaskState::new("task_1", workspace.info().root, "pedido", "modelo-x", 8_192);
        log.write_header(&state);
        log.observe(&AgentEvent::Token {
            content: "segredo".into(),
        });
        let text = fs::read_to_string(log.path()).unwrap();
        assert!(!text.contains("segredo"), "{text}");
        assert!(text.contains("\"kind\":\"header\""));
    }
}
