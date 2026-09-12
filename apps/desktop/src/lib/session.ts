import type { ApprovalAction, StopReason } from "./ipc";

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
  | { kind: "search"; query: string; matches?: number; summary?: string; error?: string }
  | { kind: "edit"; path: string; added: number; removed: number }
  // `id` is the engine's command id, so a completion can find the row it belongs to.
  | { kind: "command"; id?: number; command: string; exitCode: number | null; durationMs: number; output?: string }
  | { kind: "failure"; tool: string; message: string }
  | { kind: "report"; validated: boolean; summary: string; checks: Check[] };

// The exact action awaiting a decision (Fase 4 design §5.3): full argv or full diff, never a summary.
export type PendingApproval = { id: string; action: ApprovalAction };

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
  pendingApproval?: PendingApproval;
  // The sentence shown to the user, present only when the reason says something the report does not.
  stopReason?: string;
  // The same reason as the core sent it. Decisions read this, never the sentence.
  stopCause?: StopReason;
  // True while the last event is an assistant row still receiving tokens (plan 015, G2).
  streamingAssistant?: boolean;
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
