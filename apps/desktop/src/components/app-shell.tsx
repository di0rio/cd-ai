"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { applyAgentEvent } from "@/lib/activity";
import { demoRunningId, demoTasks } from "@/lib/demo-session";
import {
  type AgentEventMessage,
  cancelTask,
  currentWorkspace,
  getOllamaStatus,
  getSandboxStatus,
  getSettings,
  listTasks,
  openWorkspace,
  type PermissionMode,
  respondApproval,
  resumeTask,
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
import { type PanelKind, SidePanel } from "./side-panel";
import { Sidebar } from "./sidebar";
import { TooltipProvider } from "./ui/tooltip";

// Decision 0002, rule 3: every task the UI starts gets the same context window.
const NUM_CTX = 16_384;

export function AppShell() {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [panel, setPanel] = useState<PanelKind | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [demo, setDemo] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [loadedModels, setLoadedModels] = useState<string[]>([]);
  const [chosenModel, setChosenModel] = useState<string | null>(null);
  // The choice the core remembered from the last runs; only decides the default model.
  const [preferredModel, setPreferred] = useState<string | null>(null);
  const [permissionMode, setPermission] = useState<PermissionMode>("ask");
  const [sandbox, setSandbox] = useState<SandboxStatus | null>(null);
  const [runningId, setRunningId] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [taskError, setTaskError] = useState<string | null>(null);

  // Tasks whose conversation is already in memory, so a replay from disk never runs twice.
  const replayed = useRef(new Set<string>());
  // Read by the event callback, which outlives the render that created it.
  const workspaceName = useRef("");
  const composerInput = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    // Invented sessions only in development with ?demo; production shows real state only, and a
    // demonstration touches no IPC at all — hence the early return.
    if (process.env.NODE_ENV !== "production" && new URLSearchParams(window.location.search).has("demo")) {
      setTasks(demoTasks);
      setSelectedId(demoTasks[0].id);
      setRunningId(demoRunningId);
      setDemo(true);
      for (const task of demoTasks) replayed.current.add(task.id);
      return;
    }
    // Outside Tauri the call rejects and that is expected; the app then opens with no preference.
    getSettings().then(
      (settings) => {
        setPreferred(settings.model);
        setPermission(settings.permissionMode);
      },
      () => {},
    );
    getSandboxStatus().then(setSandbox, () => {});
  }, []);

  useEffect(() => {
    // Outside Tauri the call rejects and that is expected.
    currentWorkspace().then(setWorkspace, () => {});
  }, []);

  useEffect(() => {
    workspaceName.current = workspace?.name ?? "";
  }, [workspace]);

  // The task history of the open workspace, newest first, as the core has it on disk.
  useEffect(() => {
    if (demo || !workspace) return;
    let cancelled = false;
    listTasks().then(
      (summaries) => {
        if (cancelled) return;
        setTasks((previous) => {
          const live = new Map(previous.map((task) => [task.id, task]));
          return summaries.map((summary) => live.get(summary.id) ?? fromSummary(summary, workspace.name));
        });
      },
      () => {},
    );
    return () => {
      cancelled = true;
    };
  }, [demo, workspace]);

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      getOllamaStatus().then(
        (status) => {
          if (cancelled) return;
          setModels(status.models.map((model) => model.name));
          setLoadedModels(status.loaded.map((model) => model.name));
        },
        () => {},
      );
    load();
    window.addEventListener("focus", load);
    return () => {
      cancelled = true;
      window.removeEventListener("focus", load);
    };
  }, []);

  // Opening an older task rebuilds its conversation from the stored events, through the same
  // reducer the live stream uses.
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
    setTasks((previous) => applyLive(previous, message, workspaceName.current));
    if (message.event === "taskStarted") {
      replayed.current.add(message.taskId);
      setRunningId(message.taskId);
      setSelectedId(message.taskId);
    } else if (message.event === "taskFinished") {
      setRunningId((current) => (current === message.taskId ? null : current));
    }
  }, []);

  // There is no core behind a demonstration: the controls stay on screen to be looked at, and do nothing.
  const ignore = () => {};
  const task = tasks.find((candidate) => candidate.id === selectedId) ?? null;
  const model = chosenModel ?? defaultModel(preferredModel, loadedModels, models);
  const running = starting || runningId !== null;
  const togglePanel = (kind: PanelKind) => setPanel((current) => (current === kind ? null : kind));

  const [zoomLevel, setZoomLevel] = useState(1);
  const zoomIn = useCallback(() => setZoomLevel((z) => Math.min(z + 0.1, 2)), []);
  const zoomOut = useCallback(() => setZoomLevel((z) => Math.max(z - 0.1, 0.5)), []);
  const zoomReset = useCallback(() => setZoomLevel(1), []);

  useEffect(() => {
    document.documentElement.style.zoom = `${zoomLevel}`;
  }, [zoomLevel]);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "b") {
        event.preventDefault();
        setSidebarOpen((open) => !open);
      } else if (event.key === "Escape") {
        setPanel(null);
      } else if ((event.ctrlKey || event.metaKey) && (event.key === "=" || event.key === "+")) {
        event.preventDefault();
        zoomIn();
      } else if ((event.ctrlKey || event.metaKey) && event.key === "-") {
        event.preventDefault();
        zoomOut();
      } else if ((event.ctrlKey || event.metaKey) && event.key === "0") {
        event.preventDefault();
        zoomReset();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [zoomIn, zoomOut, zoomReset]);

  // The core remembers the choice for the next runs. It answers even when it could not write, and a
  // rejection here only means there is no core (browser): either way the choice holds on screen.
  const handleModelChange = (next: string) => {
    setChosenModel(next);
    setPreferredModel(next).catch(() => {});
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
      const selectedTask = tasks.find((t) => t.id === selectedId);
      const continues =
        selectedTask &&
        selectedTask.workspace === workspace?.name &&
        selectedTask.status !== "running" &&
        selectedTask.status !== "waiting_approval"
          ? selectedTask.id
          : undefined;
      setRunningId(await startTask(text, model, NUM_CTX, handleEvent, continues));
    } catch (error) {
      setTaskError(String(error));
    } finally {
      setStarting(false);
    }
  };

  // The commands answer `false` when the task is no longer in the registry: say so instead of
  // letting the correction or the cancellation look accepted.
  const handleSteer = async (text: string) => {
    if (!runningId) return;
    // A correction always goes to the one task in flight, so bring it back on screen first.
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

  // The answer only carries the decision; what it authorises is decided and enforced in Rust.
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
    // Long enough that the pointer has to rest on a control, short enough not to feel like the
    // half-second lag of the native Windows tooltip this replaced.
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
                onApprovalDecision={demo ? undefined : handleApproval}
                // One task per workspace (D5): no offer to resume while another one holds the slot.
                onResume={demo ? ignore : running ? undefined : handleResume}
              />
            ) : (
              <EmptyWorkspace workspace={workspace} error={workspaceError} onOpen={handleOpenWorkspace} />
            )}
            {(workspace || task) && (
              <Composer
                task={task}
                running={running}
                // A demonstration has a workspace in its own data, so the composer shows its normal state.
                workspaceOpen={demo || Boolean(workspace)}
                models={models}
                loadedModels={loadedModels}
                model={model}
                permissionMode={permissionMode}
                sandboxAvailable={demo || Boolean(sandbox?.available)}
                sandboxDetail={sandbox?.detail ?? "sandbox só existe no Linux nesta versão"}
                onPermissionModeChange={handlePermissionMode}
                onModelChange={demo ? setChosenModel : handleModelChange}
                onStart={demo ? ignore : handleStart}
                onSteer={demo ? ignore : handleSteer}
                onCancel={demo ? ignore : handleCancel}
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

// A task the core knows about but whose conversation has not been loaded yet.
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

// The core stores an instant; the sidebar shows how long ago it was.
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

// A task the live stream has never seen is built from its own first event, so nothing is lost
// between `start_task` accepting the request and resolving with the id.
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
