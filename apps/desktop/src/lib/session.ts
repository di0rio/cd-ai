// UI view of a task. Mirrors the event stream the Rust core will emit (SPEC §22–23).
export type TaskStatus =
  | "running"
  | "waiting_approval"
  | "completed"
  | "completed_unvalidated"
  | "failed"
  | "cancelled";

export const PHASES = ["explore", "implement", "validate", "review"] as const;
export type Phase = (typeof PHASES)[number];

export type Check = { label: string; command: string; ok: boolean | null };

export type ActivityEvent =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string }
  | { kind: "read"; path: string; error?: string }
  | { kind: "search"; query: string; matches: number; error?: string }
  | { kind: "edit"; path: string; added: number; removed: number }
  | { kind: "command"; command: string; exitCode: number | null; durationMs: number; output?: string }
  | { kind: "report"; validated: boolean; summary: string; checks: Check[] };

export type ExploreEvent = Extract<ActivityEvent, { kind: "read" | "search" }>;
export type CommandEvent = Extract<ActivityEvent, { kind: "command" }>;
export type ActivityBlock = { kind: "explore"; items: ExploreEvent[] } | ActivityEvent;

export type Task = {
  id: string;
  title: string;
  workspace: string;
  branch: string;
  status: TaskStatus;
  updated: string;
  phase: Phase;
  model: string;
  modelLoaded: boolean;
  contextUsed: number;
  contextLimit: number;
  events: ActivityEvent[];
};

// Routine exploration (successful reads and searches in a row) folds into one block; failures stay standalone.
export function groupActivity(events: ActivityEvent[]): ActivityBlock[] {
  const blocks: ActivityBlock[] = [];
  for (const event of events) {
    const last = blocks.at(-1);
    if ((event.kind === "read" || event.kind === "search") && !event.error) {
      if (last?.kind === "explore") last.items.push(event);
      else blocks.push({ kind: "explore", items: [event] });
    } else {
      blocks.push(event);
    }
  }
  return blocks;
}
