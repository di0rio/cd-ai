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
