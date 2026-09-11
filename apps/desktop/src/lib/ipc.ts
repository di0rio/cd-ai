import { invoke } from "@tauri-apps/api/core";

// Mirrors agent_core::AppInfo. The Rust side is the source of truth.
export type AppInfo = {
  name: string;
  version: string;
};

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info");
}
