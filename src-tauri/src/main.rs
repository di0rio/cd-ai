// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before anything else: on Windows this process may be the sandbox launcher.
    agent_core::sandbox::init();
    cd_ai_desktop_lib::run();
}
