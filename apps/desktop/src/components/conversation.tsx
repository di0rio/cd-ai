"use client";

import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { formatDuration, plural } from "@/lib/format";
import {
  type ActivityBlock,
  type ActivityEvent,
  type Check,
  type CommandEvent,
  type ExploreEvent,
  groupActivity,
  type Task,
} from "@/lib/session";
import { ApprovalBar } from "./approval-bar";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";

const row = "-mx-2 flex min-h-8 w-[calc(100%+1rem)] items-center gap-2.5 rounded-lg px-2 text-left text-ink-muted";
const interactiveRow = `${row} transition-[background-color,color,scale] duration-150 hover:bg-sidebar hover:text-ink active:scale-[0.995]`;

type ConversationProps = {
  task: Task;
  // The decision travels to the Rust core through the caller; this component never decides permission.
  onApprovalDecision?: (granted: boolean, reason?: string) => void;
  // Absent while another task holds the single slot of this workspace, so the offer is never a lie.
  onResume?: () => void;
  // Restores this task's agent writes. Absent while the task is still running.
  onRollback?: (force: boolean) => void;
  rollingBack?: boolean;
};

// Mounted per task (keyed by id), so it opens scrolled to the latest activity.
export function Conversation({ task, onApprovalDecision, onResume, onRollback, rollingBack }: ConversationProps) {
  const scroller = useRef<HTMLDivElement>(null);
  // Following the live text is the default; a user who scrolls up is reading, and is not yanked back.
  const following = useRef(true);
  const pendingId = task.pendingApproval?.id;
  const last = task.events.at(-1);
  // Streaming tokens grow the last row without growing the list, so its length is what moves.
  const tail = last?.kind === "assistant" ? last.text.length : 0;
  const blocks = useMemo(() => groupActivity(task.events), [task.events]);

  // Runs on mount, whenever activity arrives or the last answer grows, and when an approval opens.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally reacts to growth without reading the events.
  useEffect(() => {
    const el = scroller.current;
    if (el && following.current) el.scrollTop = el.scrollHeight;
  }, [task.events.length, tail, pendingId]);

  return (
    <div
      ref={scroller}
      onScroll={(event) => {
        const el = event.currentTarget;
        following.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
      }}
      className="min-h-0 flex-1 overflow-y-auto"
    >
      {/* 46rem of column plus the 24px of breathing room on each side, so the rows line up with the composer. */}
      <div className="mx-auto flex max-w-[49rem] flex-col gap-4 px-6 pt-18 pb-10">
        {blocks.map((block, index) => (
          <Block
            // biome-ignore lint/suspicious/noArrayIndexKey: the activity log is append-only, so positions are stable
            key={index}
            block={block}
            onRollback={onRollback}
            rollingBack={rollingBack}
            rolledBack={task.rolledBack}
          />
        ))}
        {task.stopReason && <StopReason reason={task.stopReason} />}
        {/* Only a task nobody chose to stop: the app closed with work in flight (D9). A cancellation
            was a decision, and offering to undo it one gesture later would contradict it. */}
        {task.stopCause?.kind === "interrupted" && onResume && <Resume onResume={onResume} />}
        {task.pendingApproval && (
          <ApprovalBar
            key={task.pendingApproval.id}
            approval={task.pendingApproval}
            onDecision={onApprovalDecision ?? (() => {})}
          />
        )}
      </div>
    </div>
  );
}

function StopReason({ reason }: { reason: string }) {
  return (
    <div className={`${row} text-ink-faint`}>
      <Icon name="stop" className="size-4" />
      {/* One sentence, not a label and a value: the reason completes the phrase. */}
      <span className="min-w-0 text-pretty">A tarefa parou: {reason}</span>
    </div>
  );
}

// A task that stopped short can be picked up again: the core rebuilds the messages from the
// transcript and tells the model where it left off.
function Resume({ onResume }: { onResume: () => void }) {
  return (
    <div>
      <button
        type="button"
        onClick={onResume}
        className="inline-flex h-9 items-center gap-1.5 rounded-lg bg-signal pr-3.5 pl-2.5 font-medium text-signal-ink transition-[scale] duration-100 active:scale-[0.97]"
      >
        <Icon name="chevron" className="size-4" />
        Retomar
      </button>
    </div>
  );
}

function Block({
  block,
  onRollback,
  rollingBack,
  rolledBack,
}: {
  block: ActivityBlock;
  onRollback?: (force: boolean) => void;
  rollingBack?: boolean;
  rolledBack?: boolean;
}) {
  switch (block.kind) {
    case "user":
      return (
        <div className="ml-auto max-w-[85%] rounded-2xl bg-sidebar px-4 py-2.5 text-[0.9375rem] leading-relaxed">
          <Markdown content={block.text} />
        </div>
      );
    case "assistant":
      return <Markdown content={block.text} />;
    case "explore":
      return <ExploreGroup items={block.items} />;
    case "read":
    case "search":
      return <FailedLookup event={block} />;
    case "edit":
      return (
        <div className={row}>
          <Icon name="pencil" className="size-4 text-ink-faint" />
          <span className="shrink-0">Editou</span>
          <code className="min-w-0 truncate text-ink">{block.path}</code>
          <LineCounts added={block.added} removed={block.removed} />
        </div>
      );
    case "command":
      return <CommandRow event={block} />;
    case "failure":
      return <ToolFailure tool={block.tool} message={block.message} />;
    case "report":
      return (
        <Report
          validated={block.validated}
          summary={block.summary}
          checks={block.checks}
          onRollback={onRollback}
          rollingBack={rollingBack}
          rolledBack={rolledBack}
        />
      );
    case "rollback":
      return (
        <RollbackRow
          restored={block.restored}
          skipped={block.skipped}
          onRollback={onRollback}
          rollingBack={rollingBack}
        />
      );
  }
}

function Collapse({ open, children }: { open: boolean; children: ReactNode }) {
  return (
    <div
      inert={!open}
      className={`grid transition-[grid-template-rows] duration-200 ease-out-expo ${open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"}`}
    >
      <div className="min-h-0 overflow-hidden">{children}</div>
    </div>
  );
}

function Chevron({ open }: { open: boolean }) {
  return (
    <Icon
      name="chevron"
      className={`size-3.5 shrink-0 text-ink-faint transition-transform duration-200 ease-out-expo ${open ? "rotate-90" : ""}`}
    />
  );
}

function ExploreGroup({ items }: { items: ExploreEvent[] }) {
  const [open, setOpen] = useState(false);
  const reads = items.filter((item) => item.kind === "read").length;
  const searches = items.length - reads;
  const summary = [
    reads > 0 ? plural(reads, "arquivo", "arquivos") : null,
    searches > 0 ? plural(searches, "busca", "buscas") : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <div>
      <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)} className={interactiveRow}>
        <Icon name="search" className="size-4 text-ink-faint" />
        <span>Explorou {summary}</span>
        <span className="ml-auto" />
        <Chevron open={open} />
      </button>
      <Collapse open={open}>
        <ul className="mt-1 mb-1 ml-[0.4375rem] space-y-1 border-l border-line py-1 pl-[1.1875rem]">
          {items.map((item, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: the activity log is append-only
            <li key={index} className="flex items-center gap-2 text-[0.8125rem] text-ink-muted">
              <Icon name={item.kind === "read" ? "file" : "search"} className="size-3.5 text-ink-faint" />
              <code className="min-w-0 truncate">{item.kind === "read" ? item.path : item.query}</code>
              {item.kind === "search" && <span className="shrink-0 text-ink-faint">{searchDetail(item)}</span>}
            </li>
          ))}
        </ul>
      </Collapse>
    </div>
  );
}

// The core may send a ready-made summary or just a count; both are optional.
function searchDetail(event: Extract<ActivityEvent, { kind: "search" }>) {
  if (event.summary) return event.summary;
  if (event.matches === undefined) return null;
  return event.matches === 0 ? "nenhum resultado" : plural(event.matches, "resultado", "resultados");
}

function FailedLookup({ event }: { event: Extract<ActivityEvent, { kind: "read" | "search" }> }) {
  return (
    <div className={row}>
      <Icon name="alert" className="size-4 text-bad" />
      <span className="shrink-0">{event.kind === "read" ? "Não conseguiu ler" : "A busca falhou"}</span>
      <code className="min-w-0 truncate text-ink">{event.kind === "read" ? event.path : event.query}</code>
      <span className="ml-auto shrink-0 text-xs text-bad">{event.error}</span>
    </div>
  );
}

// A tool that failed is never collapsed, and the whole message stays readable.
function ToolFailure({ tool, message }: { tool: string; message: string }) {
  return (
    <div className="-mx-2 px-2">
      <div className="flex min-h-8 items-center gap-2.5 text-ink-muted">
        <Icon name="alert" className="size-4 text-bad" />
        <span className="shrink-0">A tool falhou</span>
        <code className="min-w-0 truncate text-ink">{tool}</code>
      </div>
      <p className="mt-0.5 pl-6.5 text-[0.8125rem] text-pretty text-bad">{message}</p>
    </div>
  );
}

function LineCounts({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="ml-auto flex shrink-0 gap-2 font-mono text-xs tabular-nums">
      <span className={added > 0 ? "text-ok" : "text-ink-faint"}>+{added}</span>
      <span className={removed > 0 ? "text-bad" : "text-ink-faint"}>−{removed}</span>
    </span>
  );
}

function CommandRow({ event }: { event: CommandEvent }) {
  const running = event.exitCode === null;
  const failed = !running && event.exitCode !== 0;
  // Errors never collapse by default.
  const [open, setOpen] = useState(failed);
  // A command that starts running (collapsed) and later fails must surface its output.
  useEffect(() => {
    if (failed) setOpen(true);
  }, [failed]);

  const status = running ? (
    <span className="flex items-center gap-1.5 text-signal">
      <span className="size-3 animate-spin rounded-full border-[1.5px] border-current border-t-transparent" />
      rodando
    </span>
  ) : failed ? (
    <span className="text-bad">saiu com {event.exitCode}</span>
  ) : (
    <span className="flex items-center gap-1 text-ink-faint">
      <Icon name="check" className="size-3.5 text-ok" />
      {formatDuration(event.durationMs)}
    </span>
  );

  const header = (
    <>
      <Icon name="terminal" className={`size-4 ${failed ? "text-bad" : "text-ink-faint"}`} />
      <code className="min-w-0 truncate text-ink">{event.command}</code>
      <span className="ml-auto shrink-0 text-xs">{status}</span>
      {event.output && <Chevron open={open} />}
    </>
  );

  if (!event.output) return <div className={row}>{header}</div>;

  return (
    <div>
      <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)} className={interactiveRow}>
        {header}
      </button>
      <Collapse open={open}>
        <pre className="mt-1.5 max-h-64 overflow-auto rounded-lg bg-sidebar px-3 py-2.5 text-[0.8125rem] leading-relaxed text-ink-muted">
          {event.output}
        </pre>
      </Collapse>
    </div>
  );
}

function Report({
  validated,
  summary,
  checks,
  onRollback,
  rollingBack,
  rolledBack,
}: {
  validated: boolean;
  summary: string;
  checks: Check[];
  onRollback?: (force: boolean) => void;
  rollingBack?: boolean;
  rolledBack?: boolean;
}) {
  return (
    <section aria-label="Resultado da tarefa" className="mt-2 rounded-xl border border-line px-4 py-3.5">
      <div className="flex items-start gap-3">
        <span
          className={`mt-px grid size-5 shrink-0 place-items-center rounded-full ${validated ? "bg-ok/15 text-ok" : "bg-warn/15 text-warn"}`}
        >
          <Icon name={validated ? "check" : "alert"} className="size-3.5" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="font-semibold">{validated ? "Validado" : "Não validado"}</h3>
          <div className="mt-0.5 text-pretty text-ink-muted">
            <Markdown content={summary} />
          </div>
        </div>
      </div>
      {checks.length > 0 && (
        <ul className="mt-3 space-y-1.5 border-t border-line pt-3 pl-8">
          {checks.map((check) => (
            <li key={check.command} className="flex items-center gap-2.5 text-[0.8125rem]">
              <Icon
                name={check.ok === null ? "minus" : check.ok ? "check" : "x"}
                className={`size-3.5 ${check.ok === null ? "text-ink-faint" : check.ok ? "text-ok" : "text-bad"}`}
              />
              <span className="w-24 shrink-0">{check.label}</span>
              <code className="min-w-0 truncate text-xs text-ink-faint">{check.command}</code>
              <span className="ml-auto shrink-0 text-xs text-ink-faint">
                {check.ok === null ? "não executado" : check.ok ? "passou" : "falhou"}
              </span>
            </li>
          ))}
        </ul>
      )}
      {onRollback && (
        <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-line pt-3">
          <button
            type="button"
            disabled={rollingBack || rolledBack}
            onClick={() => onRollback(false)}
            className="inline-flex h-8 items-center gap-1.5 rounded-lg px-2.5 text-[0.8125rem] text-ink-muted transition-colors enabled:hover:bg-sidebar enabled:hover:text-ink disabled:text-ink-faint"
          >
            <Icon name="undo" className="size-3.5" />
            {rolledBack ? "Alterações revertidas" : rollingBack ? "Revertendo…" : "Reverter alterações desta tarefa"}
          </button>
        </div>
      )}
    </section>
  );
}

function RollbackRow({
  restored,
  skipped,
  onRollback,
  rollingBack,
}: {
  restored: string[];
  skipped: Array<{ path: string; reason: string; diff: string }>;
  onRollback?: (force: boolean) => void;
  rollingBack?: boolean;
}) {
  return (
    <div className="-mx-2 px-2">
      <div className={row}>
        <Icon name="undo" className="size-4 text-ink-faint" />
        <span>
          {restored.length === 0
            ? "Nenhum arquivo precisou ser revertido"
            : `Reverteu ${plural(restored.length, "arquivo", "arquivos")}`}
        </span>
      </div>
      {skipped.length > 0 && (
        <div className="mt-1 pl-6.5">
          <p className="text-[0.8125rem] text-pretty text-warn">
            {plural(skipped.length, "arquivo", "arquivos")} com mudanças suas não{" "}
            {skipped.length === 1 ? "foi" : "foram"} tocado{skipped.length === 1 ? "" : "s"}.
          </p>
          {onRollback && (
            <button
              type="button"
              disabled={rollingBack}
              onClick={() => onRollback(true)}
              className="mt-1.5 inline-flex h-8 items-center rounded-lg px-2.5 text-[0.8125rem] text-ink-muted transition-colors enabled:hover:bg-sidebar enabled:hover:text-ink"
            >
              Reverter mesmo assim
            </button>
          )}
        </div>
      )}
      {skipped.some((item) => item.diff) && (
        <pre className="mt-1.5 max-h-48 overflow-auto rounded-lg bg-sidebar px-3 py-2.5 text-[0.8125rem] leading-relaxed text-ink-muted">
          {skipped
            .filter((item) => item.diff)
            .map((item) => `${item.path}\n${item.diff}`)
            .join("\n")}
        </pre>
      )}
    </div>
  );
}
