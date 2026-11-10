#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod host;

/// Runs off the main thread, and the polling gets a blocking thread of its own, so the window
/// never freezes while a host starts.
#[tauri::command]
async fn host_connection() -> Result<host::HostConnection, String> {
    tauri::async_runtime::spawn_blocking(host::find_or_start)
        .await
        .map_err(|error| format!("The search for the host stopped: {error}"))?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        // Saves the window's size and place on exit and puts them back at the next start.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![host_connection])
        .run(tauri::generate_context!())
        .expect("tauri failed to run");
}
