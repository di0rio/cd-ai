"use client";

import { useEffect, useState } from "react";
import { demoTasks } from "@/lib/demo-session";
import { currentWorkspace, openWorkspace, type WorkspaceInfo } from "@/lib/ipc";
import type { Task } from "@/lib/session";
import { Composer } from "./composer";
import { Conversation } from "./conversation";
import { EmptyWorkspace } from "./empty-workspace";
import { IconButton } from "./icon-button";
import { type PanelKind, SidePanel } from "./side-panel";
import { Sidebar } from "./sidebar";

export function AppShell() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [panel, setPanel] = useState<PanelKind | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const demo = tasks.length > 0 && tasks === demoTasks;

  useEffect(() => {
    // Invented sessions only in development with ?demo; production shows real state only.
    if (process.env.NODE_ENV !== "production" && new URLSearchParams(window.location.search).has("demo")) {
      setTasks(demoTasks);
      setSelectedId(demoTasks[0].id);
    }
  }, []);

  useEffect(() => {
    // Outside Tauri the call rejects and that is expected.
    currentWorkspace().then(setWorkspace, () => {});
  }, []);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "b") {
        event.preventDefault();
        setSidebarOpen((open) => !open);
      } else if (event.key === "Escape") {
        setPanel(null);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const task = tasks.find((candidate) => candidate.id === selectedId) ?? null;
  const togglePanel = (kind: PanelKind) => setPanel((current) => (current === kind ? null : kind));

  const handleOpenWorkspace = async () => {
    try {
      const info = await openWorkspace();
      if (info) {
        setWorkspace(info);
        setWorkspaceError(null);
      }
    } catch (error) {
      setWorkspaceError(String(error));
    }
  };

  return (
    <div className="flex h-full">
      <Sidebar
        open={sidebarOpen}
        workspace={workspace?.name ?? task?.workspace ?? null}
        tasks={tasks}
        selectedId={selectedId}
        onSelect={setSelectedId}
        onOpenWorkspace={handleOpenWorkspace}
      />

      <main className="relative flex min-w-0 flex-1">
        <header className="material absolute inset-x-0 top-0 z-10 flex h-12 items-center gap-2 px-2.5 select-none">
          <IconButton
            label={sidebarOpen ? "Esconder barra lateral (Ctrl+B)" : "Mostrar barra lateral (Ctrl+B)"}
            icon="panelLeft"
            pressed={sidebarOpen}
            onClick={() => setSidebarOpen((open) => !open)}
          />
          {task ? (
            <div className="flex min-w-0 items-baseline gap-2.5">
              <h1 className="truncate font-medium">{task.title}</h1>
              <span className="truncate font-mono text-xs text-ink-faint">
                {task.workspace} · {task.branch}
              </span>
            </div>
          ) : (
            <h1 className="font-medium text-ink-muted">{workspace ? workspace.name : "Nenhum workspace aberto"}</h1>
          )}
          {demo && (
            <span className="shrink-0 rounded-full bg-accent/12 px-2 py-0.5 text-xs text-accent">Demonstração</span>
          )}
          <div className="ml-auto flex gap-0.5">
            <IconButton
              label="Alterações"
              icon="diff"
              pressed={panel === "diff"}
              disabled={!task}
              onClick={() => togglePanel("diff")}
            />
            <IconButton
              label="Terminal"
              icon="terminal"
              pressed={panel === "terminal"}
              disabled={!task}
              onClick={() => togglePanel("terminal")}
            />
          </div>
        </header>

        <div className="flex min-w-0 flex-1 flex-col">
          {task ? (
            <>
              <Conversation key={task.id} task={task} />
              <Composer task={task} />
            </>
          ) : (
            <EmptyWorkspace workspace={workspace} error={workspaceError} onOpen={handleOpenWorkspace} />
          )}
        </div>

        {panel && task && <SidePanel kind={panel} task={task} onClose={() => setPanel(null)} />}
      </main>
    </div>
  );
}
