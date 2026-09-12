"use client";

import { useEffect, useRef, useState } from "react";
import { formatArgv } from "@/lib/activity";
import type { ApprovalAction, CommandClass } from "@/lib/ipc";
import type { PendingApproval } from "@/lib/session";
import { Icon } from "./icons";

const TITLE: Record<ApprovalAction["type"], string> = {
  runCommand: "Executar este comando?",
  editFile: "Aplicar esta edição?",
  writeFile: "Gravar este arquivo?",
  readFile: "Ler este arquivo?",
};

const COMMAND_CLASS: Record<CommandClass, string> = {
  read: "Comando de leitura",
  validate: "Comando de validação",
  write: "Comando que escreve no disco",
  network: "Comando que usa a rede",
  destructive: "Comando destrutivo",
  unknown: "Comando não classificado",
};

type ApprovalBarProps = {
  approval: PendingApproval;
  onDecision: (granted: boolean, reason?: string) => void;
};

// Inline at the end of the conversation, never a modal: the work behind it stays readable.
// Focus moves here when it appears, and no global shortcut ever approves.
export function ApprovalBar({ approval, onDecision }: ApprovalBarProps) {
  const bar = useRef<HTMLElement>(null);
  const [reason, setReason] = useState("");

  useEffect(() => {
    bar.current?.focus();
  }, []);

  return (
    <section
      ref={bar}
      tabIndex={-1}
      aria-label="Ação aguardando aprovação"
      className="mt-2 rounded-xl border border-warn/40 px-4 py-3.5 transition-[translate,opacity] duration-200 ease-out-expo starting:translate-y-2 starting:opacity-0"
    >
      <div className="flex items-center gap-3">
        <span className="grid size-5 shrink-0 place-items-center rounded-full bg-warn/15 text-warn">
          <Icon name="lock" className="size-3.5" />
        </span>
        <h3 className="font-semibold">{TITLE[approval.action.type]}</h3>
      </div>

      <div className="mt-3">
        <Action action={approval.action} />
      </div>

      <div className="mt-3.5 flex items-center gap-2 border-t border-line pt-3">
        <label className="min-w-0 flex-1">
          <span className="sr-only">Motivo da negação (opcional)</span>
          <input
            type="text"
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            placeholder="Motivo da negação (opcional)"
            className="h-9 w-full rounded-lg border border-line bg-transparent px-2.5 text-[0.8125rem] transition-colors placeholder:text-ink-faint focus:border-accent/60 focus:outline-none"
          />
        </label>
        <button
          type="button"
          onClick={() => onDecision(false, reason.trim() || undefined)}
          className="h-9 shrink-0 rounded-lg px-3.5 text-ink-muted transition-[background-color,color,scale] duration-150 hover:bg-sidebar hover:text-ink active:scale-[0.97]"
        >
          Negar
        </button>
        <button
          type="button"
          onClick={() => onDecision(true)}
          className="h-9 shrink-0 rounded-lg bg-accent px-3.5 font-medium text-accent-ink transition-[scale] duration-100 active:scale-[0.97]"
        >
          Aprovar
        </button>
      </div>
    </section>
  );
}

// Always the exact payload the engine sent: the full argv or the full diff.
function Action({ action }: { action: ApprovalAction }) {
  switch (action.type) {
    case "runCommand":
      return (
        <>
          <pre className="overflow-x-auto rounded-lg bg-sidebar px-3 py-2.5 text-[0.8125rem] leading-relaxed text-ink">
            {formatArgv(action.argv)}
          </pre>
          <p className="mt-1.5 text-xs text-ink-faint">
            {COMMAND_CLASS[action.class]}, sem shell, em <code>{action.cwd}</code>
          </p>
        </>
      );
    case "editFile":
      return (
        <>
          <code className="text-[0.8125rem] text-ink">{action.path}</code>
          <Diff diff={action.diff} />
        </>
      );
    case "writeFile":
      return (
        <>
          <code className="text-[0.8125rem] text-ink">{action.path}</code>
          <p className="mt-1.5 text-xs text-ink-faint">Conteúdo novo, {action.size} bytes.</p>
        </>
      );
    case "readFile":
      return (
        <>
          <code className="text-[0.8125rem] text-ink">{action.path}</code>
          <p className="mt-1.5 text-xs text-pretty text-ink-faint">
            O arquivo tem cara de secret. Mesmo aprovado, o agente recebe só os nomes das chaves, nunca os valores.
          </p>
        </>
      );
  }
}

function Diff({ diff }: { diff: string }) {
  const lines = diff.replace(/\n$/, "").split("\n");

  return (
    <pre className="mt-2 max-h-72 overflow-auto rounded-lg bg-sidebar py-2 text-[0.8125rem] leading-relaxed">
      {lines.map((line, index) => (
        <code
          // biome-ignore lint/suspicious/noArrayIndexKey: a diff is a fixed list of lines, in order
          key={index}
          className={`block px-3 ${
            line.startsWith("+++") || line.startsWith("---")
              ? "text-ink-faint"
              : line.startsWith("+")
                ? "bg-ok/10 text-ok"
                : line.startsWith("-")
                  ? "bg-bad/10 text-bad"
                  : "text-ink-muted"
          }`}
        >
          {line === "" ? " " : line}
        </code>
      ))}
    </pre>
  );
}
