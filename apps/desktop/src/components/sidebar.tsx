import Image from "next/image";
import type { Task, TaskStatus } from "@/lib/session";
import { CoreStatus } from "./core-status";
import { Icon } from "./icons";

const STATUS: Record<TaskStatus, { label: string; dot: string }> = {
  running: { label: "Em andamento", dot: "bg-accent animate-pulse" },
  waiting_approval: { label: "Aguardando aprovação", dot: "bg-warn" },
  completed: { label: "Validada", dot: "bg-ok" },
  completed_unvalidated: { label: "Não validada", dot: "ring-[1.5px] ring-inset ring-warn" },
  failed: { label: "Falhou", dot: "bg-bad" },
  cancelled: { label: "Cancelada", dot: "bg-ink-faint" },
};

type SidebarProps = {
  open: boolean;
  workspace: string | null;
  tasks: Task[];
  selectedId: string | null;
  onSelect: (id: string) => void;
};

const rowButton =
  "flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-left transition-colors disabled:text-ink-faint";

export function Sidebar({ open, workspace, tasks, selectedId, onSelect }: SidebarProps) {
  return (
    <aside
      aria-label="Barra lateral"
      inert={!open}
      className={`shrink-0 overflow-hidden bg-sidebar transition-[width] duration-200 ease-out-expo select-none ${
        open ? "w-64 border-r border-line" : "w-0"
      }`}
    >
      <div className="flex h-full w-64 flex-col">
        <div className="flex h-12 items-center gap-2.5 px-4 font-semibold tracking-[-0.01em]">
          <Image src="/icon.svg" alt="" width={20} height={20} className="size-5 rounded-[5px]" />
          cd-ai
        </div>

        <div className="px-2">
          <button
            type="button"
            disabled
            title="Disponível quando o agente estiver conectado"
            className={`${rowButton} text-ink hover:bg-canvas`}
          >
            <Icon name="plus" />
            Nova tarefa
          </button>
        </div>

        <div className="mt-5 px-2">
          <p className="px-2 pb-1 text-xs text-ink-faint">Workspace</p>
          <button
            type="button"
            disabled
            title="A seleção de pasta chega na próxima fase"
            className={`${rowButton} text-ink hover:bg-canvas`}
          >
            <Icon name="folder" className="size-4 text-ink-faint" />
            <span className="truncate">{workspace ?? "Abrir workspace"}</span>
          </button>
        </div>

        <nav aria-label="Tarefas" className="mt-5 min-h-0 flex-1 overflow-y-auto px-2 pb-2">
          <p className="px-2 pb-1 text-xs text-ink-faint">Tarefas</p>
          {tasks.length === 0 ? (
            <p className="px-2 py-1 text-[0.8125rem] leading-snug text-pretty text-ink-faint">
              As tarefas deste workspace aparecem aqui.
            </p>
          ) : (
            <ul className="space-y-px">
              {tasks.map((task) => {
                const status = STATUS[task.status];
                const active = task.id === selectedId;
                return (
                  <li key={task.id}>
                    <button
                      type="button"
                      onClick={() => onSelect(task.id)}
                      aria-current={active ? "page" : undefined}
                      className={`${rowButton} ${active ? "bg-canvas text-ink" : "text-ink-muted hover:bg-canvas/60 hover:text-ink"}`}
                    >
                      <span className={`size-1.5 shrink-0 rounded-full ${status.dot}`} />
                      <span className="sr-only">{status.label}:</span>
                      <span className="min-w-0 flex-1 truncate">{task.title}</span>
                      <span className="shrink-0 text-xs text-ink-faint">{task.updated}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </nav>

        <div className="space-y-1 border-t border-line px-4 py-3 text-xs text-ink-faint">
          <p className="flex items-center gap-2">
            <span className="size-1.5 rounded-full bg-ink-faint" />
            Ollama · não verificado
          </p>
          <CoreStatus />
        </div>
      </div>
    </aside>
  );
}
