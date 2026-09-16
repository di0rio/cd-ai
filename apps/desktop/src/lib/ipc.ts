import { Channel, invoke } from "@tauri-apps/api/core";

// Only the types these wrappers name are imported; everything else in the IPC surface is
// re-exported below without being imported twice.
import type { AgentEventMessage } from "./bindings/AgentEventMessage";
import type { AppInfo } from "./bindings/AppInfo";
import type { ChatEvent } from "./bindings/ChatEvent";
import type { ChatRequest } from "./bindings/ChatRequest";
import type { OllamaStatus } from "./bindings/OllamaStatus";
import type { PermissionMode } from "./bindings/PermissionMode";
import type { RollbackResult } from "./bindings/RollbackResult";
import type { SandboxStatus } from "./bindings/SandboxStatus";
import type { Settings } from "./bindings/Settings";
import type { TaskHistoryEntry } from "./bindings/TaskHistoryEntry";
import type { TaskSummary } from "./bindings/TaskSummary";
import type { WorkspaceInfo } from "./bindings/WorkspaceInfo";

export type { AgentEvent } from "./bindings/AgentEvent";
export type { AgentEventMessage } from "./bindings/AgentEventMessage";
export type { AgentRole } from "./bindings/AgentRole";
export type { AppInfo } from "./bindings/AppInfo";
export type { ApprovalAction } from "./bindings/ApprovalAction";
export type { ApprovalRequest } from "./bindings/ApprovalRequest";
export type { ChatEvent } from "./bindings/ChatEvent";
export type { ChatMessage } from "./bindings/ChatMessage";
export type { ChatRequest } from "./bindings/ChatRequest";
export type { Checkpoint } from "./bindings/Checkpoint";
export type { CheckpointKind } from "./bindings/CheckpointKind";
export type { CommandClass } from "./bindings/CommandClass";
export type { CommandRecord } from "./bindings/CommandRecord";
export type { CommandResult } from "./bindings/CommandResult";
export type { DirEntry } from "./bindings/DirEntry";
export type { EditFileArgs } from "./bindings/EditFileArgs";
export type { EditFileResult } from "./bindings/EditFileResult";
export type { EnvKey } from "./bindings/EnvKey";
export type { IfExists } from "./bindings/IfExists";
export type { ListDirectoryArgs } from "./bindings/ListDirectoryArgs";
export type { ListDirectoryResult } from "./bindings/ListDirectoryResult";
export type { LoadedModel } from "./bindings/LoadedModel";
export type { MemoryEntry } from "./bindings/MemoryEntry";
export type { MemoryKind } from "./bindings/MemoryKind";
export type { ModelCategory } from "./bindings/ModelCategory";
export type { ModelInfo } from "./bindings/ModelInfo";
export type { OllamaStatus } from "./bindings/OllamaStatus";
export type { PermissionDecision } from "./bindings/PermissionDecision";
export type { PermissionMode } from "./bindings/PermissionMode";
export type { ReadFileArgs } from "./bindings/ReadFileArgs";
export type { ReadFileResult } from "./bindings/ReadFileResult";
export type { RollbackResult } from "./bindings/RollbackResult";
export type { RollbackSkip } from "./bindings/RollbackSkip";
export type { RunCommandArgs } from "./bindings/RunCommandArgs";
export type { SandboxStatus } from "./bindings/SandboxStatus";
export type { SearchArgs } from "./bindings/SearchArgs";
export type { SearchMatch } from "./bindings/SearchMatch";
export type { SearchResult } from "./bindings/SearchResult";
export type { SecretFileView } from "./bindings/SecretFileView";
export type { SecretKind } from "./bindings/SecretKind";
export type { SelectedSkill } from "./bindings/SelectedSkill";
export type { Settings } from "./bindings/Settings";
export type { SkillSkip } from "./bindings/SkillSkip";
export type { StopReason } from "./bindings/StopReason";
export type { TaskHistoryEntry } from "./bindings/TaskHistoryEntry";
export type { TaskKind } from "./bindings/TaskKind";
export type { TaskReport } from "./bindings/TaskReport";
export type { TaskStatus } from "./bindings/TaskStatus";
export type { TaskSummary } from "./bindings/TaskSummary";
export type { ToolError } from "./bindings/ToolError";
export type { ToolEvent } from "./bindings/ToolEvent";
export type { ToolEventMessage } from "./bindings/ToolEventMessage";
export type { ToolOutcome } from "./bindings/ToolOutcome";
export type { ToolOutput } from "./bindings/ToolOutput";
export type { ToolRequest } from "./bindings/ToolRequest";
export type { Truncation } from "./bindings/Truncation";
export type { WorkspaceInfo } from "./bindings/WorkspaceInfo";
export type { WriteFileArgs } from "./bindings/WriteFileArgs";
export type { WriteFileResult } from "./bindings/WriteFileResult";

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

export function openWorkspace(): Promise<WorkspaceInfo | null> {
  return invoke<WorkspaceInfo | null>("open_workspace");
}

export function currentWorkspace(): Promise<WorkspaceInfo | null> {
  return invoke<WorkspaceInfo | null>("current_workspace");
}

export function getOllamaStatus(): Promise<OllamaStatus> {
  return invoke<OllamaStatus>("ollama_status");
}

export async function startChat(request: ChatRequest, onEvent: (event: ChatEvent) => void): Promise<number> {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<number>("chat", { request, onEvent: channel });
}

export function cancelChat(id: number): Promise<boolean> {
  return invoke("cancel_chat", { id });
}

/** Starts a task in the open workspace and resolves with its id. Rejects if one is already running. */
export async function startTask(
  request: string,
  model: string,
  numCtx: number,
  onEvent: (event: AgentEventMessage) => void,
  continues?: string,
): Promise<string> {
  const channel = new Channel<AgentEventMessage>();
  channel.onmessage = onEvent;
  return invoke<string>("start_task", { request, model, numCtx, continues, onEvent: channel });
}

/** Picks an interrupted task back up; rejects if the id is unknown or belongs to another workspace. */
export async function resumeTask(
  taskId: string,
  model: string,
  numCtx: number,
  onEvent: (event: AgentEventMessage) => void,
): Promise<string> {
  const channel = new Channel<AgentEventMessage>();
  channel.onmessage = onEvent;
  return invoke<string>("resume_task", { taskId, model, numCtx, onEvent: channel });
}

export function cancelTask(taskId: string): Promise<boolean> {
  return invoke<boolean>("cancel_task", { taskId });
}

/** Queues a course correction; the loop reads it at the top of its next iteration. */
export function steerTask(taskId: string, text: string): Promise<boolean> {
  return invoke<boolean>("steer_task", { taskId, text });
}

export function listTasks(): Promise<TaskSummary[]> {
  return invoke<TaskSummary[]>("list_tasks");
}

/** Tasks of the open workspace, with files, commands and the baseline checkpoint. */
export function workspaceHistory(): Promise<TaskHistoryEntry[]> {
  return invoke<TaskHistoryEntry[]>("workspace_history");
}

/** Restores this task's agent writes. `force` overwrites files the user changed after the agent. */
export function rollbackTask(taskId: string, force = false): Promise<RollbackResult> {
  return invoke<RollbackResult>("rollback_task", { taskId, force });
}

/** Every event a task recorded, for replaying a conversation from disk. */
export function taskEvents(taskId: string): Promise<AgentEventMessage[]> {
  return invoke<AgentEventMessage[]>("task_events", { taskId });
}

/** What the core remembered from the last runs. */
export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/** Remembers the model for the next runs. The core answers even when it could not write. */
export function setPreferredModel(model: string): Promise<void> {
  return invoke<void>("set_preferred_model", { model });
}

/** Remembers FAST/CODER/REASONER names and the trajectories opt-in. */
export function setRouterSettings(
  fast: string | null,
  coder: string | null,
  reasoner: string | null,
  trajectories: boolean,
): Promise<void> {
  return invoke<void>("set_router_settings", { fast, coder, reasoner, trajectories });
}

export function getSandboxStatus(): Promise<SandboxStatus> {
  return invoke<SandboxStatus>("sandbox_status");
}

/** Persists ASK / AUTO / FULL ACCESS. The core refuses FULL ACCESS without a sandbox. */
export function setPermissionMode(mode: PermissionMode): Promise<PermissionMode> {
  return invoke<PermissionMode>("set_permission_mode", { mode });
}

export function respondApproval(
  taskId: string,
  id: string,
  granted: boolean,
  reason?: string | null,
): Promise<boolean> {
  return invoke<boolean>("respond_approval", { taskId, id, granted, reason });
}
