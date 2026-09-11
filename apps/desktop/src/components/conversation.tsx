"use client";

import { type ReactNode, useEffect, useRef, useState } from "react";
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
import { Icon } from "./icons";

const row = "-mx-2 flex min-h-8 w-[calc(100%+1rem)] items-center gap-2.5 rounded-lg px-2 text-left text-ink-muted";
const interactiveRow = `${row} transition-[background-color,color,scale] duration-150 hover:bg-sidebar hover:text-ink active:scale-[0.995]`;

// Mounted per task (keyed by id), so it opens scrolled to the latest activity.
export function Conversation({ task }: { task: Task }) {
  const scroller = useRef<HTMLDivElement>(null);

  // Runs on mount and whenever a new batch of events arrives.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally reacts to event-list growth without reading it.
  useEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [task.events.length]);

  return (
    <div ref={scroller} className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex max-w-[46rem] flex-col gap-4 px-6 pt-18 pb-10">
        {groupActivity(task.events).map((block, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: the activity log is append-only, so positions are stable
          <Block key={index} block={block} />
        ))}
      </div>
    </div>
  );
}

function Block({ block }: { block: ActivityBlock }) {
  switch (block.kind) {
    case "user":
      return (
        <div className="ml-auto max-w-[85%] rounded-2xl bg-sidebar px-4 py-2.5 text-[0.9375rem] leading-relaxed">
          <InlineCode text={block.text} />
        </div>
      );
    case "assistant":
      return (
        <p className="text-[0.9375rem] leading-relaxed text-pretty">
          <InlineCode text={block.text} />
        </p>
      );
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
    case "report":
      return <Report validated={block.validated} summary={block.summary} checks={block.checks} />;
  }
}

function InlineCode({ text }: { text: string }) {
  return text.split("`").map((part, index) =>
    index % 2 === 1 ? (
      // biome-ignore lint/suspicious/noArrayIndexKey: segments of a static string
      <code key={index} className="rounded-md bg-sidebar px-1 py-0.5 text-[0.85em]">
        {part}
      </code>
    ) : (
      part
    ),
  );
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
              {item.kind === "search" && (
                <span className="shrink-0 text-ink-faint">
                  {item.matches === 0 ? "nenhum resultado" : plural(item.matches, "resultado", "resultados")}
                </span>
              )}
            </li>
          ))}
        </ul>
      </Collapse>
    </div>
  );
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
    <span className="flex items-center gap-1.5 text-accent">
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

function Report({ validated, summary, checks }: { validated: boolean; summary: string; checks: Check[] }) {
  return (
    <section aria-label="Resultado da tarefa" className="mt-2 rounded-xl border border-line px-4 py-3.5">
      <div className="flex items-start gap-3">
        <span
          className={`mt-px grid size-5 shrink-0 place-items-center rounded-full ${validated ? "bg-ok/15 text-ok" : "bg-warn/15 text-warn"}`}
        >
          <Icon name={validated ? "check" : "alert"} className="size-3.5" />
        </span>
        <div className="min-w-0">
          <h3 className="font-semibold">{validated ? "Validado" : "Não validado"}</h3>
          <p className="mt-0.5 text-pretty text-ink-muted">
            <InlineCode text={summary} />
          </p>
        </div>
      </div>
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
    </section>
  );
}
