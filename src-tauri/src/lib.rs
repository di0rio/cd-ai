#[tauri::command]
fn app_info() -> agent_core::AppInfo {
    agent_core::app_info()
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
