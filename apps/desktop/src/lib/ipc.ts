import { Channel, invoke } from "@tauri-apps/api/core";

import type { AppInfo } from "./bindings/AppInfo";
import type { ChatEvent } from "./bindings/ChatEvent";
import type { ChatRequest } from "./bindings/ChatRequest";
import type { OllamaStatus } from "./bindings/OllamaStatus";
import type { WorkspaceInfo } from "./bindings/WorkspaceInfo";

export type { AppInfo } from "./bindings/AppInfo";
export type { ChatEvent } from "./bindings/ChatEvent";
export type { ChatMessage } from "./bindings/ChatMessage";
export type { ChatRequest } from "./bindings/ChatRequest";
export type { LoadedModel } from "./bindings/LoadedModel";
export type { ModelInfo } from "./bindings/ModelInfo";
export type { OllamaStatus } from "./bindings/OllamaStatus";
export type { WorkspaceInfo } from "./bindings/WorkspaceInfo";

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
