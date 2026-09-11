// Phase 0b model benchmark (docs/decisions/0002).
// Usage: bun scripts/bench-models.ts <model> [model...]
import { writeFile } from "node:fs/promises";
import { freemem } from "node:os";

const OLLAMA = process.env.OLLAMA_HOST ?? "http://localhost:11434";
const CONTEXTS = [8192, 32768];
const FILL_TOKENS = 4000;
const GEN_TOKENS = 128;
const REQUEST_TIMEOUT_MS = 15 * 60_000;
const GB = 1024 ** 3;

type ToolCall = { function: { name: string; arguments: Record<string, unknown> } };
type ChatResponse = {
  message: { content: string; tool_calls?: ToolCall[] };
  done_reason?: string;
  load_duration?: number;
  prompt_eval_count?: number;
  prompt_eval_duration?: number;
  eval_count?: number;
  eval_duration?: number;
};
type PsResponse = { models: { name: string; size: number; size_vram: number }[] };
type ShowResponse = { capabilities?: string[] };

type ContextRun = {
  numCtx: number;
  loadSeconds: number;
  promptTokens: number;
  prefillTokensPerSecond: number;
  genTokens: number;
  genTokensPerSecond: number;
  allocatedGb: number;
  vramGb: number;
  gpuPercent: number;
  freeRamGbAfter: number;
};
type ToolOutcome = "valid" | "no_call" | "wrong_tool" | "bad_args";
type ToolCase = { prompt: string; expected: string };
type ToolCaseResult = ToolCase & {
  outcome: ToolOutcome;
  called?: string;
  genTokens: number;
  doneReason?: string;
  content?: string;
};
type ModelResult =
  | {
      model: string;
      runs: ContextRun[];
      tools: Record<ToolOutcome, number>;
      toolSeconds: number;
      toolCases: ToolCaseResult[];
    }
  | { model: string; error: string };

async function request<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${OLLAMA}${path}`, {
    method: body === undefined ? "GET" : "POST",
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
  });
  if (!res.ok) throw new Error(`${path} ${res.status}: ${await res.text()}`);
  return (await res.json()) as T;
}

const perSecond = (count = 0, durationNs = 0) => (durationNs > 0 ? count / (durationNs / 1e9) : 0);
const round = (value: number, digits = 1) => Number(value.toFixed(digits));

async function unloadAll() {
  const ps = await request<PsResponse>("/api/ps");
  for (const m of ps.models) await request("/api/generate", { model: m.name, keep_alive: 0 });
}

function chat(model: string, thinking: boolean, body: Record<string, unknown>) {
  return request<ChatResponse>("/api/chat", { model, stream: false, ...(thinking ? { think: false } : {}), ...body });
}

const FILL_SNIPPET = `export function parseConfig(raw: string): Config {
  const lines = raw.split("\\n").filter((line) => line.trim() && !line.startsWith("#"));
  const entries = lines.map((line) => line.split("=").map((part) => part.trim()));
  return Object.fromEntries(entries) as Config;
}
`;
// ~4 characters per token is close enough for code-like text; the real count comes back in prompt_eval_count.
const FILL_PROMPT = `${FILL_SNIPPET.repeat(Math.ceil((FILL_TOKENS * 4) / FILL_SNIPPET.length))}
Summarize what the code above does in one sentence.`;

async function runContext(model: string, numCtx: number, thinking: boolean): Promise<ContextRun> {
  await unloadAll();
  // Cold load plus warm-up, so the measured request is not skewed by first-run GPU initialization.
  const warmup = await chat(model, thinking, {
    messages: [{ role: "user", content: "Reply with: ok" }],
    options: { num_ctx: numCtx, num_predict: 4, temperature: 0 },
  });
  const res = await chat(model, thinking, {
    messages: [{ role: "user", content: FILL_PROMPT }],
    options: { num_ctx: numCtx, num_predict: GEN_TOKENS, temperature: 0 },
  });
  // /api/ps reports the full tag, so "devstral-small-2" is listed as "devstral-small-2:latest".
  const loaded = (await request<PsResponse>("/api/ps")).models.find(
    (m) => m.name === model || m.name === `${model}:latest`,
  );
  const size = loaded?.size ?? 0;
  const vram = loaded?.size_vram ?? 0;
  return {
    numCtx,
    loadSeconds: round((warmup.load_duration ?? 0) / 1e9),
    promptTokens: res.prompt_eval_count ?? 0,
    prefillTokensPerSecond: round(perSecond(res.prompt_eval_count, res.prompt_eval_duration)),
    genTokens: res.eval_count ?? 0,
    genTokensPerSecond: round(perSecond(res.eval_count, res.eval_duration)),
    allocatedGb: round(size / GB),
    vramGb: round(vram / GB),
    gpuPercent: size > 0 ? Math.round((vram / size) * 100) : 0,
    freeRamGbAfter: round(freemem() / GB),
  };
}

type ToolSpec = { name: string; description: string; required: string[] };
const TOOL_SPECS: ToolSpec[] = [
  { name: "read_file", description: "Read a file from the workspace", required: ["path"] },
  { name: "search", description: "Search the workspace for text or a regex", required: ["query"] },
  { name: "edit_file", description: "Replace an exact snippet in a file", required: ["path", "search", "replace"] },
  { name: "run_command", description: "Run a shell command in the workspace", required: ["command"] },
];
const TOOLS = TOOL_SPECS.map((spec) => ({
  type: "function",
  function: {
    name: spec.name,
    description: spec.description,
    parameters: {
      type: "object",
      required: spec.required,
      properties: Object.fromEntries(spec.required.map((key) => [key, { type: "string" }])),
    },
  },
}));

const TOOL_CASES: ToolCase[] = [
  { prompt: "Open src/app/page.tsx so we can see how the layout is built.", expected: "read_file" },
  { prompt: "Find where the function getAppInfo is used in the project.", expected: "search" },
  { prompt: "Run the test suite with `cargo test --workspace`.", expected: "run_command" },
  {
    prompt: 'In src/lib/ipc.ts replace `invoke<AppInfo>("app_info")` with `invoke<AppInfo>("get_app_info")`.',
    expected: "edit_file",
  },
  { prompt: "Show me the contents of Cargo.toml at the repository root.", expected: "read_file" },
  { prompt: "Look for every TODO comment in the codebase.", expected: "search" },
  { prompt: "Check the TypeScript types by running `bun run typecheck`.", expected: "run_command" },
  {
    prompt: "In apps/cli/src/main.rs rename the constant declared as `const USAGE: &str` to HELP_TEXT.",
    expected: "edit_file",
  },
  { prompt: "What does crates/agent-core/src/lib.rs export? Read it.", expected: "read_file" },
  { prompt: 'Search for the string "core v" to find where the version label is rendered.', expected: "search" },
];

function classify(call: ToolCall | undefined, expected: string): ToolOutcome {
  if (!call) return "no_call";
  if (call.function.name !== expected) return "wrong_tool";
  const spec = TOOL_SPECS.find((s) => s.name === expected);
  const args = call.function.arguments;
  const ok = spec?.required.every((key) => typeof args[key] === "string" && args[key] !== "") ?? false;
  return ok ? "valid" : "bad_args";
}

async function runTools(model: string, thinking: boolean) {
  const tools: Record<ToolOutcome, number> = { valid: 0, no_call: 0, wrong_tool: 0, bad_args: 0 };
  const toolCases: ToolCaseResult[] = [];
  const started = performance.now();
  for (const toolCase of TOOL_CASES) {
    const res = await chat(model, thinking, {
      tools: TOOLS,
      messages: [
        { role: "system", content: "You are a coding agent. Perform the next step with exactly one tool call." },
        { role: "user", content: toolCase.prompt },
      ],
      // Some models reason in `content` even with think=false; leave room for that before the tool call.
      options: { num_ctx: CONTEXTS[0], num_predict: 1024, temperature: 0 },
    });
    const call = res.message.tool_calls?.[0];
    const outcome = classify(call, toolCase.expected);
    tools[outcome]++;
    toolCases.push({
      ...toolCase,
      outcome,
      called: call?.function.name,
      genTokens: res.eval_count ?? 0,
      ...(outcome === "valid" ? {} : { doneReason: res.done_reason, content: res.message.content.slice(0, 300) }),
    });
  }
  return { tools, toolSeconds: round((performance.now() - started) / 1000), toolCases };
}

async function benchModel(model: string): Promise<ModelResult> {
  const show = await request<ShowResponse>("/api/show", { model });
  const thinking = show.capabilities?.includes("thinking") ?? false;
  const runs: ContextRun[] = [await runContext(model, CONTEXTS[0], thinking)];
  console.log(JSON.stringify(runs[0]));
  // Tool calls reuse the model already loaded at CONTEXTS[0]; changing num_ctx would force a reload into toolSeconds.
  const toolResult = await runTools(model, thinking);
  console.log(JSON.stringify({ tools: toolResult.tools, toolSeconds: toolResult.toolSeconds }));
  for (const numCtx of CONTEXTS.slice(1)) {
    const run = await runContext(model, numCtx, thinking);
    console.log(JSON.stringify(run));
    runs.push(run);
  }
  return { model, runs, ...toolResult };
}

const models = process.argv.slice(2);
if (models.length === 0) {
  console.error("uso: bun scripts/bench-models.ts <modelo> [modelo...]");
  process.exit(2);
}

const results: ModelResult[] = [];
for (const model of models) {
  console.log(`== ${model}`);
  try {
    results.push(await benchModel(model));
  } catch (error) {
    console.error(`${model}: ${String(error)}`);
    results.push({ model, error: String(error) });
  }
}
await unloadAll();

const out = `docs/audit/benchmark-${new Date().toISOString().slice(0, 10)}.json`;
await writeFile(out, `${JSON.stringify({ host: OLLAMA, fillTokens: FILL_TOKENS, results }, null, 2)}\n`);
console.log(`resultado salvo em ${out}`);
