import type { AgentEventMessage, CommandRecord, StopReason, ToolEvent, ToolEventMessage } from "./ipc";
import type { ActivityEvent, Check, Phase, Task } from "./session";

// Presentation-only reconciliation of the engine's tool events into the conversation view
// (Fase 4 design §7). Pure: the Rust core stays the source of truth for what actually happened.
export function applyToolEvent(task: Task, message: ToolEventMessage): Task {
  switch (message.event) {
    case "fileRead":
      return append(task, { kind: "read", path: message.data.path });

    case "fileChanged": {
      const { added, removed } = countDiffLines(message.data.diff);
      return append(task, { kind: "edit", path: message.data.path, added, removed });
    }

    case "commandStarted":
      return append(task, {
        kind: "command",
        id: message.data.id,
        command: formatArgv(message.data.argv),
        exitCode: null,
        durationMs: 0,
      });

    case "commandCompleted": {
      const { id, exitCode, durationMs } = message.data;
      return {
        ...task,
        events: task.events.map((event) =>
          event.kind === "command" && event.id === id ? { ...event, exitCode, durationMs } : event,
        ),
      };
    }

    case "toolFailed":
      return append(task, { kind: "failure", tool: message.data.tool, message: message.data.message });

    case "approvalRequired":
      return {
        ...task,
        status: "waiting_approval",
        pendingApproval: { id: message.data.id, action: message.data.action },
      };

    case "approvalGranted":
    case "approvalDenied": {
      if (task.pendingApproval?.id !== message.data.id) return task;
      const { pendingApproval: _decided, ...rest } = task;
      return { ...rest, status: "running" };
    }

    // Lifecycle and checkpoint events carry no row of their own.
    default:
      return task;
  }
}

// Presentation-only reconciliation of the loop's events into the conversation view
// (plan 015, Part G2). Pure and total, so replaying a stored task through `taskEvents`
// rebuilds exactly the Task the live stream had built.
export function applyAgentEvent(task: Task, message: AgentEventMessage): Task {
  switch (message.event) {
    case "tool": {
      const { taskId, sequence, at } = message;
      const tool: ToolEventMessage = { taskId, sequence, at, ...message.data };
      return withPhase(applyToolEvent(task, tool), phaseForTool(message.data));
    }

    case "userMessage":
      return append(closeStream(task), { kind: "user", text: message.data.text });

    case "token":
      return appendToken(task, message.data.content);

    case "assistantMessage":
      return closeAssistantText(task, message.data.content);

    // The prompt of the turn that just ended is what is sitting in the context window.
    case "modelTurnCompleted":
      return { ...task, contextUsed: message.data.promptTokens };

    case "statusChanged":
      return withStop({ ...task, status: message.data.status }, null, message.data.reason);

    case "taskFinished": {
      const { status, stopReason, report } = message.data;
      const finished = withStop({ ...closeStream(task), status }, stopReason, describeStopReason(stopReason));
      return append(finished, {
        kind: "report",
        // The core is the one that decides: `validated` is true only with Verifier evidence.
        validated: report.validated,
        summary: report.summary,
        checks: report.evidence.map(toCheck),
      });
    }

    case "rollbackCompleted": {
      const { restored, skipped } = message.data;
      return append({ ...task, rolledBack: true }, { kind: "rollback", restored, skipped });
    }

    // A search is the only exploration with no tool event of its own, so the phase — and only
    // the phase, never a row — is read off the call itself.
    case "toolCallRequested":
      return message.data.tool === "search" ? withPhase(task, "explore") : task;

    // taskStarted, modelTurnStarted, thinking, toolCallFinished, contextTrimmed,
    // contextBudgetCut, contextCompacted, roleChanged, skillsDetected, skillLoaded,
    // skillSkipped and retrying carry no row of their own.
    default:
      return task;
  }
}

// Which phase the header shows is a heuristic over the last visible activity (plan 015, G2).
function phaseForTool(event: ToolEvent): Phase | null {
  switch (event.event) {
    case "fileRead":
      return "explore";
    case "fileChanged":
      return "implement";
    case "commandStarted":
      return event.data.class === "validate" ? "validate" : null;
    default:
      return null;
  }
}

function withPhase(task: Task, phase: Phase | null): Task {
  return phase && phase !== task.phase ? { ...task, phase } : task;
}

// The raw reason and its sentence travel together, and a task that is running again carries neither.
function withStop(task: Task, cause: StopReason | null, reason: string | null): Task {
  const { stopCause: _rawCleared, stopReason: _sentenceCleared, ...rest } = task;
  const withCause = cause ? { ...rest, stopCause: cause } : rest;
  return reason ? { ...withCause, stopReason: reason } : withCause;
}

function appendToken(task: Task, content: string): Task {
  const last = task.events.at(-1);
  if (task.streamingAssistant && last?.kind === "assistant") {
    return { ...task, events: [...task.events.slice(0, -1), { ...last, text: last.text + content }] };
  }
  return { ...append(task, { kind: "assistant", text: content }), streamingAssistant: true };
}

// The message closes the turn and wins over the tokens: it is the whole text, already redacted.
function closeAssistantText(task: Task, content: string): Task {
  const last = task.events.at(-1);
  const closed = closeStream(task);
  if (task.streamingAssistant && last?.kind === "assistant") {
    return { ...closed, events: [...task.events.slice(0, -1), { kind: "assistant", text: content }] };
  }
  return append(closed, { kind: "assistant", text: content });
}

// Nothing accumulates into a closed row, so the next turn opens one of its own.
function closeStream(task: Task): Task {
  if (!task.streamingAssistant) return task;
  const { streamingAssistant: _closed, ...rest } = task;
  return rest;
}

// Evidence carries no label of its own; the program is the shortest honest one.
function toCheck(record: CommandRecord): Check {
  return {
    label: record.argv[0] ?? "comando",
    command: formatArgv(record.argv),
    ok: record.exitCode === null ? null : record.exitCode === 0,
  };
}

// A sentence only when the reason says something the report does not: an ordinary ending is already
// the whole story, and a line saying the task "stopped" would read as a problem that is not there.
function describeStopReason(reason: StopReason): string | null {
  switch (reason.kind) {
    case "finished":
      return null;
    case "verified":
      return null;
    // Each sentence completes "A tarefa parou: …", so none of them repeats "a tarefa".
    case "maxIterations":
      return "chegou ao limite de iterações";
    case "taskTimeout":
      return "passou do tempo limite";
    case "loopDetected":
      return `estava se repetindo (${reason.detail})`;
    case "invalidToolCalls":
      return "o modelo insistiu em chamadas de ferramenta inválidas";
    case "modelError":
      return `erro do modelo: ${reason.message}`;
    case "contextExhausted":
      return "o contexto acabou";
    case "cancelled":
      return "você cancelou";
    case "interrupted":
      return "o app fechou com ela em andamento";
  }
}

function append(task: Task, event: ActivityEvent): Task {
  return { ...task, events: [...task.events, event] };
}

// Unified diff from the engine: `--- before` / `+++ after` header, then one signed line per change.
function countDiffLines(diff: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++") || line.startsWith("---")) continue;
    if (line.startsWith("+")) added += 1;
    else if (line.startsWith("-")) removed += 1;
  }
  return { added, removed };
}

// One readable line out of an argv, without losing where an argument starts and ends.
export function formatArgv(argv: string[]): string {
  return argv.map(quoteArgument).join(" ");
}

function quoteArgument(argument: string): string {
  if (argument === "") return '""';
  if (!/[\s"]/.test(argument)) return argument;
  return `"${argument.replaceAll('"', '\\"')}"`;
}
