use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use agent_core::events::ToolEventMessage;
use agent_core::ollama::{ChatEvent, ChatRequest, OllamaClient};
use agent_core::permissions::{ApprovalRequest, ApprovalResponse};
use agent_core::tools::{ToolEngine, ToolOutcome, ToolRequest};
use tauri::ipc::Channel;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    // The engine owns the workspace (Fase 5 replaces it with one engine per task, SPEC §34).
    engine: Mutex<Option<ToolEngine>>,
}

/// Approval requests the engine is waiting on: the responder hands each one a channel
/// and blocks until `respond_approval` resolves it (design §5.3).
#[derive(Default)]
struct PendingApprovals(Arc<Mutex<HashMap<String, mpsc::Sender<ApprovalResponse>>>>);

#[derive(Clone, Default)]
struct ChatTasks(Arc<Mutex<HashMap<u64, tauri::async_runtime::JoinHandle<()>>>>);

#[derive(Default)]
struct NextChatId(AtomicU64);

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
    let workspace =
        agent_core::workspace::Workspace::open(path).map_err(|error| error.to_string())?;
    let info = workspace.info();
    // A workspace reset gets a fresh engine: approvals and command ids never survive a switch.
    *state
        .engine
        .lock()
        .map_err(|_| "estado do engine corrompido".to_string())? =
        Some(ToolEngine::new(workspace, "root"));
    Ok(Some(info))
}

#[tauri::command]
fn current_workspace(
    state: tauri::State<'_, AppState>,
) -> Option<agent_core::workspace::WorkspaceInfo> {
    let engine = state.engine.lock().ok()?;
    engine.as_ref().map(|engine| engine.workspace.info())
}

#[tauri::command]
async fn run_tool(
    state: tauri::State<'_, AppState>,
    pending: tauri::State<'_, PendingApprovals>,
    request: ToolRequest,
    on_event: tauri::ipc::Channel<ToolEventMessage>,
) -> Result<ToolOutcome, String> {
    let mut engine = state
        .engine
        .lock()
        .map_err(|_| "estado do engine corrompido".to_string())?
        .take()
        .ok_or_else(|| "nenhum workspace aberto".to_string())?;
    let pending_map = pending.0.clone();

    // Off the main thread (async command), so a blocked responder never freezes the UI.
    let (outcome, engine) = tauri::async_runtime::spawn_blocking(move || {
        let mut events = |message: ToolEventMessage| {
            let _ = on_event.send(message);
        };
        let mut responder = |request: ApprovalRequest| {
            let (tx, rx) = mpsc::channel();
            let id = request.id.clone();
            if let Ok(mut open) = pending_map.lock() {
                open.insert(id, tx);
            }
            match rx.recv() {
                // Dropped sender (respond_approval unknown id) falls back to denied.
                Ok(response) => response,
                Err(_) => ApprovalResponse::Denied { reason: None },
            }
        };
        let outcome = engine.run_tool(request, &mut events, &mut responder);
        (outcome, engine)
    })
    .await
    .map_err(|error| error.to_string())?;

    if let Ok(mut slot) = state.engine.lock() {
        *slot = Some(engine);
    }
    Ok(outcome)
}

#[tauri::command]
fn respond_approval(
    id: String,
    granted: bool,
    reason: Option<String>,
    pending: tauri::State<'_, PendingApprovals>,
) -> bool {
    let tx = match pending.0.lock().ok().and_then(|mut open| open.remove(&id)) {
        Some(tx) => tx,
        None => return false,
    };
    let response = if granted {
        ApprovalResponse::Granted
    } else {
        ApprovalResponse::Denied { reason }
    };
    tx.send(response).is_ok()
}

#[tauri::command]
async fn ollama_status() -> agent_core::ollama::OllamaStatus {
    let raw = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| agent_core::ollama::DEFAULT_BASE_URL.to_string());
    let base = if raw.starts_with("http") {
        raw
    } else {
        format!("http://{raw}")
    };
    match agent_core::ollama::OllamaClient::new(&base) {
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
    tauri::Builder::default()
        .manage(AppState {
            engine: Default::default(),
        })
        .manage(PendingApprovals::default())
        .manage(ChatTasks::default())
        .manage(NextChatId::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_workspace,
            current_workspace,
            run_tool,
            respond_approval,
            ollama_status,
            chat,
            cancel_chat
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
