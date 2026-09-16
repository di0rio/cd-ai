use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use agent_core::RollbackResult;
use agent_core::agent::{
    AgentEvent, AgentEventMessage, AgentLimits, OllamaModel, Settings, SettingsStore, StopReason,
    TaskContext, TaskHistoryEntry, TaskReport, TaskStart, TaskStatus, TaskStore, TaskSummary,
    run_task, workspace_key,
};
use agent_core::ollama::{ChatEvent, ChatRequest, OllamaClient};
use agent_core::permissions::{ApprovalRequest, ApprovalResponse, PermissionMode};
use agent_core::sandbox::{self, SandboxStatus};
use agent_core::tools::cancel::CancelToken;
use agent_core::workspace::Workspace;
use tauri::ipc::Channel;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    workspace: Mutex<Option<Workspace>>,
    /// `Err` only when the app data directory could not be opened at boot; the task commands then
    /// answer with that reason instead of pretending there is no history.
    store: Result<TaskStore, String>,
    /// Same directory, same failure: without it preferences simply do not persist, which costs the
    /// user a click and never a task.
    settings: Result<SettingsStore, String>,
}

/// Course corrections typed while a task runs; the loop drains it each iteration.
type SteerQueue = Arc<Mutex<VecDeque<String>>>;

/// One task running in this process: what the UI needs to steer it, cancel it or answer it.
struct RunningTask {
    cancel: CancelToken,
    steer: SteerQueue,
    /// Approvals the loop thread is blocked on, by approval id (D4).
    pending: HashMap<String, mpsc::Sender<ApprovalResponse>>,
}

#[derive(Default)]
struct TaskRegistry {
    /// Set the moment a start is accepted and cleared when its thread ends, so a second start is
    /// refused even before the loop has minted an id (D5).
    busy: bool,
    tasks: HashMap<String, RunningTask>,
}

#[derive(Clone, Default)]
struct RunningTasks(Arc<Mutex<TaskRegistry>>);

impl RunningTasks {
    fn lock(&self) -> Result<MutexGuard<'_, TaskRegistry>, String> {
        self.0
            .lock()
            .map_err(|_| "estado das tarefas corrompido".to_string())
    }

    /// Takes the single task slot (D5) and mints the handles the loop will run with.
    fn claim(&self) -> Result<(CancelToken, SteerQueue), String> {
        let mut registry = self.lock()?;
        if registry.busy {
            return Err("já existe uma tarefa em andamento".to_string());
        }
        registry.busy = true;
        Ok((CancelToken::default(), Arc::default()))
    }

    /// Frees the slot when the start never got off the ground.
    fn release_claim(&self) {
        if let Ok(mut registry) = self.0.lock() {
            registry.busy = false;
        }
    }

    fn register(&self, task_id: &str, cancel: CancelToken, steer: SteerQueue) {
        if let Ok(mut registry) = self.0.lock() {
            registry.tasks.insert(
                task_id.to_string(),
                RunningTask {
                    cancel,
                    steer,
                    pending: HashMap::new(),
                },
            );
        }
    }

    /// Drops the task and frees the slot. Any sender left behind dies with it, and a responder
    /// still blocked on one reads the closed channel as a denial.
    ///
    /// Called only from `TaskSlot::drop`, so the slot is freed exactly once no matter how the loop
    /// thread ended.
    fn finish(&self, task_id: &str) {
        if let Ok(mut registry) = self.0.lock() {
            registry.tasks.remove(task_id);
            registry.busy = false;
        }
    }

    /// Parks an approval under `(task_id, approval_id)` and hands back the end the loop waits on.
    ///
    /// A cancelled task parks nothing: `cancel` has already drained what was pending, so an
    /// approval asked in the gap between the engine's cancel check and this call would never be
    /// answered. Both sides take this lock, so either order ends in a denial, never in a wait.
    fn park_approval(&self, request: &ApprovalRequest) -> Option<mpsc::Receiver<ApprovalResponse>> {
        let mut registry = self.0.lock().ok()?;
        let task = registry.tasks.get_mut(&request.task_id)?;
        if task.cancel.is_cancelled() {
            return None;
        }
        let (tx, rx) = mpsc::channel();
        task.pending.insert(request.id.clone(), tx);
        Some(rx)
    }

    fn answer(&self, task_id: &str, approval_id: &str, response: ApprovalResponse) -> bool {
        let Ok(mut registry) = self.0.lock() else {
            return false;
        };
        let Some(task) = registry.tasks.get_mut(task_id) else {
            return false;
        };
        match task.pending.remove(approval_id) {
            Some(tx) => tx.send(response).is_ok(),
            None => false,
        }
    }

    /// Cancels the loop and denies every approval it is waiting on: without the denial the loop
    /// thread would sit in the responder forever (D6).
    fn cancel(&self, task_id: &str) -> bool {
        let Ok(mut registry) = self.0.lock() else {
            return false;
        };
        let Some(task) = registry.tasks.get_mut(task_id) else {
            return false;
        };
        task.cancel.cancel();
        for (_, tx) in task.pending.drain() {
            let _ = tx.send(ApprovalResponse::Denied {
                reason: Some("tarefa cancelada".to_string()),
            });
        }
        true
    }

    fn steer(&self, task_id: &str, text: String) -> bool {
        let Ok(registry) = self.0.lock() else {
            return false;
        };
        let Some(task) = registry.tasks.get(task_id) else {
            return false;
        };
        match task.steer.lock() {
            Ok(mut queue) => {
                queue.push_back(text);
                true
            }
            Err(_) => false,
        }
    }
}

/// Frees the task slot when the loop thread ends, however it ends.
///
/// `Drop` runs while unwinding too, so a panic inside `run_task` can no longer leave `busy` stuck
/// at `true` with the task still registered — the app would then refuse every later start until it
/// was restarted, with nothing in the UI to say why. The guard is the only path that clears the
/// slot: the thread body never calls `finish` itself, so the two can never disagree.
struct TaskSlot {
    registry: RunningTasks,
    on_event: Channel<AgentEventMessage>,
    /// Known only once the loop mints the id; `None` while the task has no identity yet.
    task_id: Option<String>,
    /// Last sequence seen, so an event emitted from `Drop` continues the stream.
    last_sequence: u64,
    /// Set when `run_task` returned: the loop emitted its own `taskFinished`, so the guard must not
    /// emit another one.
    finished: bool,
}

impl TaskSlot {
    fn new(registry: RunningTasks, on_event: Channel<AgentEventMessage>) -> Self {
        Self {
            registry,
            on_event,
            task_id: None,
            last_sequence: 0,
            finished: false,
        }
    }

    fn observe(&mut self, message: &AgentEventMessage) {
        if self.task_id.is_none() {
            self.task_id = Some(message.task_id.clone());
        }
        self.last_sequence = message.sequence;
    }

    /// The loop came back on its own: from here a `Drop` only has to free the slot.
    fn settle(&mut self, task_id: &str) {
        self.task_id = Some(task_id.to_string());
        self.finished = true;
    }
}

impl Drop for TaskSlot {
    fn drop(&mut self) {
        let Some(task_id) = self.task_id.clone() else {
            // Nothing was ever registered: only the claim taken before the id existed.
            self.registry.release_claim();
            return;
        };
        if !self.finished {
            // The loop thread died mid-task. Without this the UI would wait forever for a
            // `taskFinished` that the panicked thread never sent. `Interrupted` is exactly what the
            // stored state will report on the next open (D9), since the panic skipped every write
            // to disk — this event reaches the UI only.
            let _ = self.on_event.send(AgentEventMessage::new(
                task_id.clone(),
                self.last_sequence.saturating_add(1),
                AgentEvent::TaskFinished {
                    status: TaskStatus::Failed,
                    stop_reason: StopReason::Interrupted,
                    report: TaskReport {
                        summary: "a tarefa terminou de forma inesperada".to_string(),
                        validated: false,
                        evidence: Vec::new(),
                        files_changed: Vec::new(),
                    },
                },
            ));
        }
        self.registry.finish(&task_id);
    }
}

#[derive(Clone, Default)]
struct ChatTasks(Arc<Mutex<HashMap<u64, tauri::async_runtime::JoinHandle<()>>>>);

#[derive(Default)]
struct NextChatId(AtomicU64);

/// The open workspace, cloned: a `MutexGuard` cannot be held across an `await`.
fn open_workspace_of(state: &AppState) -> Result<Workspace, String> {
    state
        .workspace
        .lock()
        .map_err(|_| "estado do workspace corrompido".to_string())?
        .clone()
        .ok_or_else(|| "nenhum workspace aberto".to_string())
}

#[tauri::command]
fn app_info() -> agent_core::AppInfo {
    agent_core::app_info()
}

#[tauri::command]
async fn open_workspace(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<agent_core::workspace::WorkspaceInfo>, String> {
    // Runs off the main thread (async command), where the blocking dialog is allowed.
    let Some(folder) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None); // user cancelled
    };
    let path = folder.into_path().map_err(|error| error.to_string())?;
    let workspace = Workspace::open(path).map_err(|error| error.to_string())?;
    let info = workspace.info();
    *state
        .workspace
        .lock()
        .map_err(|_| "estado do workspace corrompido".to_string())? = Some(workspace);
    Ok(Some(info))
}

#[tauri::command]
fn current_workspace(
    state: tauri::State<'_, AppState>,
) -> Option<agent_core::workspace::WorkspaceInfo> {
    let workspace = state.workspace.lock().ok()?;
    workspace.as_ref().map(|workspace| workspace.info())
}

/// Starts the loop for `start` on a dedicated thread and answers with the task id.
///
/// The loop is blocking by design (D1) and `OllamaModel` reaches the network through
/// `Handle::block_on`, which panics from inside a runtime: it must never run in an async task.
/// `run_task` also mints the task id itself, so the thread registers the task on its first event
/// and hands the id back here — registering from the thread guarantees the slot exists before any
/// tool can ask for approval.
async fn spawn_task(
    state: tauri::State<'_, AppState>,
    tasks: tauri::State<'_, RunningTasks>,
    start: TaskStart,
    model_name: String,
    num_ctx: u32,
    on_event: Channel<AgentEventMessage>,
) -> Result<String, String> {
    let workspace = open_workspace_of(&state)?;
    let store = state.store.clone()?;
    let permission_mode = state
        .settings
        .as_ref()
        .ok()
        .map(|settings| settings.load().permission_mode)
        .unwrap_or_default();
    let client = OllamaClient::new(&build_ollama_base())?;
    let runtime = tauri::async_runtime::handle().inner().clone();
    let limits = AgentLimits::default();
    let turn_timeout = Duration::from_millis(limits.model_turn_timeout_ms);

    let (cancel, steer) = tasks.claim()?;
    let registry = tasks.inner().clone();
    let (id_tx, id_rx) = mpsc::channel::<String>();

    let spawned = std::thread::Builder::new()
        .name("cd-ai-task".to_string())
        .spawn(move || {
            // First thing in the body: from here on every exit, including a panic, frees the slot.
            let mut slot = TaskSlot::new(registry.clone(), on_event.clone());
            let mut model = OllamaModel::new(client, runtime, model_name, num_ctx, turn_timeout);
            let mut ctx = TaskContext::new(&store, workspace);
            ctx.limits = limits;
            ctx.cancel = cancel.clone();
            ctx.steer = steer.clone();
            ctx.permission_mode = permission_mode;

            let mut responder = |request: ApprovalRequest| match registry.park_approval(&request) {
                // `respond_approval` answers through the channel. A closed channel means the task
                // is gone: an unanswerable approval is a denial, never a deadlock.
                Some(rx) => rx
                    .recv()
                    .unwrap_or(ApprovalResponse::Denied { reason: None }),
                None => ApprovalResponse::Denied { reason: None },
            };

            let final_state = {
                let mut sink = |message: AgentEventMessage| {
                    if slot.task_id.is_none() {
                        registry.register(&message.task_id, cancel.clone(), steer.clone());
                        let _ = id_tx.send(message.task_id.clone());
                    }
                    slot.observe(&message);
                    let _ = on_event.send(message);
                };
                run_task(ctx, &mut model, start, &mut sink, &mut responder)
            };
            // A task that could not even start emits nothing; the caller still gets its id.
            if slot.task_id.is_none() {
                let _ = id_tx.send(final_state.id.clone());
            }
            slot.settle(&final_state.id);
        });
    if let Err(error) = spawned {
        tasks.release_claim();
        return Err(format!("não foi possível iniciar a tarefa: {error}"));
    }

    // Waits off the UI thread for the id the loop minted; its first event comes right away.
    tauri::async_runtime::spawn_blocking(move || id_rx.recv())
        .await
        .map_err(|error| error.to_string())?
        .map_err(|_| "a tarefa terminou antes de começar".to_string())
}

#[tauri::command]
async fn start_task(
    request: String,
    model: String,
    num_ctx: u32,
    // Task this one continues: it inherits that task's report, never its conversation.
    continues: Option<String>,
    on_event: Channel<AgentEventMessage>,
    state: tauri::State<'_, AppState>,
    tasks: tauri::State<'_, RunningTasks>,
) -> Result<String, String> {
    // Checked here for the same reason a resume is: `run_task` refuses a wrong id too, but as a
    // failed task with no events, and the user would only see a row that never ran.
    if let Some(previous_id) = &continues {
        task_of_workspace(&state, previous_id, "continuá-la")?;
    }
    let start = TaskStart::New {
        request,
        model: model.clone(),
        num_ctx,
        continues,
    };
    spawn_task(state, tasks, start, model, num_ctx, on_event).await
}

#[tauri::command]
async fn resume_task(
    task_id: String,
    model: String,
    num_ctx: u32,
    on_event: Channel<AgentEventMessage>,
    state: tauri::State<'_, AppState>,
    tasks: tauri::State<'_, RunningTasks>,
) -> Result<String, String> {
    // Checked here so a wrong id fails with its own message: `run_task` would accept it and come
    // back `failed` with an imprecise reason, which stays only as a safety net.
    task_of_workspace(&state, &task_id, "retomá-la")?;
    let start = TaskStart::Resume { task_id };
    spawn_task(state, tasks, start, model, num_ctx, on_event).await
}

/// Loads a task of the open workspace, refusing an id from anywhere else. The workspace boundary
/// is decided here, in Rust: the frontend only ever names an id.
///
/// `what` completes "abra aquela pasta para …", so the refusal says what was being attempted.
fn task_of_workspace(
    state: &tauri::State<'_, AppState>,
    task_id: &str,
    what: &str,
) -> Result<agent_core::agent::TaskState, String> {
    let key = workspace_key(&open_workspace_of(state)?);
    let existing = state
        .store
        .clone()?
        .load_state(task_id)
        .map_err(|error| error.to_string())?;
    if existing.workspace != key {
        return Err(format!(
            "a tarefa pertence a outro workspace ({}); abra aquela pasta para {what}",
            existing.workspace
        ));
    }
    Ok(existing)
}

#[tauri::command]
fn cancel_task(task_id: String, tasks: tauri::State<'_, RunningTasks>) -> bool {
    tasks.cancel(&task_id)
}

#[tauri::command]
fn steer_task(task_id: String, text: String, tasks: tauri::State<'_, RunningTasks>) -> bool {
    tasks.steer(&task_id, text)
}

#[tauri::command]
fn respond_approval(
    task_id: String,
    id: String,
    granted: bool,
    reason: Option<String>,
    tasks: tauri::State<'_, RunningTasks>,
) -> bool {
    let response = if granted {
        ApprovalResponse::Granted
    } else {
        ApprovalResponse::Denied { reason }
    };
    tasks.answer(&task_id, &id, response)
}

#[tauri::command]
async fn list_tasks(state: tauri::State<'_, AppState>) -> Result<Vec<TaskSummary>, String> {
    let key = workspace_key(&open_workspace_of(&state)?);
    let store = state.store.clone()?;
    // One `state.json` per task: off the main thread, so a long history never janks the UI.
    tauri::async_runtime::spawn_blocking(move || {
        store.list(&key).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn workspace_history(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<TaskHistoryEntry>, String> {
    let key = workspace_key(&open_workspace_of(&state)?);
    let store = state.store.clone()?;
    tauri::async_runtime::spawn_blocking(move || {
        store.history(&key).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn rollback_task(
    task_id: String,
    force: bool,
    state: tauri::State<'_, AppState>,
) -> Result<RollbackResult, String> {
    let workspace = open_workspace_of(&state)?;
    task_of_workspace(&state, &task_id, "revertê-la")?;
    let store = state.store.clone()?;
    tauri::async_runtime::spawn_blocking(move || {
        agent_core::rollback_task(&store, &workspace, &task_id, force)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn task_events(
    task_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentEventMessage>, String> {
    // The workspace boundary belongs to the Rust side, not to the frontend: an id from another
    // workspace is refused here the same way `resume_task` refuses it, so `task_events` cannot be
    // used to read a task the open workspace does not own.
    let key = workspace_key(&open_workspace_of(&state)?);
    let store = state.store.clone()?;
    tauri::async_runtime::spawn_blocking(move || {
        let existing = store
            .load_state(&task_id)
            .map_err(|error| error.to_string())?;
        if existing.workspace != key {
            return Err(format!(
                "a tarefa pertence a outro workspace ({}); abra aquela pasta para ver seus eventos",
                existing.workspace
            ));
        }
        store
            .load_events(&task_id)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

/// What the app remembered from the last runs. Reading never fails on content: an unreadable file
/// answers as "nothing chosen yet", so the UI always has a value to fall back from.
#[tauri::command]
async fn get_settings(state: tauri::State<'_, AppState>) -> Result<Settings, String> {
    let store = state.settings.clone()?;
    tauri::async_runtime::spawn_blocking(move || store.load())
        .await
        .map_err(|error| error.to_string())
}

/// Remembers the model the user picked. The choice is the user's, so it goes to disk as it came;
/// nothing here reads meaning into the name (decision 0002, rule 5).
///
/// Writing is a convenience and answers `Ok` even when it failed: the reason is logged, and a full
/// or read-only disk must not turn picking a model into an error in the middle of a task.
#[tauri::command]
async fn set_preferred_model(
    model: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let store = match state.settings.clone() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("cd-ai: não foi possível guardar o modelo escolhido: {error}");
            return Ok(());
        }
    };
    // Read then write: a field this version does not know about survives the update.
    let written = tauri::async_runtime::spawn_blocking(move || {
        let mut settings = store.load();
        settings.model = Some(model);
        store.save(&settings)
    })
    .await;
    match written {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("cd-ai: não foi possível guardar o modelo escolhido: {error}"),
        Err(error) => eprintln!("cd-ai: não foi possível guardar o modelo escolhido: {error}"),
    }
    Ok(())
}

#[tauri::command]
fn sandbox_status() -> SandboxStatus {
    sandbox::status().clone()
}

/// Persists ASK / AUTO / FULL ACCESS. FULL ACCESS is refused when the OS sandbox is not ready;
/// the webview cannot enable it on its own.
#[tauri::command]
async fn set_permission_mode(
    mode: PermissionMode,
    state: tauri::State<'_, AppState>,
) -> Result<PermissionMode, String> {
    if mode == PermissionMode::FullAccess && !sandbox::status().available {
        return Err(format!(
            "Acesso total exige sandbox ativo ({})",
            sandbox::status().detail
        ));
    }
    let store = match state.settings.clone() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("cd-ai: não foi possível guardar o modo de permissão: {error}");
            return Ok(mode);
        }
    };
    let written = tauri::async_runtime::spawn_blocking(move || {
        let mut settings = store.load();
        settings.permission_mode = mode;
        store.save(&settings).map(|()| mode)
    })
    .await
    .map_err(|error| error.to_string())?;
    written.map_err(|error| error.to_string())
}

#[tauri::command]
async fn ollama_status() -> agent_core::ollama::OllamaStatus {
    match agent_core::ollama::OllamaClient::new(&build_ollama_base()) {
        Ok(client) => client.status().await,
        Err(error) => agent_core::ollama::OllamaStatus {
            reachable: false,
            version: None,
            models: vec![],
            loaded: vec![],
            error: Some(error),
        },
    }
}

fn build_ollama_base() -> String {
    let raw = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| agent_core::ollama::DEFAULT_BASE_URL.to_string());
    if raw.starts_with("http") {
        raw
    } else {
        format!("http://{raw}")
    }
}

#[tauri::command]
fn chat(
    request: ChatRequest,
    on_event: Channel<ChatEvent>,
    tasks: tauri::State<'_, ChatTasks>,
    next_chat_id: tauri::State<'_, NextChatId>,
) -> Result<u64, String> {
    let base = build_ollama_base();
    let client = OllamaClient::new(&base).map_err(|error| error.to_string())?;
    let id = next_chat_id.0.fetch_add(1, Ordering::Relaxed);

    // Clone the Arc so the spawned task can remove its own entry when it finishes.
    let tasks_for_stream = tasks.inner().clone();

    let handle = tauri::async_runtime::spawn(async move {
        let result = client
            .chat_stream(&request, |event| {
                let _ = on_event.send(event);
            })
            .await;
        if let Err(message) = result {
            let _ = on_event.send(ChatEvent::Error { message });
        }
        if let Ok(mut entries) = tasks_for_stream.0.lock() {
            entries.remove(&id);
        }
    });

    let mut entries = tasks
        .0
        .lock()
        .map_err(|_| "estado do chat corrompido".to_string())?;
    entries.insert(id, handle);
    Ok(id)
}

#[tauri::command]
fn cancel_chat(id: u64, tasks: tauri::State<'_, ChatTasks>) -> bool {
    let mut entries = match tasks.0.lock() {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    match entries.remove(&id) {
        Some(handle) => {
            handle.abort();
            true
        }
        None => false,
    }
}

pub fn run() {
    let store = match TaskStore::open_default() {
        Ok(store) => {
            // A task still marked running means the app died on it, so it becomes resumable
            // before the UI can list anything (D9).
            if let Err(error) = store.recover_interrupted() {
                eprintln!("cd-ai: não foi possível recuperar tarefas interrompidas: {error}");
            }
            Ok(store)
        }
        Err(error) => Err(error.to_string()),
    };
    let settings = SettingsStore::open_default().map_err(|error| error.to_string());

    tauri::Builder::default()
        .manage(AppState {
            workspace: Default::default(),
            store,
            settings,
        })
        .manage(RunningTasks::default())
        .manage(ChatTasks::default())
        .manage(NextChatId::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_workspace,
            current_workspace,
            start_task,
            resume_task,
            cancel_task,
            steer_task,
            respond_approval,
            list_tasks,
            workspace_history,
            rollback_task,
            task_events,
            get_settings,
            set_preferred_model,
            sandbox_status,
            set_permission_mode,
            ollama_status,
            chat,
            cancel_chat
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::InvokeResponseBody;

    /// A channel that keeps every serialized message, standing in for the webview.
    fn recording_channel() -> (Channel<AgentEventMessage>, Arc<Mutex<Vec<String>>>) {
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = seen.clone();
        let channel = Channel::new(move |body: InvokeResponseBody| {
            let text = match body {
                InvokeResponseBody::Json(json) => json,
                InvokeResponseBody::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            };
            sink.lock().expect("canal de teste").push(text);
            Ok(())
        });
        (channel, seen)
    }

    fn token_event(task_id: &str, sequence: u64) -> AgentEventMessage {
        AgentEventMessage::new(
            task_id,
            sequence,
            AgentEvent::Token {
                content: "oi".to_string(),
            },
        )
    }

    #[test]
    fn guard_frees_the_slot_and_warns_the_ui_when_the_loop_thread_panics() {
        let tasks = RunningTasks::default();
        let (channel, seen) = recording_channel();
        let (cancel, steer) = tasks.claim().expect("a vaga começa livre");

        let registry = tasks.clone();
        let panicked = std::thread::spawn(move || {
            let mut slot = TaskSlot::new(registry.clone(), channel);
            registry.register("task_1", cancel, steer);
            slot.observe(&token_event("task_1", 7));
            panic!("o loop morreu");
        })
        .join();
        assert!(panicked.is_err());

        assert!(tasks.lock().unwrap().tasks.is_empty());
        assert!(!tasks.lock().unwrap().busy);
        tasks.claim().expect("a vaga volta a ficar livre");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "o pânico emite um taskFinished: {seen:?}");
        assert!(
            seen[0].contains("\"event\":\"taskFinished\""),
            "{}",
            seen[0]
        );
        assert!(seen[0].contains("\"status\":\"failed\""), "{}", seen[0]);
        assert!(seen[0].contains("\"sequence\":8"), "{}", seen[0]);
    }

    #[test]
    fn guard_stays_quiet_when_the_loop_finishes_on_its_own() {
        let tasks = RunningTasks::default();
        let (channel, seen) = recording_channel();
        let (cancel, steer) = tasks.claim().expect("a vaga começa livre");

        {
            let mut slot = TaskSlot::new(tasks.clone(), channel);
            tasks.register("task_1", cancel, steer);
            slot.observe(&token_event("task_1", 1));
            slot.settle("task_1");
        }

        assert!(tasks.lock().unwrap().tasks.is_empty());
        assert!(!tasks.lock().unwrap().busy);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn guard_releases_the_claim_when_no_id_was_ever_minted() {
        let tasks = RunningTasks::default();
        let (channel, seen) = recording_channel();
        let (_cancel, _steer) = tasks.claim().expect("a vaga começa livre");

        drop(TaskSlot::new(tasks.clone(), channel));

        assert!(!tasks.lock().unwrap().busy);
        assert!(seen.lock().unwrap().is_empty());
    }
}
