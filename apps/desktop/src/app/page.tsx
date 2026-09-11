import type { ReactNode } from "react";
import { CoreStatus } from "@/components/core-status";

export default function Home() {
  return (
    <div className="grid h-screen grid-rows-[auto_1fr_12rem]">
      <header className="flex items-center justify-between border-b border-neutral-200 px-4 py-2 dark:border-neutral-800">
        <h1 className="text-sm font-semibold">Cauã AI</h1>
        <CoreStatus />
      </header>

      <main className="grid min-h-0 grid-cols-[16rem_1fr_18rem]">
        <Panel title="Workspace">Nenhum workspace aberto.</Panel>

        <section
          aria-label="Chat"
          className="flex min-h-0 flex-col border-x border-neutral-200 dark:border-neutral-800"
        >
          <div className="flex flex-1 items-center justify-center text-sm text-neutral-500 dark:text-neutral-400">
            Abra um workspace para começar.
          </div>
          <div className="border-t border-neutral-200 p-3 dark:border-neutral-800">
            <label htmlFor="task" className="sr-only">
              Tarefa
            </label>
            <textarea
              id="task"
              disabled
              rows={3}
              placeholder="Descreva a tarefa…"
              className="w-full resize-none rounded-md border border-neutral-300 bg-white px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-60 dark:border-neutral-700 dark:bg-neutral-900"
            />
          </div>
        </section>

        <Panel title="Atividade">Nenhuma atividade.</Panel>
      </main>

      <Panel title="Terminal" className="border-t border-neutral-200 dark:border-neutral-800">
        Nenhum comando executado.
      </Panel>
    </div>
  );
}

function Panel({ title, className = "", children }: { title: string; className?: string; children: ReactNode }) {
  return (
    <section aria-label={title} className={`flex min-h-0 flex-col ${className}`}>
      <h2 className="px-4 pt-3 pb-2 text-xs font-medium tracking-wide text-neutral-500 uppercase dark:text-neutral-400">
        {title}
      </h2>
      <div className="px-4 text-sm text-neutral-500 dark:text-neutral-400">{children}</div>
    </section>
  );
}
