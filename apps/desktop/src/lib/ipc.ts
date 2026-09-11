import { Channel, invoke } from "@tauri-apps/api/core";

import type { AppInfo } from "./bindings/AppInfo";
import type { ApprovalAction } from "./bindings/ApprovalAction";
import type { ApprovalRequest } from "./bindings/ApprovalRequest";
import type { ChatEvent } from "./bindings/ChatEvent";
import type { ChatRequest } from "./bindings/ChatRequest";
import type { CommandClass } from "./bindings/CommandClass";
import type { CommandResult } from "./bindings/CommandResult";
import type { DirEntry } from "./bindings/DirEntry";
import type { EditFileArgs } from "./bindings/EditFileArgs";
import type { EditFileResult } from "./bindings/EditFileResult";
import type { EnvKey } from "./bindings/EnvKey";
import type { IfExists } from "./bindings/IfExists";
import type { ListDirectoryArgs } from "./bindings/ListDirectoryArgs";
import type { ListDirectoryResult } from "./bindings/ListDirectoryResult";
import type { OllamaStatus } from "./bindings/OllamaStatus";
import type { PermissionDecision } from "./bindings/PermissionDecision";
import type { ReadFileArgs } from "./bindings/ReadFileArgs";
import type { ReadFileResult } from "./bindings/ReadFileResult";
import type { RunCommandArgs } from "./bindings/RunCommandArgs";
import type { SearchArgs } from "./bindings/SearchArgs";
import type { SearchMatch } from "./bindings/SearchMatch";
import type { SearchResult } from "./bindings/SearchResult";
import type { SecretFileView } from "./bindings/SecretFileView";
import type { SecretKind } from "./bindings/SecretKind";
import type { ToolError } from "./bindings/ToolError";
import type { ToolEvent } from "./bindings/ToolEvent";
import type { ToolEventMessage } from "./bindings/ToolEventMessage";
import type { ToolOutcome } from "./bindings/ToolOutcome";
import type { ToolOutput } from "./bindings/ToolOutput";
import type { ToolRequest } from "./bindings/ToolRequest";
import type { Truncation } from "./bindings/Truncation";
import type { WorkspaceInfo } from "./bindings/WorkspaceInfo";
import type { WriteFileArgs } from "./bindings/WriteFileArgs";
import type { WriteFileResult } from "./bindings/WriteFileResult";

export type { AppInfo } from "./bindings/AppInfo";
export type { ApprovalAction } from "./bindings/ApprovalAction";
export type { ApprovalRequest } from "./bindings/ApprovalRequest";
export type { ChatEvent } from "./bindings/ChatEvent";
export type { ChatMessage } from "./bindings/ChatMessage";
export type { ChatRequest } from "./bindings/ChatRequest";
export type { CommandClass } from "./bindings/CommandClass";
export type { CommandResult } from "./bindings/CommandResult";
export type { DirEntry } from "./bindings/DirEntry";
export type { EditFileArgs } from "./bindings/EditFileArgs";
export type { EditFileResult } from "./bindings/EditFileResult";
export type { EnvKey } from "./bindings/EnvKey";
export type { IfExists } from "./bindings/IfExists";
export type { ListDirectoryArgs } from "./bindings/ListDirectoryArgs";
export type { ListDirectoryResult } from "./bindings/ListDirectoryResult";
export type { LoadedModel } from "./bindings/LoadedModel";
export type { ModelInfo } from "./bindings/ModelInfo";
export type { OllamaStatus } from "./bindings/OllamaStatus";
export type { PermissionDecision } from "./bindings/PermissionDecision";
export type { ReadFileArgs } from "./bindings/ReadFileArgs";
export type { ReadFileResult } from "./bindings/ReadFileResult";
export type { RunCommandArgs } from "./bindings/RunCommandArgs";
export type { SearchArgs } from "./bindings/SearchArgs";
export type { SearchMatch } from "./bindings/SearchMatch";
export type { SearchResult } from "./bindings/SearchResult";
export type { SecretFileView } from "./bindings/SecretFileView";
export type { SecretKind } from "./bindings/SecretKind";
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

export function runTool(request: ToolRequest, onEvent: (event: ToolEventMessage) => void): Promise<ToolOutcome> {
  const channel = new Channel<ToolEventMessage>();
  channel.onmessage = onEvent;
  return invoke<ToolOutcome>("run_tool", { request, onEvent: channel });
}

export function respondApproval(id: string, granted: boolean, reason?: string | null): Promise<boolean> {
  return invoke<boolean>("respond_approval", { id, granted, reason });
}
