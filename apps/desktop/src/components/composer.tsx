"use client";

import { useState } from "react";
import { formatTokens } from "@/lib/format";
import { PHASES, type Phase, type Task, type TaskStatus } from "@/lib/session";
import { Icon, type IconName } from "./icons";

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

type PermissionMode = "ask" | "auto" | "full";
const MODES: { value: PermissionMode; label: string; disabled?: boolean }[] = [
  { value: "ask", label: "Perguntar antes" },
  { value: "auto", label: "Automático" },
  { value: "full", label: "Acesso total (requer sandbox)", disabled: true },
];

export function Composer({ task }: { task: Task }) {
  const [draft, setDraft] = useState("");
  const [mode, setMode] = useState<PermissionMode>("ask");

  return (
    <div className="relative px-6 pb-5 before:pointer-events-none before:absolute before:inset-x-0 before:-top-8 before:h-8 before:bg-linear-to-t before:from-canvas before:to-transparent">
      <div className="mx-auto max-w-[46rem]">
        <StatusLine task={task} />

        <form
          onSubmit={(event) => event.preventDefault()}
          className="rounded-2xl border border-line bg-raised transition-colors focus-within:border-accent/60"
        >
          <label htmlFor="composer" className="sr-only">
            Mensagem para o agente
          </label>
          <textarea
            id="composer"
            rows={1}
            value={draft}
            onChange={(event) => {
              setDraft(event.target.value);
              // Grow with the text up to the max height; CSS field-sizing is not available in WebKitGTK.
              event.target.style.height = "auto";
              event.target.style.height = `${event.target.scrollHeight}px`;
            }}
            placeholder="Responder ao agente…"
            className="block max-h-50 w-full resize-none bg-transparent px-4 pt-3.5 pb-1 text-[0.9375rem] leading-relaxed placeholder:text-ink-faint focus:outline-none"
          />
          <div className="flex items-center gap-1 px-2.5 pb-2.5">
            <Select label="Modo de permissão" value={mode} options={MODES} onChange={setMode} />
            <span className="flex items-center gap-1.5 px-2 text-xs text-ink-muted">
              <span className={`size-1.5 rounded-full ${task.modelLoaded ? "bg-ok" : "bg-ink-faint"}`} />
              <code>{task.model}</code>
              <span className="text-ink-faint">{task.modelLoaded ? "carregado" : "não carregado"}</span>
            </span>
            <button
              type="submit"
              disabled
              aria-label="Enviar"
              title="O agente ainda não está conectado"
              className="ml-auto grid size-8 place-items-center rounded-full bg-accent text-accent-ink transition-[scale,opacity] duration-150 active:scale-95 disabled:opacity-30"
            >
              <Icon name="arrowUp" />
            </button>
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
        <ol aria-label="Fase da tarefa" className="flex items-center gap-1.5">
          {PHASES.map((phase, index) => (
            <li
              key={phase}
              aria-current={index === current ? "step" : undefined}
              className={`flex items-center gap-1.5 ${index < current ? "text-ink-muted" : index === current ? "font-medium text-ink" : ""}`}
            >
              {index > 0 && <Icon name="chevron" className="size-3 text-ink-faint" />}
              {index === current && <span className="size-1.5 animate-pulse rounded-full bg-accent" />}
              {PHASE_LABEL[phase]}
            </li>
          ))}
        </ol>
      ) : (
        <Outcome status={task.status} />
      )}
      <label className="ml-auto flex items-center gap-2">
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
  options: { value: T; label: string; disabled?: boolean }[];
  onChange: (value: T) => void;
};

function Select<T extends string>({ label, value, options, onChange }: SelectProps<T>) {
  return (
    <label className="relative flex items-center rounded-lg text-xs text-ink-muted transition-colors hover:bg-sidebar hover:text-ink">
      <span className="sr-only">{label}</span>
      <select
        value={value}
        onChange={(event) => {
          const next = options.find((option) => option.value === event.target.value);
          if (next) onChange(next.value);
        }}
        className="appearance-none bg-transparent py-1.5 pr-6 pl-2"
      >
        {options.map((option) => (
          <option key={option.value} value={option.value} disabled={option.disabled}>
            {option.label}
          </option>
        ))}
      </select>
      <Icon name="chevronDown" className="pointer-events-none absolute right-1.5 size-3" />
    </label>
  );
}
