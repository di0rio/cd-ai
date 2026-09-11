use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_core::ollama::{ChatEvent, ChatRequest, OllamaClient};
use tauri::ipc::Channel;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    workspace: Mutex<Option<agent_core::workspace::Workspace>>,
}

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
    state
        .workspace
        .lock()
        .ok()?
        .as_ref()
        .map(|workspace| workspace.info())
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
            workspace: Default::default(),
        })
        .manage(ChatTasks::default())
        .manage(NextChatId::default())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_workspace,
            current_workspace,
            ollama_status,
            chat,
            cancel_chat
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
