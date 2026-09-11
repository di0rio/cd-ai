import { invoke } from "@tauri-apps/api/core";

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
