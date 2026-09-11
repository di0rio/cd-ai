import type { CommandEvent, Task } from "@/lib/session";
import { IconButton } from "./icon-button";
import { Icon } from "./icons";

export type PanelKind = "diff" | "terminal";

const TITLE: Record<PanelKind, string> = { diff: "Alterações", terminal: "Terminal" };

export function SidePanel({ kind, task, onClose }: { kind: PanelKind; task: Task; onClose: () => void }) {
  return (
    <aside
      aria-label={TITLE[kind]}
      className="mt-12 flex w-[22rem] shrink-0 flex-col border-l border-line transition-[translate,opacity] duration-200 ease-out-expo starting:translate-x-3 starting:opacity-0"
    >
      <div className="flex h-10 shrink-0 items-center justify-between pr-2 pl-4">
        <h2 className="font-medium">{TITLE[kind]}</h2>
        <IconButton label="Fechar painel (Esc)" icon="x" onClick={onClose} />
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        {kind === "diff" ? <ChangedFiles task={task} /> : <TerminalLog task={task} />}
      </div>
    </aside>
  );
}

function ChangedFiles({ task }: { task: Task }) {
  const files = new Map<string, { added: number; removed: number }>();
  for (const event of task.events) {
    if (event.kind !== "edit") continue;
    const current = files.get(event.path) ?? { added: 0, removed: 0 };
    files.set(event.path, { added: current.added + event.added, removed: current.removed + event.removed });
  }

  if (files.size === 0) return <p className="text-ink-faint">Nenhum arquivo alterado nesta tarefa.</p>;

  return (
    <>
      <ul>
        {[...files].map(([path, counts]) => (
          <li key={path} className="flex h-8 items-center gap-2 text-[0.8125rem]">
            <Icon name="file" className="size-3.5 text-ink-faint" />
            <code className="min-w-0 truncate">{path}</code>
            <span className="ml-auto flex shrink-0 gap-2 font-mono text-xs tabular-nums">
              <span className="text-ok">+{counts.added}</span>
              <span className={counts.removed > 0 ? "text-bad" : "text-ink-faint"}>−{counts.removed}</span>
            </span>
          </li>
        ))}
      </ul>
      <p className="mt-3 text-xs text-pretty text-ink-faint">O diff linha a linha chega junto com o Tool Engine.</p>
    </>
  );
}

function TerminalLog({ task }: { task: Task }) {
  const commands = task.events.filter((event): event is CommandEvent => event.kind === "command");

  if (commands.length === 0) return <p className="text-ink-faint">Nenhum comando executado nesta tarefa.</p>;

  return (
    <ol className="space-y-4">
      {commands.map((command, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: the same command can run twice; the log is append-only
        <li key={index} className="text-xs">
          <p className="font-mono text-ink">
            <span className="text-ink-faint select-none">$ </span>
            {command.command}
          </p>
          {command.output && (
            <pre className="mt-1.5 text-[0.75rem] leading-relaxed whitespace-pre-wrap text-ink-muted">
              {command.output}
            </pre>
          )}
          <p
            className={`mt-1.5 ${command.exitCode === null ? "text-accent" : command.exitCode === 0 ? "text-ink-faint" : "text-bad"}`}
          >
            {command.exitCode === null ? "rodando…" : `exit ${command.exitCode}`}
          </p>
        </li>
      ))}
    </ol>
  );
}
