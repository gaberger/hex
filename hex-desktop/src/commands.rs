use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStatus {
    pub running: bool,
    pub port: u16,
    pub version: String,
    pub build_hash: String,
}

/// Return current hub status (called from JS via tauri::invoke).
#[tauri::command]
pub fn get_hub_status() -> HubStatus {
    HubStatus {
        running: true,
        port: hex_hub_core::DEFAULT_PORT,
        version: hex_hub_core::version().to_string(),
        build_hash: hex_hub_core::build_hash().to_string(),
    }
}

/// Return version info.
#[tauri::command]
pub fn get_hub_version() -> String {
    format!(
        "hex-desktop {} ({})",
        hex_hub_core::version(),
        hex_hub_core::build_hash()
    )
}

/// Open a project directory via native file dialog.
/// Returns the selected path or null if cancelled.
#[tauri::command]
pub async fn open_project(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let folder = app
        .dialog()
        .file()
        .blocking_pick_folder();

    Ok(folder.map(|p| p.to_string()))
}
