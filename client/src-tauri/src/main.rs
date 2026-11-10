#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod host;
#[cfg(target_os = "macos")]
mod menu;

use std::path::PathBuf;

use serde_json::{Map, Value};
use tauri::Manager;

/// Runs off the main thread, and the polling gets a blocking thread of its own, so the window
/// never freezes while a host starts.
#[tauri::command]
async fn host_connection() -> Result<host::HostConnection, String> {
    tauri::async_runtime::spawn_blocking(host::find_or_start)
        .await
        .map_err(|error| format!("The search for the host stopped: {error}"))?
}

/// The renderer a speed run asked for through TA_RENDERER, `webgl` or `dom`. None leaves the
/// default.
#[tauri::command]
fn renderer() -> Option<String> {
    std::env::var("TA_RENDERER").ok()
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let folder = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Found no folder for the settings: {error}"))?;
    Ok(folder.join("settings.json"))
}

/// What settings.json holds. A missing file is no settings yet, and a broken one is treated the
/// same way, so the window still opens with the defaults.
fn read_settings(path: &std::path::Path) -> Result<Map<String, Value>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(match serde_json::from_slice(&bytes) {
            Ok(Value::Object(settings)) => settings,
            _ => Map::new(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

/// The saved settings. The window fills in a default for anything missing.
#[tauri::command]
fn settings_get(app: tauri::AppHandle) -> Result<Map<String, Value>, String> {
    read_settings(&settings_path(&app)?)
}

/// Merges `patch` into the saved settings. Written to a side file and renamed over the old one,
/// so a crash halfway never leaves half a file.
#[tauri::command]
fn settings_set(app: tauri::AppHandle, patch: Map<String, Value>) -> Result<(), String> {
    let path = settings_path(&app)?;
    let mut settings = read_settings(&path)?;
    settings.extend(patch);
    let text = serde_json::to_vec_pretty(&settings)
        .map_err(|error| format!("Could not write the settings: {error}"))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .map_err(|error| format!("Could not make {}: {error}", folder.display()))?;
    }
    let side = path.with_extension("json.tmp");
    std::fs::write(&side, text)
        .and_then(|()| std::fs::rename(&side, &path))
        .map_err(|error| format!("Could not save {}: {error}", path.display()))
}

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        // Saves the window's size and place on exit and puts them back at the next start.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            host_connection,
            renderer,
            settings_get,
            settings_set
        ]);
    // Close tab moves to Cmd+Shift+W so Cmd+W can close a pane.
    #[cfg(target_os = "macos")]
    let builder = builder.menu(menu::build).on_menu_event(menu::on_event);
    builder
        .run(tauri::generate_context!())
        .expect("tauri failed to run");
}
