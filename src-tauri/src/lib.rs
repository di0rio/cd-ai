use tauri_plugin_dialog::DialogExt;

struct AppState {
    workspace: std::sync::Mutex<Option<agent_core::workspace::Workspace>>,
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

pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            workspace: Default::default(),
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            app_info,
            open_workspace,
            current_workspace
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
