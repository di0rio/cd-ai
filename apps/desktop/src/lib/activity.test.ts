import { describe, expect, test } from "bun:test";
import { applyAgentEvent, applyToolEvent, formatArgv } from "./activity";
import type { AgentEventMessage, StopReason, ToolEvent, ToolEventMessage } from "./ipc";
import { groupActivity, type Task } from "./session";

function task(overrides: Partial<Task> = {}): Task {
  return {
    id: "task_1",
    title: "Tarefa",
    workspace: "cd-ai",
    branch: "main",
    status: "running",
    updated: "agora",
    phase: "explore",
    model: "qwen3-coder:30b",
    modelLoaded: true,
    contextUsed: 0,
    contextLimit: 32_768,
    events: [],
    ...overrides,
  };
}

type ToolEventBody = Omit<ToolEventMessage, "taskId" | "sequence" | "at">;

let sequence = 0;
function message(body: ToolEventBody): ToolEventMessage {
  sequence += 1;
  return { taskId: "task_1", sequence, at: "2026-09-11T10:00:00.000Z", ...body } as ToolEventMessage;
}

describe("applyToolEvent", () => {
  test("fileRead becomes a read row", () => {
    const next = applyToolEvent(
      task(),
      message({
        event: "fileRead",
        data: { path: "src/soma.ts", startLine: 1, lineCount: 3, totalLines: 3, truncated: false, redacted: 0 },
      }),
    );

    expect(next.events).toEqual([{ kind: "read", path: "src/soma.ts" }]);
  });

  test("fileChanged becomes an edit row with the counts taken from the diff", () => {
    const next = applyToolEvent(
      task(),
      message({
        event: "fileChanged",
        data: {
          path: "src/soma.ts",
          diff: "--- before\n+++ after\n-  return a - b;\n+  return a + b;\n+  // soma\n",
          fuzzy: false,
          hashBefore: "aaa",
          hashAfter: "bbb",
        },
      }),
    );

    expect(next.events).toEqual([{ kind: "edit", path: "src/soma.ts", added: 2, removed: 1 }]);
  });

  test("commandStarted opens a running command and commandCompleted closes it by id", () => {
    const started = applyToolEvent(
      task(),
      message({ event: "commandStarted", data: { id: 7, argv: ["bun", "test"], class: "validate" } }),
    );
    expect(started.events).toEqual([{ kind: "command", id: 7, command: "bun test", exitCode: null, durationMs: 0 }]);

    const completed = applyToolEvent(
      started,
      message({
        event: "commandCompleted",
        data: { id: 7, exitCode: 1, durationMs: 5200, truncated: false, outputLen: 12 },
      }),
    );
    expect(completed.events).toEqual([{ kind: "command", id: 7, command: "bun test", exitCode: 1, durationMs: 5200 }]);
  });

  test("commandCompleted leaves other commands untouched", () => {
    const before = task({
      events: [
        { kind: "command", id: 1, command: "bun test", exitCode: 0, durationMs: 100 },
        { kind: "command", id: 2, command: "bun run typecheck", exitCode: null, durationMs: 0 },
      ],
    });

    const next = applyToolEvent(
      before,
      message({
        event: "commandCompleted",
        data: { id: 2, exitCode: 0, durationMs: 900, truncated: false, outputLen: 0 },
      }),
    );

    expect(next.events[0]).toEqual({ kind: "command", id: 1, command: "bun test", exitCode: 0, durationMs: 100 });
    expect(next.events[1]).toEqual({
      kind: "command",
      id: 2,
      command: "bun run typecheck",
      exitCode: 0,
      durationMs: 900,
    });
  });

  test("approvalRequired holds the task with the exact action", () => {
    const next = applyToolEvent(
      task(),
      message({
        event: "approvalRequired",
        data: {
          id: "aprv_0001",
          taskId: "task_1",
          action: { type: "runCommand", argv: ["cargo", "clippy", "--fix"], class: "write", cwd: "C:/tmp/proj" },
        },
      }),
    );

    expect(next.status).toBe("waiting_approval");
    expect(next.pendingApproval).toEqual({
      id: "aprv_0001",
      action: { type: "runCommand", argv: ["cargo", "clippy", "--fix"], class: "write", cwd: "C:/tmp/proj" },
    });
    expect(next.events).toEqual([]);
  });

  test("approvalGranted clears the pending approval and goes back to running", () => {
    const waiting = task({
      status: "waiting_approval",
      pendingApproval: { id: "aprv_0001", action: { type: "writeFile", path: "novo.ts", size: 42 } },
    });

    const next = applyToolEvent(waiting, message({ event: "approvalGranted", data: { id: "aprv_0001" } }));

    expect(next.status).toBe("running");
    expect(next.pendingApproval).toBeUndefined();
  });

  test("approvalDenied clears the pending approval and goes back to running", () => {
    const waiting = task({
      status: "waiting_approval",
      pendingApproval: { id: "aprv_0001", action: { type: "readFile", path: ".env" } },
    });

    const next = applyToolEvent(
      waiting,
      message({ event: "approvalDenied", data: { id: "aprv_0001", reason: "não quero" } }),
    );

    expect(next.status).toBe("running");
    expect(next.pendingApproval).toBeUndefined();
  });

  test("a decision about another approval leaves the pending one alone", () => {
    const waiting = task({
      status: "waiting_approval",
      pendingApproval: { id: "aprv_0002", action: { type: "readFile", path: ".env" } },
    });

    const next = applyToolEvent(waiting, message({ event: "approvalGranted", data: { id: "aprv_0001" } }));

    expect(next).toBe(waiting);
  });

  test("toolFailed becomes a failure row with the whole message", () => {
    const next = applyToolEvent(
      task(),
      message({
        event: "toolFailed",
        data: { tool: "edit_file", requestId: 3, message: "o trecho aparece 2 vezes no arquivo" },
      }),
    );

    expect(next.events).toEqual([
      { kind: "failure", tool: "edit_file", message: "o trecho aparece 2 vezes no arquivo" },
    ]);
  });

  test("lifecycle events change nothing", () => {
    const before = task({ events: [{ kind: "read", path: "a.ts" }] });

    for (const body of [
      { event: "toolStarted", data: { tool: "read_file", requestId: 1 } },
      {
        event: "toolCompleted",
        data: { tool: "read_file", requestId: 1, durationMs: 3, truncated: false, decision: "auto" },
      },
      { event: "checkpointCreated", data: { hash: "abc123" } },
    ] as ToolEventBody[]) {
      expect(applyToolEvent(before, message(body))).toBe(before);
    }
  });

  test("never mutates the task it is given", () => {
    const before = task();
    applyToolEvent(
      before,
      message({
        event: "fileRead",
        data: { path: "a.ts", startLine: 1, lineCount: 1, totalLines: 1, truncated: false, redacted: 0 },
      }),
    );

    expect(before.events).toEqual([]);
  });
});

describe("applyToolEvent with groupActivity", () => {
  test("reads still fold and failures stay visible", () => {
    const events: ToolEventBody[] = [
      {
        event: "fileRead",
        data: { path: "a.ts", startLine: 1, lineCount: 1, totalLines: 1, truncated: false, redacted: 0 },
      },
      {
        event: "fileRead",
        data: { path: "b.ts", startLine: 1, lineCount: 1, totalLines: 1, truncated: false, redacted: 0 },
      },
      { event: "toolFailed", data: { tool: "read_file", requestId: 2, message: "arquivo não encontrado" } },
      { event: "commandStarted", data: { id: 1, argv: ["bun", "test"], class: "validate" } },
      { event: "commandCompleted", data: { id: 1, exitCode: 1, durationMs: 10, truncated: false, outputLen: 0 } },
    ];

    const result = events.reduce((current, body) => applyToolEvent(current, message(body)), task());
    const blocks = groupActivity(result.events);

    expect(blocks.map((block) => block.kind)).toEqual(["explore", "failure", "command"]);
    expect(blocks[0]).toEqual({
      kind: "explore",
      items: [
        { kind: "read", path: "a.ts" },
        { kind: "read", path: "b.ts" },
      ],
    });
    // A command that failed stands alone, so the conversation can open its output.
    expect(blocks[2]).toEqual({ kind: "command", id: 1, command: "bun test", exitCode: 1, durationMs: 10 });
  });
});

describe("formatArgv", () => {
  test("keeps plain arguments as they are", () => {
    expect(formatArgv(["cargo", "test", "-p", "agent-core"])).toBe("cargo test -p agent-core");
  });

  test("quotes what would otherwise change the command", () => {
    expect(formatArgv(["git", "commit", "-m", "duas palavras", ""])).toBe('git commit -m "duas palavras" ""');
    expect(formatArgv(["echo", 'ele disse "oi"'])).toBe('echo "ele disse \\"oi\\""');
  });
});

type AgentEventBody = Omit<AgentEventMessage, "taskId" | "sequence" | "at">;

let agentSequence = 0;
function agentMessage(body: AgentEventBody): AgentEventMessage {
  agentSequence += 1;
  return {
    taskId: "task_1",
    sequence: agentSequence,
    at: "2026-09-12T10:00:00.000Z",
    ...body,
  } as AgentEventMessage;
}

function reduceAgent(start: Task, bodies: AgentEventMessage[]): Task {
  return bodies.reduce((current, event) => applyAgentEvent(current, event), start);
}

describe("applyAgentEvent", () => {
  test("userMessage becomes a user row", () => {
    const next = applyAgentEvent(task(), agentMessage({ event: "userMessage", data: { text: "corrija a soma" } }));

    expect(next.events).toEqual([{ kind: "user", text: "corrija a soma" }]);
  });

  test("tokens pile up into one assistant row", () => {
    const next = reduceAgent(task(), [
      agentMessage({ event: "token", data: { content: "Vou " } }),
      agentMessage({ event: "token", data: { content: "ler " } }),
      agentMessage({ event: "token", data: { content: "o arquivo." } }),
    ]);

    expect(next.events).toEqual([{ kind: "assistant", text: "Vou ler o arquivo." }]);
  });

  test("assistantMessage closes the turn with the final text", () => {
    const streamed = reduceAgent(task(), [
      agentMessage({ event: "token", data: { content: "Vou ler" } }),
      agentMessage({ event: "assistantMessage", data: { content: "Vou ler o arquivo." } }),
    ]);

    expect(streamed.events).toEqual([{ kind: "assistant", text: "Vou ler o arquivo." }]);
    expect(streamed.streamingAssistant).toBeUndefined();
  });

  test("a closed turn never takes the tokens of the next one", () => {
    const next = reduceAgent(task(), [
      agentMessage({ event: "assistantMessage", data: { content: "Primeiro turno." } }),
      agentMessage({ event: "token", data: { content: "Segundo" } }),
      agentMessage({ event: "token", data: { content: " turno." } }),
    ]);

    expect(next.events).toEqual([
      { kind: "assistant", text: "Primeiro turno." },
      { kind: "assistant", text: "Segundo turno." },
    ]);
  });

  test("a user message interrupts the streaming row", () => {
    const next = reduceAgent(task(), [
      agentMessage({ event: "token", data: { content: "Estou lendo" } }),
      agentMessage({ event: "userMessage", data: { text: "na verdade, rode os testes" } }),
      agentMessage({ event: "token", data: { content: "Ok." } }),
    ]);

    expect(next.events).toEqual([
      { kind: "assistant", text: "Estou lendo" },
      { kind: "user", text: "na verdade, rode os testes" },
      { kind: "assistant", text: "Ok." },
    ]);
  });

  test("modelTurnCompleted updates the context in use", () => {
    const next = applyAgentEvent(
      task(),
      agentMessage({
        event: "modelTurnCompleted",
        data: { promptTokens: 4210, genTokens: 180, promptMs: 300, genMs: 900 },
      }),
    );

    expect(next.contextUsed).toBe(4210);
  });

  test("statusChanged carries the status and the reason, and clears it when there is none", () => {
    const failed = applyAgentEvent(
      task(),
      agentMessage({ event: "statusChanged", data: { status: "failed", reason: "o Ollama caiu" } }),
    );
    expect(failed.status).toBe("failed");
    expect(failed.stopReason).toBe("o Ollama caiu");

    const running = applyAgentEvent(
      failed,
      agentMessage({ event: "statusChanged", data: { status: "running", reason: null } }),
    );
    expect(running.status).toBe("running");
    expect(running.stopReason).toBeUndefined();
  });

  test("taskFinished closes the task with an unvalidated report and its evidence", () => {
    const next = applyAgentEvent(
      task({ events: [{ kind: "user", text: "corrija a soma" }] }),
      agentMessage({
        event: "taskFinished",
        data: {
          status: "completed_unvalidated",
          stopReason: { kind: "finished" },
          report: {
            summary: "Corrigi a soma e rodei os testes.",
            validated: false,
            evidence: [
              { argv: ["bun", "test"], exitCode: 0, durationMs: 5200 },
              { argv: ["bun", "run", "typecheck"], exitCode: null, durationMs: 30_000 },
            ],
            filesChanged: ["src/soma.ts"],
          },
        },
      }),
    );

    expect(next.status).toBe("completed_unvalidated");
    // An ordinary ending is already in the report: no "a tarefa parou" line on top of it.
    expect(next.stopReason).toBeUndefined();
    expect(next.stopCause).toEqual({ kind: "finished" });
    expect(next.events.at(-1)).toEqual({
      kind: "report",
      validated: false,
      summary: "Corrigi a soma e rodei os testes.",
      checks: [
        { label: "bun", command: "bun test", ok: true },
        { label: "bun", command: "bun run typecheck", ok: null },
      ],
    });
  });

  test("every stop reason becomes a sentence of its own", () => {
    const reasons: StopReason[] = [
      { kind: "maxIterations" },
      { kind: "taskTimeout" },
      { kind: "loopDetected", detail: "leu o mesmo arquivo 5 vezes" },
      { kind: "invalidToolCalls" },
      { kind: "modelError", message: "connection refused" },
      { kind: "contextExhausted" },
      { kind: "cancelled" },
      { kind: "interrupted" },
    ];

    const texts = reasons.map((stopReason) => {
      const next = applyAgentEvent(
        task(),
        agentMessage({
          event: "taskFinished",
          data: {
            status: "failed",
            stopReason,
            report: { summary: "", validated: false, evidence: [], filesChanged: [] },
          },
        }),
      );
      return next.stopReason;
    });

    expect(texts.every((text) => typeof text === "string" && text.length > 0)).toBe(true);
    expect(new Set(texts).size).toBe(reasons.length);
    expect(texts[2]).toBe("estava se repetindo (leu o mesmo arquivo 5 vezes)");
    expect(texts[4]).toBe("erro do modelo: connection refused");
  });

  test("the raw stop reason travels with the sentence, so decisions never match on text", () => {
    const next = applyAgentEvent(
      task(),
      agentMessage({
        event: "taskFinished",
        data: {
          status: "cancelled",
          stopReason: { kind: "interrupted" },
          report: { summary: "", validated: false, evidence: [], filesChanged: [] },
        },
      }),
    );

    expect(next.stopCause).toEqual({ kind: "interrupted" });
    expect(next.stopReason).toBe("o app fechou com ela em andamento");

    // Picking the task back up clears both, so nothing of the old ending survives the resume.
    const resumed = applyAgentEvent(
      next,
      agentMessage({ event: "statusChanged", data: { status: "running", reason: null } }),
    );
    expect(resumed.stopCause).toBeUndefined();
    expect(resumed.stopReason).toBeUndefined();
  });

  test("tool events go through applyToolEvent untouched", () => {
    const data: ToolEvent = {
      event: "fileChanged",
      data: {
        path: "src/soma.ts",
        diff: "--- before\n+++ after\n-  return a - b;\n+  return a + b;\n",
        fuzzy: false,
        hashBefore: "aaa",
        hashAfter: "bbb",
      },
    };
    const before = task();

    const viaAgent = applyAgentEvent(before, agentMessage({ event: "tool", data }));
    const viaTool = applyToolEvent(before, message(data as ToolEventBody));

    expect(viaAgent.events).toEqual(viaTool.events);
    expect(viaAgent.events).toEqual([{ kind: "edit", path: "src/soma.ts", added: 1, removed: 1 }]);
  });

  test("the shown phase follows what the agent is doing", () => {
    const implementing = applyAgentEvent(
      task(),
      agentMessage({
        event: "tool",
        data: {
          event: "fileChanged",
          data: { path: "src/soma.ts", diff: "+a\n", fuzzy: false, hashBefore: "a", hashAfter: "b" },
        },
      }),
    );
    expect(implementing.phase).toBe("implement");

    const validating = applyAgentEvent(
      implementing,
      agentMessage({
        event: "tool",
        data: { event: "commandStarted", data: { id: 1, argv: ["bun", "test"], class: "validate" } },
      }),
    );
    expect(validating.phase).toBe("validate");

    const exploring = applyAgentEvent(
      validating,
      agentMessage({
        event: "tool",
        data: {
          event: "fileRead",
          data: { path: "src/soma.ts", startLine: 1, lineCount: 3, totalLines: 3, truncated: false, redacted: 0 },
        },
      }),
    );
    expect(exploring.phase).toBe("explore");

    // A search has no tool event of its own; the call itself is what says it is exploration.
    const searching = applyAgentEvent(
      implementing,
      agentMessage({ event: "toolCallRequested", data: { tool: "search", input: { query: "soma" } } }),
    );
    expect(searching.phase).toBe("explore");
    expect(searching.events).toEqual(implementing.events);
  });

  test("a command that is not validation leaves the phase alone", () => {
    const next = applyAgentEvent(
      task({ phase: "implement" }),
      agentMessage({
        event: "tool",
        data: { event: "commandStarted", data: { id: 1, argv: ["git", "status"], class: "read" } },
      }),
    );

    expect(next.phase).toBe("implement");
  });

  test("events without a row of their own change nothing", () => {
    const before = task({ events: [{ kind: "read", path: "a.ts" }] });

    for (const body of [
      {
        event: "taskStarted",
        data: {
          summary: {
            id: "task_1",
            title: "Tarefa",
            status: "running",
            updatedAt: "2026-09-12T10:00:00.000Z",
            model: "qwen3-coder:30b",
          },
        },
      },
      { event: "modelTurnStarted", data: { iteration: 1, model: "qwen3-coder:30b" } },
      { event: "thinking", data: { content: "hmm" } },
      { event: "toolCallRequested", data: { tool: "read_file", input: { path: "a.ts" } } },
      { event: "toolCallFinished", data: { tool: "read_file", ok: true, detail: "3 linhas", output: null } },
      { event: "contextTrimmed", data: { removedMessages: 2, estimatedTokens: 900 } },
      { event: "retrying", data: { attempt: 2, reason: "timeout" } },
    ] as AgentEventBody[]) {
      expect(applyAgentEvent(before, agentMessage(body))).toBe(before);
    }
  });

  test("never mutates the task it is given", () => {
    const before = task();
    applyAgentEvent(before, agentMessage({ event: "token", data: { content: "oi" } }));
    applyAgentEvent(before, agentMessage({ event: "userMessage", data: { text: "oi" } }));

    expect(before.events).toEqual([]);
    expect(before.streamingAssistant).toBeUndefined();
  });
});

describe("applyAgentEvent replay", () => {
  // The app-shell replays a stored task with `taskEvents(id)`, so the same list has to rebuild
  // the same Task the live stream had built.
  const events: AgentEventMessage[] = [
    agentMessage({ event: "userMessage", data: { text: "o teste de soma falha, corrija" } }),
    agentMessage({ event: "modelTurnStarted", data: { iteration: 1, model: "qwen3-coder:30b" } }),
    agentMessage({ event: "token", data: { content: "Vou ler" } }),
    agentMessage({ event: "token", data: { content: " o arquivo." } }),
    agentMessage({ event: "assistantMessage", data: { content: "Vou ler o arquivo." } }),
    agentMessage({
      event: "modelTurnCompleted",
      data: { promptTokens: 1200, genTokens: 40, promptMs: 100, genMs: 200 },
    }),
    agentMessage({ event: "toolCallRequested", data: { tool: "read_file", input: { path: "src/soma.ts" } } }),
    agentMessage({
      event: "tool",
      data: {
        event: "fileRead",
        data: { path: "src/soma.ts", startLine: 1, lineCount: 3, totalLines: 3, truncated: false, redacted: 0 },
      },
    }),
    agentMessage({
      event: "tool",
      data: {
        event: "fileChanged",
        data: {
          path: "src/soma.ts",
          diff: "--- before\n+++ after\n-  return a - b;\n+  return a + b;\n",
          fuzzy: false,
          hashBefore: "aaa",
          hashAfter: "bbb",
        },
      },
    }),
    agentMessage({
      event: "tool",
      data: { event: "commandStarted", data: { id: 1, argv: ["bun", "test"], class: "validate" } },
    }),
    agentMessage({
      event: "tool",
      data: {
        event: "commandCompleted",
        data: { id: 1, exitCode: 0, durationMs: 5200, truncated: false, outputLen: 9 },
      },
    }),
    agentMessage({ event: "assistantMessage", data: { content: "Corrigi a soma e os testes passaram." } }),
    agentMessage({
      event: "modelTurnCompleted",
      data: { promptTokens: 2400, genTokens: 60, promptMs: 120, genMs: 260 },
    }),
    agentMessage({
      event: "taskFinished",
      data: {
        status: "completed_unvalidated",
        stopReason: { kind: "finished" },
        report: {
          summary: "Corrigi a soma e os testes passaram.",
          validated: false,
          evidence: [{ argv: ["bun", "test"], exitCode: 0, durationMs: 5200 }],
          filesChanged: ["src/soma.ts"],
        },
      },
    }),
  ];

  test("a whole task ends unvalidated, with its activity and its report", () => {
    const result = reduceAgent(task(), events);

    expect(result.status).toBe("completed_unvalidated");
    expect(result.phase).toBe("validate");
    expect(result.contextUsed).toBe(2400);
    expect(result.events).toEqual([
      { kind: "user", text: "o teste de soma falha, corrija" },
      { kind: "assistant", text: "Vou ler o arquivo." },
      { kind: "read", path: "src/soma.ts" },
      { kind: "edit", path: "src/soma.ts", added: 1, removed: 1 },
      { kind: "command", id: 1, command: "bun test", exitCode: 0, durationMs: 5200 },
      { kind: "assistant", text: "Corrigi a soma e os testes passaram." },
      {
        kind: "report",
        validated: false,
        summary: "Corrigi a soma e os testes passaram.",
        checks: [{ label: "bun", command: "bun test", ok: true }],
      },
    ]);
  });

  test("replaying the same list twice rebuilds the same task", () => {
    expect(reduceAgent(task(), events)).toEqual(reduceAgent(task(), events));
  });

  test("the grouped view of a replay still folds the reads", () => {
    const result = reduceAgent(task(), events);

    expect(groupActivity(result.events).map((block) => block.kind)).toEqual([
      "user",
      "assistant",
      "explore",
      "edit",
      "command",
      "assistant",
      "report",
    ]);
  });
});
