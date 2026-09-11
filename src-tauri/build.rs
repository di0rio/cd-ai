fn main() {
    // Every app command must be listed here and allowed in capabilities/default.json;
    // anything else is unreachable from the webview.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_info",
            "open_workspace",
            "current_workspace",
            "ollama_status",
            "chat",
            "cancel_chat",
        ]),
    ))
    .expect("failed to run tauri-build");
}
