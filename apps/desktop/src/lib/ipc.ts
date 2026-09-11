import { Channel, invoke } from "@tauri-apps/api/core";

// Mirrors agent_core::AppInfo. The Rust side is the source of truth.
export type AppInfo = {
  name: string;
  version: string;
};

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}

// Mirrors agent_core::workspace::WorkspaceInfo.
export type WorkspaceInfo = { name: string; root: string };

export function openWorkspace(): Promise<WorkspaceInfo | null> {
  return invoke<WorkspaceInfo | null>("open_workspace");
}

export function currentWorkspace(): Promise<WorkspaceInfo | null> {
  return invoke<WorkspaceInfo | null>("current_workspace");
}

// Mirrors agent_core::ollama::{ModelInfo, LoadedModel, OllamaStatus}.
export type ModelInfo = {
  name: string;
  size_bytes: number;
  parameter_size: string;
  quantization: string;
};

export type LoadedModel = {
  name: string;
  size_bytes: number;
  vram_bytes: number;
};

export type OllamaStatus = {
  reachable: boolean;
  version: string | null;
  models: ModelInfo[];
  loaded: LoadedModel[];
  error: string | null;
};

export function getOllamaStatus(): Promise<OllamaStatus> {
  return invoke<OllamaStatus>("ollama_status");
}

// Mirrors agent_core::ollama::{ChatRequest, ChatEvent}.
export type ChatMessage = { role: "system" | "user" | "assistant"; content: string };

export type ChatRequest = { model: string; messages: ChatMessage[]; numCtx: number };

export type ChatEvent =
  | { event: "token"; data: { content: string } }
  | { event: "thinking"; data: { content: string } }
  | {
      event: "done";
      data: { promptTokens: number; genTokens: number; promptMs: number; genMs: number };
    }
  | { event: "error"; data: { message: string } };

export async function startChat(request: ChatRequest, onEvent: (event: ChatEvent) => void): Promise<number> {
  const channel = new Channel<ChatEvent>();
  channel.onmessage = onEvent;
  return invoke<number>("chat", { request, onEvent: channel });
}

export function cancelChat(id: number): Promise<boolean> {
  return invoke<boolean>("cancel_chat", { id });
}
