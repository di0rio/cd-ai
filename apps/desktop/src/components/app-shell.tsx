"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { applyAgentEvent } from "@/lib/activity";
import { demoRunningId, demoTasks } from "@/lib/demo-session";
import {
  type AgentEventMessage,
  cancelTask,
  currentWorkspace,
  getSandboxStatus,
  getSettings,
  listTasks,
  openWorkspace,
  type PermissionMode,
  type RollbackResult,
  respondApproval,
  resumeTask,
  rollbackTask,
  type SandboxStatus,
  setPermissionMode,
  setPreferredModel,
  startTask,
  steerTask,
  type TaskSummary,
  taskEvents,
  type WorkspaceInfo,
} from "@/lib/ipc";
import { defaultModel } from "@/lib/models";
import type { Task } from "@/lib/session";
import { Composer } from "./composer";
import { Conversation } from "./conversation";
import { EmptyWorkspace } from "./empty-workspace";
import { IconButton } from "./icon-button";
import { ollamaModels, useOllamaStatus } from "./ollama-status";
import { type PanelKind, SidePanel } from "./side-panel";
import { Sidebar } from "./sidebar";
import { TooltipProvider } from "./ui/tooltip";

// Decision 0002, rule 3: every task the UI starts gets the same context window.
const NUM_CTX = 16_384;
const noop = () => {};

export function AppShell() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [panel, setPanel] = useState<PanelKind | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [demo, setDemo] = useState(false);
  const [chosenModel, setChosenModel] = useState<string | null>(null);
  const [preferredModel, setPreferred] = useState<string | null>(null);
  const [permissionMode, setPermission] = useState<PermissionMode>("ask");
  const [sandbox, setSandbox] = useState<SandboxStatus | null>(null);
  const [runningId, setRunningId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [taskError, setTaskError] = useState<string | null>(null);
  const [rollingBack, setRollingBack] = useState(false);
  const [zoomLevel, setZoomLevel] = useState(1);

  const replayed = useRef(new Set<string>());
  const workspaceName = useRef("");
  const composerInput = useRef<HTMLTextAreaElement>(null);
  const queued = useRef<AgentEventMessage[]>([]);
  const frame = useRef<number | null>(null);
  const ollama = useOllamaStatus();
  const { models, loaded: loadedModels } = ollamaModels(ollama);

  useEffect(() => {
    if (process.env.NODE_ENV !== "production" && new URLSearchParams(window.location.search).has("demo")) {
      setTasks(demoTasks);
      setSelectedId(demoTasks[0].id);
      setRunningId(demoRunningId);
      setDemo(true);
      for (const task of demoTasks) replayed.current.add(task.id);
      return;
    }
    getSettings().then((settings) => {
      setPreferred(settings.model);
      setPermission(settings.permissionMode);
    }, noop);
    getSandboxStatus().then(setSandbox, noop);
    currentWorkspace().then(setWorkspace, noop);
  }, []);

  useEffect(() => {
    workspaceName.current = workspace?.name ?? "";
  }, [workspace]);

  useEffect(() => {
    if (demo || !workspace) return;
    let cancelled = false;
    listTasks().then((summaries) => {
      if (cancelled) return;
      setTasks((previous) => {
        const live = new Map(previous.map((task) => [task.id, task]));
        return summaries.map((summary) => live.get(summary.id) ?? fromSummary(summary, workspace.name));
      });
    }, noop);
    return () => {
      cancelled = true;
    };
  }, [demo, workspace]);

  useEffect(() => {
    const id = selectedId;
    if (!id || replayed.current.has(id)) return;
    replayed.current.add(id);
    let cancelled = false;
    taskEvents(id).then(
      (events) => {
        if (cancelled) return;
        setTasks((previous) =>
          previous.map((task) =>
            task.id === id ? events.reduce(applyAgentEvent, { ...task, events: [], contextUsed: 0 }) : task,
          ),
        );
      },
      (error) => {
        if (!cancelled) setTaskError(String(error));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [selectedId]);

  const handleEvent = useCallback((message: AgentEventMessage) => {
    queued.current.push(message);
    if (frame.current != null) return;
    frame.current = window.requestAnimationFrame(() => {
      frame.current = null;
      const batch = queued.current;
      queued.current = [];
      if (batch.length === 0) return;
      setTasks((previous) => {
        let next = previous;
        for (const item of batch) next = applyLive(next, item, workspaceName.current);
        return next;
      });
      for (const item of batch) {
        if (item.event === "taskStarted") {
          replayed.current.add(item.taskId);
          setRunningId(item.taskId);
          setSelectedId(item.taskId);
        } else if (item.event === "taskFinished") {
          setRunningId((current) => (current === item.taskId ? null : current));
        }
      }
    });
  }, []);

  useEffect(() => {
    document.documentElement.style.zoom = String(zoomLevel);
  }, [zoomLevel]);

  useEffect(() => {
    return () => {
      document.documentElement.style.zoom = "";
      if (frame.current != null) window.cancelAnimationFrame(frame.current);
    };
  }, []);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const mod = event.ctrlKey || event.metaKey;
      if (mod && event.key.toLowerCase() === "b") {
        event.preventDefault();
        setSidebarOpen((open) => !open);
      } else if (event.key === "Escape") {
        setPanel(null);
      } else if (mod && (event.key === "=" || event.key === "+")) {
        event.preventDefault();
        setZoomLevel((z) => Math.min(z + 0.1, 2));
      } else if (mod && event.key === "-") {
        event.preventDefault();
        setZoomLevel((z) => Math.max(z - 0.1, 0.5));
      } else if (mod && event.key === "0") {
        event.preventDefault();
        setZoomLevel(1);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const task = tasks.find((candidate) => candidate.id === selectedId) ?? null;
  const model = chosenModel ?? defaultModel(preferredModel, loadedModels, models);
  const running = starting || runningId !== null;
  const togglePanel = (kind: PanelKind) => setPanel((current) => (current === kind ? null : kind));
  const live = !demo;
  const rollbackBusy = running || task?.status === "running" || task?.status === "waiting_approval";

  const handleModelChange = (next: string) => {
    setChosenModel(next);
    setPreferredModel(next).catch(noop);
  };

  const handlePermissionMode = (next: PermissionMode) => {
    setPermission(next);
    if (demo) return;
    setPermissionMode(next)
      .then(setPermission)
      .catch((error) => setTaskError(String(error)));
  };

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

  const handleStart = async (text: string) => {
    setTaskError(null);
    setStarting(true);
    try {
      const continues =
        task && task.workspace === workspace?.name && task.status !== "running" && task.status !== "waiting_approval"
          ? task.id
          : undefined;
      setRunningId(await startTask(text, model, NUM_CTX, handleEvent, continues));
    } catch (error) {
      setTaskError(String(error));
    } finally {
      setStarting(false);
    }
  };

  const handleSteer = async (text: string) => {
    if (!runningId) return;
    setSelectedId(runningId);
    try {
      if (!(await steerTask(runningId, text))) setTaskError("a tarefa não está mais em andamento");
    } catch (error) {
      setTaskError(String(error));
    }
  };

  const handleCancel = async () => {
    if (!runningId) return;
    try {
      if (!(await cancelTask(runningId))) setTaskError("a tarefa não está mais em andamento");
    } catch (error) {
      setTaskError(String(error));
    }
  };

  const handleResume = async () => {
    if (!task) return;
    setTaskError(null);
    setStarting(true);
    try {
      setRunningId(await resumeTask(task.id, task.model || model, NUM_CTX, handleEvent));
    } catch (error) {
      setTaskError(String(error));
    } finally {
      setStarting(false);
    }
  };

  const markRolledBack = (id: string, restored: string[], skipped: RollbackResult["skipped"]) => {
    setTasks((previous) =>
      previous.map((candidate) =>
        candidate.id === id
          ? applyAgentEvent(candidate, {
              taskId: id,
              sequence: 0,
              at: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
              event: "rollbackCompleted",
              data: { restored, skipped },
            })
          : candidate,
      ),
    );
  };

  const handleRollback = async (force: boolean) => {
    if (!task) return;
    setTaskError(null);
    setRollingBack(true);
    try {
      const result = await rollbackTask(task.id, force);
      markRolledBack(task.id, result.restored, result.skipped);
    } catch (error) {
      setTaskError(String(error));
    } finally {
      setRollingBack(false);
    }
  };

  const handleDemoRollback = () => {
    if (task) markRolledBack(task.id, ["apps/cli/src/main.rs"], []);
  };

  const handleApproval = async (granted: boolean, reason?: string) => {
    const pending = task?.pendingApproval;
    if (!task || !pending) return;
    try {
      if (!(await respondApproval(task.id, pending.id, granted, reason ?? null))) {
        setTaskError("o pedido de aprovação não está mais aberto");
      }
    } catch (error) {
      setTaskError(String(error));
    }
  };

  const handleNewTask = () => {
    setSelectedId(null);
    setPanel(null);
    setTaskError(null);
    composerInput.current?.focus();
  };

  return (
    <TooltipProvider delayDuration={400}>
      <div className="flex h-full">
        <Sidebar
          open={sidebarOpen}
          workspace={workspace?.name ?? task?.workspace ?? null}
          workspaceOpen={Boolean(workspace)}
          tasks={tasks}
          selectedId={selectedId}
          onSelect={setSelectedId}
          onOpenWorkspace={handleOpenWorkspace}
          onNewTask={handleNewTask}
          ollama={ollama}
        />

        <main className="relative flex min-w-0 flex-1">
          <header className="material absolute inset-x-0 top-0 z-10 flex h-12 items-center gap-2 px-2.5 select-none">
            {task ? (
              <div className="flex min-w-0 items-baseline gap-2.5">
                <h1 className="truncate font-medium">{task.title}</h1>
                <span className="truncate font-mono text-xs text-ink-faint">
                  {[task.workspace, task.branch].filter(Boolean).join(" · ")}
                </span>
              </div>
            ) : (
              <h1 className="font-medium text-ink-muted">{workspace ? workspace.name : "Nenhum workspace aberto"}</h1>
            )}
            {demo && (
              <span className="shrink-0 rounded-full bg-signal/12 px-2 py-0.5 text-xs text-signal">Demonstração</span>
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
              <Conversation
                key={task.id}
                task={task}
                onApprovalDecision={live ? handleApproval : undefined}
                onResume={demo ? noop : running ? undefined : handleResume}
                onRollback={demo ? handleDemoRollback : rollbackBusy ? undefined : handleRollback}
                rollingBack={rollingBack}
              />
            ) : (
              <EmptyWorkspace workspace={workspace} error={workspaceError} onOpen={handleOpenWorkspace} />
            )}
            {(workspace || task) && (
              <Composer
                task={task}
                running={running}
                workspaceOpen={demo || Boolean(workspace)}
                models={models}
                loadedModels={loadedModels}
                model={model}
                permissionMode={permissionMode}
                sandboxAvailable={demo || Boolean(sandbox?.available)}
                sandboxDetail={sandbox?.detail ?? "sandbox só existe no Linux nesta versão"}
                onPermissionModeChange={handlePermissionMode}
                onModelChange={live ? handleModelChange : setChosenModel}
                onStart={live ? handleStart : noop}
                onSteer={live ? handleSteer : noop}
                onCancel={live ? handleCancel : noop}
                error={taskError}
                inputRef={composerInput}
              />
            )}
          </div>

          {panel && task && <SidePanel kind={panel} task={task} onClose={() => setPanel(null)} />}
        </main>
      </div>
    </TooltipProvider>
  );
}

function taskShell(id: string, title: string, model: string, workspace: string): Task {
  return {
    id,
    title,
    workspace,
    branch: "",
    status: "running",
    updated: "agora",
    phase: "explore",
    model,
    modelLoaded: false,
    contextUsed: 0,
    contextLimit: NUM_CTX,
    events: [],
    continues: undefined,
  };
}

function fromSummary(summary: TaskSummary, workspace: string): Task {
  return {
    ...taskShell(summary.id, summary.title, summary.model, workspace),
    status: summary.status,
    updated: relativeTime(summary.updatedAt),
    continues: summary.continues ?? undefined,
  };
}

function relativeTime(iso: string): string {
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return "";
  const minutes = Math.round((Date.now() - at) / 60_000);
  if (minutes < 1) return "agora";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h`;
  const days = Math.round(hours / 24);
  return days === 1 ? "ontem" : `${days} dias`;
}

function applyLive(tasks: Task[], message: AgentEventMessage, workspace: string): Task[] {
  const index = tasks.findIndex((task) => task.id === message.taskId);
  if (index >= 0) {
    const next = [...tasks];
    next[index] = { ...applyAgentEvent(next[index], message), updated: "agora" };
    return next;
  }
  const base =
    message.event === "taskStarted"
      ? fromSummary(message.data.summary, workspace)
      : taskShell(message.taskId, "Nova tarefa", "", workspace);
  return [applyAgentEvent(base, message), ...tasks];
}
