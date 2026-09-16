//! The agent: the loop that turns a request into changes on disk, plus the task state, its
//! persistence, the deterministic workspace profile and the prompt that go into every task
//! (plan 015, Parts C and D).

pub mod context;
pub mod events;
pub mod model;
pub mod profile;
pub mod prompt;
pub mod repo_map;
pub mod role;
pub mod runner;
pub mod settings;
pub mod state;
pub mod storage;
pub mod tool_calls;
pub mod verify;

pub use events::{AgentEvent, AgentEventMessage};
pub use model::{ChatModel, ModelError, ModelReply, OllamaModel, ScriptedModel};
pub use profile::{WorkspaceProfile, workspace_profile};
pub use prompt::{estimate_tokens, system_prompt, trim_for_budget};
pub use role::{AgentRole, TaskKind};
pub use runner::{RESUME_NOTE, TaskContext, TaskStart, run_task, workspace_key};
pub use settings::{Settings, SettingsStore};
pub use state::{
    AgentLimits, Checkpoint, CheckpointKind, CommandRecord, FileChange, RollbackResult,
    RollbackSkip, StopReason, TaskHistoryEntry, TaskMetrics, TaskReport, TaskState, TaskStatus,
    TaskSummary,
};
pub use storage::{DATA_DIR_ENV, StorageError, TaskStore, data_dir};
pub use tool_calls::{MAX_TOOL_RESULT_CHARS, TOOL_NAMES, render_outcome, tool_specs};
pub use verify::{Review, Verdict};
