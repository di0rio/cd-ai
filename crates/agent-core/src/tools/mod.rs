pub mod cancel;
pub mod command;
pub mod edit;
pub mod git;
pub mod read;
pub mod search;

use std::time::Instant;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::events::{ToolEvent, ToolEventMessage, timestamp};
use crate::permissions::{
    ApprovalAction, ApprovalRequest, ApprovalResponse, PermissionDecision, PermissionKind,
    PermissionManager, PermissionMode, Policy, policy,
};
use crate::redactor::SecretFileView;
use crate::sandbox::{SandboxCapabilities, status as sandbox_status};
use crate::tools::cancel::CancelToken;
use crate::workspace::Workspace;
use crate::workspace::WorkspaceError;

// Design §2 limits.
pub const MAX_READ_LINES: u64 = 2000;
pub const MAX_READ_BYTES: usize = 200 * 1024;
pub const MAX_SEARCH_RESULTS: usize = 200;
pub const MAX_SEARCH_BYTES: usize = 64 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_LIST_ENTRIES: usize = 500;
pub const MAX_REWRITE_BYTES: usize = 8 * 1024;
pub const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 30_000;

/// How the model asks for a tool run (design §2). Paths are always workspace-relative or absolute
/// and go through `Workspace::resolve` — no raw path is ever used directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(
    tag = "tool",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolRequest {
    ReadFile(ReadFileArgs),
    Search(SearchArgs),
    EditFile(EditFileArgs),
    WriteFile(WriteFileArgs),
    ListDirectory(ListDirectoryArgs),
    RunCommand(RunCommandArgs),
    GitStatus(GitStatusArgs),
    GitDiff(GitDiffArgs),
    GitLog(GitLogArgs),
    GitBranch(GitBranchArgs),
}

impl ToolRequest {
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::ReadFile(_) => "readFile",
            Self::Search(_) => "search",
            Self::EditFile(_) => "editFile",
            Self::WriteFile(_) => "writeFile",
            Self::ListDirectory(_) => "listDirectory",
            Self::RunCommand(_) => "runCommand",
            Self::GitStatus(_) => "gitStatus",
            Self::GitDiff(_) => "gitDiff",
            Self::GitLog(_) => "gitLog",
            Self::GitBranch(_) => "gitBranch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileArgs {
    pub path: String,
    #[serde(default)]
    #[ts(type = "number")]
    pub start_line: Option<u64>,
    #[serde(default)]
    #[ts(type = "number")]
    pub end_line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SearchArgs {
    pub query: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct EditFileArgs {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub if_exists: IfExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum IfExists {
    #[default]
    #[serde(rename = "error")]
    Error,
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ListDirectoryArgs {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RunCommandArgs {
    pub argv: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    #[ts(type = "number")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusArgs {}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffArgs {
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitLogArgs {
    #[serde(default)]
    #[ts(type = "number")]
    pub max_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchArgs {}

/// Everything a tool produced, tagged by tool (design §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(
    tag = "tool",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolOutput {
    ReadFile(ReadFileResult),
    Search(SearchResult),
    EditFile(EditFileResult),
    WriteFile(WriteFileResult),
    ListDirectory(ListDirectoryResult),
    RunCommand(CommandResult),
    GitStatus(GitStatusResult),
    GitDiff(GitDiffResult),
    GitLog(GitLogResult),
    GitBranch(GitBranchResult),
}

/// The uniform envelope for every tool run (design §2 `ToolResult`). Output-only:
/// nothing here is ever parsed back into the engine, so no `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutcome {
    pub ok: bool,
    pub data: Option<ToolOutput>,
    pub error: Option<ToolError>,
    pub truncated: Option<Truncation>,
}

impl ToolOutcome {
    pub fn ok(data: ToolOutput, truncated: Option<Truncation>) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
            truncated,
        }
    }
    pub fn err(error: ToolError) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
            truncated: None,
        }
    }
}

/// Why a result was cut short, with instructions the model can follow (design §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Truncation {
    pub shown: usize,
    #[ts(type = "number")]
    pub total: u64,
    pub how_to_get_more: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ToolError {
    OutsideWorkspace,
    NotFound,
    NotAFile,
    NotADirectory,
    AlreadyExists,
    ParseFailed { detail: String, line: usize },
    AmbiguousEdit,
    EditNotFound,
    CompoundCommand,
    UnknownCommand,
    PermissionDenied { reason: String },
    SecretDenied,
    Cancelled,
    Io(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideWorkspace => write!(f, "caminho fora do workspace"),
            Self::NotFound => write!(f, "arquivo não encontrado"),
            Self::NotAFile => write!(f, "não é um arquivo"),
            Self::NotADirectory => write!(f, "não é uma pasta"),
            Self::AlreadyExists => write!(f, "arquivo já existe"),
            Self::ParseFailed { detail, line } => {
                write!(f, "erro de parse na linha {line}: {detail}")
            }
            Self::AmbiguousEdit => {
                write!(
                    f,
                    "edição ambígua: o bloco aparece mais de uma vez; dê mais contexto"
                )
            }
            Self::EditNotFound => write!(f, "bloco de edição não encontrado no arquivo"),
            Self::CompoundCommand => {
                write!(
                    f,
                    "comandos compostos não são suportados; chame um comando por vez"
                )
            }
            Self::UnknownCommand => write!(f, "comando vazio ou desconhecido"),
            Self::PermissionDenied { reason } => write!(f, "permissão negada: {reason}"),
            Self::SecretDenied => write!(f, "leitura de arquivo de secret negada"),
            Self::Cancelled => write!(f, "tarefa cancelada"),
            Self::Io(message) => write!(f, "erro de E/S: {message}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<WorkspaceError> for ToolError {
    fn from(error: WorkspaceError) -> Self {
        match error {
            WorkspaceError::OutsideWorkspace(_) => Self::OutsideWorkspace,
            WorkspaceError::NotFound(_) => Self::NotFound,
            WorkspaceError::NotADirectory(_) => Self::NotADirectory,
            WorkspaceError::Io(inner) => Self::Io(inner.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileResult {
    pub path: String,
    pub text: String,
    #[ts(type = "number")]
    pub start_line: u64,
    #[ts(type = "number")]
    pub end_line: u64,
    #[ts(type = "number")]
    pub total_lines: u64,
    pub is_truncated: bool,
    pub redacted: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretFileView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: String,
    #[ts(type = "number")]
    pub line_number: u64,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    #[ts(type = "number")]
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct EditFileResult {
    pub path: String,
    pub changed_lines: usize,
    pub removed: usize,
    pub added: usize,
    pub hash_before: String,
    pub hash_after: String,
    pub fuzzy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileResult {
    pub path: String,
    pub size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_file: bool,
    #[ts(type = "number")]
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ListDirectoryResult {
    pub path: String,
    pub entries: Vec<DirEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    #[ts(type = "number")]
    pub id: u64,
    pub exit_code: Option<i32>,
    #[ts(type = "number")]
    pub duration_ms: u64,
    pub output: String,
    pub truncated: bool,
    pub timed_out: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResult {
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffResult {
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitLogEntry {
    pub hash: String,
    pub subject: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitLogResult {
    pub entries: Vec<GitLogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchResult {
    pub output: String,
}

pub type EventSink<'e> = &'e mut dyn FnMut(ToolEventMessage);
pub type Responder<'r> = &'r mut dyn FnMut(ApprovalRequest) -> ApprovalResponse;

/// Orchestrates the tools: resolve → permission → redact → events (design §7).
pub struct ToolEngine {
    pub workspace: Workspace,
    task_id: String,
    permissions: PermissionManager,
    mode: PermissionMode,
    caps: SandboxCapabilities,
    sequence: u64,
    request_id: u64,
    command_next_id: u64,
    cancel: CancelToken,
}

impl ToolEngine {
    pub fn new(workspace: Workspace, task_id: impl Into<String>) -> Self {
        Self::with_mode(workspace, task_id, PermissionMode::Ask)
    }

    pub fn with_mode(
        workspace: Workspace,
        task_id: impl Into<String>,
        mode: PermissionMode,
    ) -> Self {
        Self {
            workspace,
            task_id: task_id.into(),
            permissions: PermissionManager::default(),
            mode,
            caps: sandbox_status().capabilities(),
            sequence: 0,
            request_id: 0,
            command_next_id: 0,
            cancel: CancelToken::default(),
        }
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn sandbox_caps(&self) -> SandboxCapabilities {
        self.caps
    }

    /// Test seam: pretend the host has (or lacks) a sandbox without probing the kernel.
    #[cfg(test)]
    pub(crate) fn set_sandbox_caps(&mut self, caps: SandboxCapabilities) {
        self.caps = caps;
    }

    /// Shares the task's token, so cancelling it also stops whatever the engine is running.
    pub fn set_cancel(&mut self, token: CancelToken) {
        self.cancel = token;
    }

    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    pub fn emit(&mut self, events: EventSink, event: ToolEvent) {
        self.sequence += 1;
        events(ToolEventMessage {
            task_id: self.task_id.clone(),
            sequence: self.sequence,
            at: timestamp(),
            event,
        });
    }

    pub fn ask_approval(
        &mut self,
        events: EventSink,
        responder: Responder,
        action: ApprovalAction,
    ) -> PermissionDecision {
        let request = self.permissions.pending_for(&self.task_id, action);
        let request_id = request.id.clone();
        self.emit(
            events,
            ToolEvent::ApprovalRequired {
                id: request_id.clone(),
                task_id: request.task_id.clone(),
                action: request.action.clone(),
            },
        );
        match responder(request) {
            ApprovalResponse::Granted => {
                self.permissions.resolve(&request_id);
                self.emit(events, ToolEvent::ApprovalGranted { id: request_id });
                PermissionDecision::Granted
            }
            ApprovalResponse::Denied { reason } => {
                self.emit(
                    events,
                    ToolEvent::ApprovalDenied {
                        id: request_id,
                        reason,
                    },
                );
                PermissionDecision::Denied
            }
        }
    }

    /// SPEC §20.4: Auto never opens a prompt; Ask always shows the exact action, never a model
    /// summary. The policy does not read tool output (SPEC §20.5).
    pub fn authorize(
        &mut self,
        events: EventSink,
        responder: Responder,
        kind: PermissionKind,
        action: ApprovalAction,
    ) -> PermissionDecision {
        match policy(self.mode, &kind, self.caps) {
            Policy::Auto => PermissionDecision::Auto,
            Policy::Deny => PermissionDecision::Denied,
            Policy::Ask => self.ask_approval(events, responder, action),
        }
    }

    fn emit_done(
        &mut self,
        events: EventSink,
        tool: &'static str,
        request_id: u64,
        duration_ms: u64,
        decision: PermissionDecision,
        truncated: bool,
    ) {
        self.emit(
            events,
            ToolEvent::ToolCompleted {
                tool: tool.to_string(),
                request_id,
                duration_ms,
                truncated,
                decision,
            },
        );
    }

    fn emit_failed(
        &mut self,
        events: EventSink,
        tool: &'static str,
        request_id: u64,
        message: String,
    ) {
        self.emit(
            events,
            ToolEvent::ToolFailed {
                tool: tool.to_string(),
                request_id,
                message: crate::redactor::redact(&message).text,
            },
        );
    }

    /// Runs a single request end to end and returns its outcome.
    pub fn run_tool(
        &mut self,
        request: ToolRequest,
        events: EventSink,
        responder: Responder,
    ) -> ToolOutcome {
        // A cancelled task runs nothing: no approval is asked and no event is emitted.
        if self.cancel.is_cancelled() {
            return ToolOutcome::err(ToolError::Cancelled);
        }
        self.request_id += 1;
        let request_id = self.request_id;
        let tool: &'static str = request.tool_name();
        self.emit(
            events,
            ToolEvent::ToolStarted {
                tool: tool.to_string(),
                request_id,
            },
        );
        let started = Instant::now();
        let outcome = self.invoke(request, events, responder);
        let duration_ms = started.elapsed().as_millis() as u64;

        match outcome {
            Ok((decision, data, truncated)) => {
                let is_truncated = truncated.is_some();
                self.emit_done(
                    events,
                    tool,
                    request_id,
                    duration_ms,
                    decision,
                    is_truncated,
                );
                ToolOutcome::ok(data, truncated)
            }
            Err(error) => {
                self.emit_failed(events, tool, request_id, error.to_string());
                ToolOutcome::err(error)
            }
        }
    }

    /// Independent non-secret `read_file` calls, I/O in parallel, events and results in request order.
    pub fn run_read_batch(
        &mut self,
        args: Vec<ReadFileArgs>,
        events: EventSink,
        responder: Responder,
    ) -> Vec<ToolOutcome> {
        if args.len() < 2 {
            return args
                .into_iter()
                .map(|item| self.run_tool(ToolRequest::ReadFile(item), events, responder))
                .collect();
        }
        if self.cancel.is_cancelled() {
            return args
                .iter()
                .map(|_| ToolOutcome::err(ToolError::Cancelled))
                .collect();
        }

        let mut prepared: Vec<(u64, ReadFileArgs, PermissionDecision)> =
            Vec::with_capacity(args.len());
        for item in args {
            self.request_id += 1;
            let request_id = self.request_id;
            self.emit(
                events,
                ToolEvent::ToolStarted {
                    tool: "readFile".to_string(),
                    request_id,
                },
            );
            let decision = self.authorize(
                events,
                responder,
                PermissionKind::ReadFile { secret: false },
                ApprovalAction::ReadFile {
                    path: item.path.clone(),
                },
            );
            if decision == PermissionDecision::Denied {
                self.emit_failed(
                    events,
                    "readFile",
                    request_id,
                    "leitura de arquivo de secret negada".to_string(),
                );
                prepared.push((request_id, item, decision));
                continue;
            }
            prepared.push((request_id, item, decision));
        }

        let to_read: Vec<ReadFileArgs> = prepared
            .iter()
            .filter(|(_, _, decision)| *decision != PermissionDecision::Denied)
            .map(|(_, item, _)| item.clone())
            .collect();
        let started = Instant::now();
        let mut loaded = read::read_many(&self.workspace, to_read).into_iter();
        let duration_ms = started.elapsed().as_millis() as u64;

        let mut outcomes = Vec::with_capacity(prepared.len());
        for (request_id, item, decision) in prepared {
            if decision == PermissionDecision::Denied {
                outcomes.push(ToolOutcome::err(ToolError::PermissionDenied {
                    reason: "leitura de arquivo de secret negada".to_string(),
                }));
                continue;
            }
            match loaded.next() {
                Some(Ok(result)) => {
                    let truncated = result.is_truncated.then(|| Truncation {
                        shown: result.end_line as usize,
                        total: result.total_lines,
                        how_to_get_more: "read_file com startLine/endLine".to_string(),
                    });
                    self.emit(
                        events,
                        ToolEvent::FileRead {
                            path: result.path.clone(),
                            start_line: result.start_line,
                            line_count: if result.text.is_empty() {
                                0
                            } else {
                                result
                                    .end_line
                                    .saturating_sub(result.start_line)
                                    .saturating_add(1)
                            },
                            total_lines: result.total_lines,
                            truncated: result.is_truncated,
                            redacted: result.redacted,
                        },
                    );
                    self.emit_done(
                        events,
                        "readFile",
                        request_id,
                        duration_ms,
                        decision,
                        truncated.is_some(),
                    );
                    outcomes.push(ToolOutcome::ok(ToolOutput::ReadFile(result), truncated));
                }
                Some(Err(error)) => {
                    self.emit_failed(events, "readFile", request_id, error.to_string());
                    outcomes.push(ToolOutcome::err(error));
                }
                None => {
                    self.emit_failed(
                        events,
                        "readFile",
                        request_id,
                        format!("falha ao ler {}", item.path),
                    );
                    outcomes.push(ToolOutcome::err(ToolError::Io(item.path)));
                }
            }
        }
        outcomes
    }

    fn invoke(
        &mut self,
        request: ToolRequest,
        events: EventSink,
        responder: Responder,
    ) -> Result<(PermissionDecision, ToolOutput, Option<Truncation>), ToolError> {
        match request {
            ToolRequest::ReadFile(args) => {
                let (decision, result) = read::read_file(self, args, events, responder)?;
                let truncated = if result.is_truncated {
                    Some(Truncation {
                        shown: result.end_line as usize,
                        total: result.total_lines,
                        how_to_get_more: "read_file com startLine/endLine".to_string(),
                    })
                } else {
                    None
                };
                Ok((decision, ToolOutput::ReadFile(result), truncated))
            }
            ToolRequest::ListDirectory(args) => {
                let (result, total_scanned) = read::list_directory(self, args)?;
                let truncated = if result.entries.len() < total_scanned {
                    Some(Truncation {
                        shown: result.entries.len(),
                        total: total_scanned as u64,
                        how_to_get_more: "list_directory sem recursão; use subpastas".to_string(),
                    })
                } else {
                    None
                };
                Ok((
                    PermissionDecision::Auto,
                    ToolOutput::ListDirectory(result),
                    truncated,
                ))
            }
            ToolRequest::Search(args) => {
                let result = search::search(self, args)?;
                let truncated = if (result.matches.len() as u64) < result.total {
                    Some(Truncation {
                        shown: result.matches.len(),
                        total: result.total,
                        how_to_get_more: "refine a query ou reduza o path".to_string(),
                    })
                } else {
                    None
                };
                Ok((
                    PermissionDecision::Auto,
                    ToolOutput::Search(result),
                    truncated,
                ))
            }
            ToolRequest::EditFile(args) => {
                let (decision, result) = edit::edit_file(self, args, events, responder)?;
                Ok((decision, ToolOutput::EditFile(result), None))
            }
            ToolRequest::WriteFile(args) => {
                let (decision, result) = edit::write_file(self, args, events, responder)?;
                Ok((decision, ToolOutput::WriteFile(result), None))
            }
            ToolRequest::RunCommand(args) => {
                let (decision, result) = command::run_command(self, args, events, responder)?;
                let truncated = result.truncated.then(|| Truncation {
                    shown: result.output.len(),
                    total: result.output.len() as u64,
                    how_to_get_more: "reexecute com escopo menor".to_string(),
                });
                Ok((decision, ToolOutput::RunCommand(result), truncated))
            }
            ToolRequest::GitStatus(args) => {
                let (decision, result) = git::git_status(self, args, events, responder)?;
                Ok((decision, ToolOutput::GitStatus(result), None))
            }
            ToolRequest::GitDiff(args) => {
                let (decision, result) = git::git_diff(self, args, events, responder)?;
                Ok((decision, ToolOutput::GitDiff(result), None))
            }
            ToolRequest::GitLog(args) => {
                let (decision, result) = git::git_log(self, args, events, responder)?;
                Ok((decision, ToolOutput::GitLog(result), None))
            }
            ToolRequest::GitBranch(args) => {
                let (decision, result) = git::git_branch(self, args, events, responder)?;
                Ok((decision, ToolOutput::GitBranch(result), None))
            }
        }
    }

    pub fn permissions(&self) -> &PermissionManager {
        &self.permissions
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }
}

/// Small helper to render a canonical path as workspace-relative when possible.
///
/// Always joins with `/`, even on Windows: the result crosses the IPC boundary
/// to the model and the UI, both of which expect a forward-slash contract.
pub fn display_path(workspace: &Workspace, path: &std::path::Path) -> String {
    if let Ok(relative) = path.strip_prefix(workspace.root()) {
        if relative.as_os_str().is_empty() {
            return ".".to_string();
        }
        return relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
    }
    path.display().to_string()
}

/// Reads a file as UTF-8 (lossy for the model window), classifying missing/dir paths first.
pub fn read_utf8_lossy(path: &std::path::Path) -> Result<String, ToolError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ToolError::NotFound,
        _ => ToolError::Io(error.to_string()),
    })?;
    if !metadata.is_file() {
        return Err(ToolError::NotAFile);
    }
    let bytes = std::fs::read(path).map_err(|error| ToolError::Io(error.to_string()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// SHA-256 hex of arbitrary bytes (design D5).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::digest::FixedOutput;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize_fixed())
}

/// Little helper for the hex digest above (kept free of the `hex` crate).
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_match_ipc_contract() {
        assert_eq!(
            ToolRequest::ReadFile(ReadFileArgs {
                path: "a".into(),
                start_line: None,
                end_line: None
            })
            .tool_name(),
            "readFile"
        );
        assert_eq!(
            ToolRequest::RunCommand(RunCommandArgs {
                argv: vec![],
                cwd: None,
                timeout_ms: None
            })
            .tool_name(),
            "runCommand"
        );
    }

    #[test]
    fn outcome_roundtrips_serde() {
        let outcome = ToolOutcome::ok(
            ToolOutput::ListDirectory(ListDirectoryResult {
                path: ".".into(),
                entries: vec![],
            }),
            None,
        );
        let value = serde_json::to_value(&outcome).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["tool"], "listDirectory");
    }

    #[test]
    fn request_serializes_with_tool_tag() {
        let request = ToolRequest::ReadFile(ReadFileArgs {
            path: "src/main.rs".into(),
            start_line: Some(1),
            end_line: None,
        });
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["tool"], "readFile");
        assert_eq!(value["path"], "src/main.rs");
        assert_eq!(value["startLine"], 1);
    }

    #[test]
    fn sha256_hex_is_64_chars() {
        let hash = sha256_hex(b"hello");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn cancelled_engine_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let mut engine = ToolEngine::new(ws, "task_cancel");
        let token = cancel::CancelToken::default();
        engine.set_cancel(token.clone());
        token.cancel();

        let mut seen = Vec::new();
        let mut sink = |message: ToolEventMessage| seen.push(message);
        let mut refuse = |_: ApprovalRequest| -> ApprovalResponse {
            panic!("a cancelled engine must never ask for approval")
        };

        let outcome = engine.run_tool(
            ToolRequest::ListDirectory(ListDirectoryArgs { path: ".".into() }),
            &mut sink,
            &mut refuse,
        );
        assert!(!outcome.ok);
        assert_eq!(outcome.error, Some(ToolError::Cancelled));
        assert!(seen.is_empty(), "nothing runs, so nothing is emitted");
    }

    #[test]
    fn display_path_prefixes_relative() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let canonical = ws.resolve("src/main.rs").unwrap();
        assert_eq!(display_path(&ws, &canonical), "src/main.rs");
        assert_eq!(display_path(&ws, ws.root()), ".");
    }
}
