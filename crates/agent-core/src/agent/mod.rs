//! The agent: the loop that turns a request into changes on disk, plus the task state, its
//! persistence, the deterministic workspace profile and the prompt that go into every task
//! (plan 015, Parts C and D; plan 020 Context Manager).

pub mod context;
pub mod events;
pub mod model;
pub mod profile;
pub mod prompt;
pub mod repo_map;
pub mod runner;
pub mod settings;
pub mod state;
pub mod storage;
pub mod tool_calls;
pub mod verify;

pub use context::{AgentRole, AssembledContext, TaskKind, assemble};
pub use events::{AgentEvent, AgentEventMessage};
pub use model::{ChatModel, ModelError, ModelReply, OllamaModel, ScriptedModel};
pub use profile::{WorkspaceProfile, workspace_profile};
pub use prompt::{compact_for_budget, estimate_tokens, system_prompt, trim_for_budget};
pub use runner::{RESUME_NOTE, TaskContext, TaskStart, run_task, workspace_key};
pub use settings::{Settings, SettingsStore};
pub use state::{
    AgentLimits, Checkpoint, CheckpointKind, CommandRecord, FileChange, RollbackResult,
    RollbackSkip, StopReason, TaskHistoryEntry, TaskMetrics, TaskReport, TaskState, TaskStatus,
    TaskSummary,
};
pub use storage::{DATA_DIR_ENV, StorageError, TaskStore, data_dir};
pub use tool_calls::{
    EXPLORER_TOOL_NAMES, MAX_TOOL_RESULT_CHARS, TOOL_NAMES, render_outcome, tool_specs,
    tool_specs_for,
};
pub use verify::{Review, Verdict};
