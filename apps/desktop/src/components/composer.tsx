"use client";

import { type RefObject, useEffect, useState } from "react";
import { formatTokens } from "@/lib/format";
import { PHASES, type Phase, type Task, type TaskStatus } from "@/lib/session";
import { IconButton } from "./icon-button";
import { Icon, type IconName } from "./icons";
import { Tooltip, TooltipContent, TooltipTrigger } from "./ui/tooltip";

const PHASE_LABEL: Record<Phase, string> = {
  explore: "Explorar",
  implement: "Implementar",
  validate: "Validar",
  review: "Revisar",
};

type FinishedStatus = Exclude<TaskStatus, "running" | "waiting_approval">;
const OUTCOME: Record<FinishedStatus, { label: string; icon: IconName; tone: string }> = {
  completed: { label: "Validado", icon: "check", tone: "text-ok" },
  completed_unvalidated: { label: "Não validado", icon: "alert", tone: "text-warn" },
  failed: { label: "Falhou", icon: "x", tone: "text-bad" },
  cancelled: { label: "Cancelada", icon: "x", tone: "text-ink-faint" },
};

// SPEC §20.4 has three permission modes; the core implements only ASK (decision 0001). A chooser
// with one option is not a chooser, so the mode shows as the state it is — visible, because the user
// has to know that writes will stop for approval, and inert, because there is nothing to choose yet.
const PERMISSION_MODE = "Perguntar antes";
const PERMISSION_HINT = "Toda escrita e todo comando que não seja de leitura pedem aprovação.";

type ComposerProps = {
  task: Task | null;
  /** True while this workspace has a task in flight: sending steers it instead of starting a new one. */
  running: boolean;
  workspaceOpen: boolean;
  models: string[];
  loadedModels: string[];
  model: string;
  onModelChange: (model: string) => void;
  onStart: (text: string) => void;
  onSteer: (text: string) => void;
  onCancel: () => void;
  error: string | null;
  inputRef: RefObject<HTMLTextAreaElement | null>;
};

export function Composer({
  task,
  running,
  workspaceOpen,
  models,
  loadedModels,
  model,
  onModelChange,
  onStart,
  onSteer,
  onCancel,
  error,
  inputRef,
}: ComposerProps) {
  const [draft, setDraft] = useState("");
  const [steered, setSteered] = useState(false);

  // A correction is never echoed as a conversation row: the core emits the real `userMessage`
  // when the loop drains the queue. This notice only says the text got there.
  useEffect(() => {
    if (!steered) return;
    const timer = setTimeout(() => setSteered(false), 5_000);
    return () => clearTimeout(timer);
  }, [steered]);

  // A running task is pinned to the model it started with; otherwise the Ollama list decides, and
  // the shown value is always one of the options actually offered.
  const names = running && task ? [task.model] : models.length > 0 ? models : task ? [task.model] : [];
  const activeModel = names.includes(model) ? model : (names[0] ?? "");
  const text = draft.trim();
  const canSend = workspaceOpen && text !== "" && (running || model !== "");
  const blocked = !workspaceOpen
    ? "Abra um workspace para dar uma tarefa ao agente"
    : !running && model === ""
      ? "Nenhum modelo disponível no Ollama"
      : undefined;

  // Outside Tauri (demonstration) there is no Ollama answer, so the task's own flag is all there is.
  const loaded =
    loadedModels.length > 0
      ? loadedModels.includes(activeModel)
      : Boolean(task?.modelLoaded && task.model === activeModel);

  const loadedHint = loaded ? "Modelo carregado no Ollama" : "Modelo ainda não carregado no Ollama";

  // When there's no task running but a completed task is selected, the next message continues it.
  const willContinue = !running && task && task.status !== "running" && task.status !== "waiting_approval";

  const submit = () => {
    if (!canSend) return;
    if (running) {
      onSteer(text);
      setSteered(true);
    } else {
      onStart(text);
    }
    setDraft("");
    if (inputRef.current) inputRef.current.style.height = "auto";
  };

  const send = (
    <button
      type="submit"
      disabled={!canSend}
      aria-label={running ? "Enviar correção" : willContinue ? "Continuar tarefa" : "Iniciar tarefa"}
      className="grid size-8 place-items-center rounded-full bg-signal text-signal-ink transition-[scale,opacity] duration-150 active:scale-95 disabled:pointer-events-none disabled:opacity-30"
    >
      <Icon name="arrowUp" />
    </button>
  );

  return (
    <div className="relative px-6 pb-5 before:pointer-events-none before:absolute before:inset-x-0 before:-top-8 before:h-8 before:bg-linear-to-t before:from-canvas before:to-transparent">
      {/* Container, not viewport: the room this column also depends on the sidebar and the side panel. */}
      <div className="@container mx-auto max-w-[46rem]">
        {task && <StatusLine task={task} />}

        {error && (
          <p role="alert" className="mb-2 px-1 text-[0.8125rem] text-pretty text-bad">
            {error}
          </p>
        )}
        {steered && !error && (
          <p aria-live="polite" className="mb-2 px-1 text-[0.8125rem] text-ink-faint">
            Correção na fila: o agente lê no próximo passo.
          </p>
        )}
        {willContinue && (
          <p aria-live="polite" className="mb-2 px-1 text-[0.8125rem] text-ink-faint flex items-center gap-1.5">
            <Icon name="gitBranch" className="size-3.5" />
            <span>Esta mensagem continuará a tarefa selecionada</span>
          </p>
        )}

        <form
          onSubmit={(event) => {
            event.preventDefault();
            submit();
          }}
          className="rounded-2xl border border-line bg-raised transition-colors focus-within:border-signal/60"
        >
          <label htmlFor="composer" className="sr-only">
            Mensagem para o agente
          </label>
          <textarea
            id="composer"
            ref={inputRef}
            rows={1}
            value={draft}
            onChange={(event) => {
              setDraft(event.target.value);
              // Grow with the text up to the max height; CSS field-sizing is not available in WebKitGTK.
              event.target.style.height = "auto";
              event.target.style.height = `${event.target.scrollHeight}px`;
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                submit();
              }
            }}
            placeholder={
              running
                ? "Corrigir o rumo do agente…"
                : willContinue
                  ? "Continuar a tarefa…"
                  : "Descrever uma tarefa para o agente…"
            }
            className="block max-h-50 w-full resize-none bg-transparent px-4 pt-3.5 pb-1 text-[0.9375rem] leading-relaxed placeholder:text-ink-faint focus:outline-none"
          />
          <div className="flex min-w-0 items-center gap-1 px-2.5 pb-2.5">
            {/* In a narrow column only the icons stay: the model name is what has to remain readable. */}
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex shrink-0 items-center gap-1.5 px-1 text-xs text-ink-faint">
                  <Icon name="lock" className="size-3.5" />
                  <span className="hidden @lg:inline">{PERMISSION_MODE}</span>
                  {/* The tooltip is a hover affordance; a reader that never hovers still gets the rule. */}
                  <span className="sr-only">{PERMISSION_HINT}</span>
                </span>
              </TooltipTrigger>
              <TooltipContent>{PERMISSION_HINT}</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <span
                  role="img"
                  aria-label={loadedHint}
                  className={`ml-1 size-1.5 shrink-0 rounded-full ${loaded ? "bg-ok" : "bg-ink-faint"}`}
                />
              </TooltipTrigger>
              <TooltipContent>{loadedHint}</TooltipContent>
            </Tooltip>
            {names.length > 0 ? (
              <Select
                label="Modelo"
                value={activeModel}
                options={names.map((name) => ({ value: name, label: name }))}
                onChange={onModelChange}
                disabled={running || models.length === 0}
                mono
              />
            ) : (
              <span className="px-2 text-xs text-ink-faint">Nenhum modelo</span>
            )}
            <span className="hidden shrink-0 text-xs text-ink-faint @lg:inline">
              {loaded ? "carregado" : "não carregado"}
            </span>
            <div className="ml-auto flex items-center gap-1">
              {running && <IconButton label="Cancelar tarefa" icon="stop" onClick={onCancel} />}
              {blocked ? (
                <Tooltip>
                  {/* A disabled button swallows pointer events, so the reason hangs on a wrapper. */}
                  <TooltipTrigger asChild>
                    <span className="inline-flex">{send}</span>
                  </TooltipTrigger>
                  <TooltipContent>{blocked}</TooltipContent>
                </Tooltip>
              ) : (
                send
              )}
            </div>
          </div>
        </form>
      </div>
    </div>
  );
}

function StatusLine({ task }: { task: Task }) {
  const current = PHASES.indexOf(task.phase);

  return (
    <div className="mb-2 flex items-center gap-4 px-1 text-xs text-ink-faint select-none">
      {task.status === "running" || task.status === "waiting_approval" ? (
        <ol aria-label="Fase da tarefa" className="flex min-w-0 items-center gap-1.5">
          {PHASES.map((phase, index) => (
            // Too narrow for the whole progression: the phases around the current one step aside
            // (still read aloud) instead of the line clipping the one that matters.
            <li
              key={phase}
              aria-current={index === current ? "step" : undefined}
              className={`flex items-center gap-1.5 ${index === current ? "" : "sr-only @lg:not-sr-only"} ${index < current ? "text-ink-muted" : index === current ? "font-medium text-ink" : ""}`}
            >
              {index > 0 && <Icon name="chevron" className="hidden size-3 text-ink-faint @lg:block" />}
              {index === current && <span className="size-1.5 animate-pulse rounded-full bg-signal" />}
              {PHASE_LABEL[phase]}
            </li>
          ))}
        </ol>
      ) : (
        <Outcome status={task.status} />
      )}
      {/* The gauge never wraps: a line that grows a second row as tokens come in would shove the composer. */}
      <label className="ml-auto flex shrink-0 items-center gap-2 whitespace-nowrap">
        <span className="sr-only">Contexto usado</span>
        <meter
          className="context-meter"
          min={0}
          max={task.contextLimit}
          value={task.contextUsed}
          low={task.contextLimit * 0.6}
          high={task.contextLimit * 0.8}
          optimum={0}
        />
        <span className="tabular-nums">
          {formatTokens(task.contextUsed)} de {formatTokens(task.contextLimit)} tokens
        </span>
      </label>
    </div>
  );
}

function Outcome({ status }: { status: FinishedStatus }) {
  const outcome = OUTCOME[status];
  return (
    <span className={`flex items-center gap-1.5 font-medium ${outcome.tone}`}>
      <Icon name={outcome.icon} className="size-3.5" />
      {outcome.label}
    </span>
  );
}

type SelectProps<T extends string> = {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
  mono?: boolean;
};

function Select<T extends string>({ label, value, options, onChange, disabled, mono }: SelectProps<T>) {
  // The floor keeps the model name readable when the row runs out of room, instead of truncating it to a letter.
  return (
    <label className="relative flex min-w-24 items-center rounded-lg text-xs text-ink-muted transition-colors has-enabled:hover:bg-sidebar has-enabled:hover:text-ink">
      <span className="sr-only">{label}</span>
      <select
        value={value}
        disabled={disabled}
        onChange={(event) => {
          const next = options.find((option) => option.value === event.target.value);
          if (next) onChange(next.value);
        }}
        className={`min-w-0 appearance-none truncate bg-transparent py-1.5 pr-6 pl-2 disabled:text-ink-faint ${mono ? "font-mono" : ""}`}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <Icon name="chevronDown" className="pointer-events-none absolute right-1.5 size-3" />
    </label>
  );
}
