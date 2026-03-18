use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Managed state shared across Tauri commands.
pub struct HubState {
    pub port: u16,
    pub start_time: Instant,
    pub http: reqwest::Client,
}

impl HubState {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            start_time: Instant::now(),
            http: reqwest::Client::new(),
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

pub type SharedHubState = Arc<Mutex<HubState>>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStatus {
    pub running: bool,
    pub port: u16,
    pub version: String,
    pub build_hash: String,
    pub uptime_secs: u64,
    pub active_agents: usize,
}

/// Return current hub status including uptime and active agent count.
#[tauri::command]
pub async fn get_hub_status(
    state: tauri::State<'_, SharedHubState>,
) -> Result<HubStatus, String> {
    let hub = state.lock().await;
    let uptime = hub.start_time.elapsed().as_secs();
    let port = hub.port;

    // Query agent count from the HTTP API
    let agent_count = match hub
        .http
        .get(format!("{}/api/agents", hub.base_url()))
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                body.get("agents")
                    .and_then(|a| a.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0)
            } else {
                0
            }
        }
        Err(_) => 0,
    };

    Ok(HubStatus {
        running: true,
        port,
        version: hex_hub_core::version().to_string(),
        build_hash: hex_hub_core::build_hash().to_string(),
        uptime_secs: uptime,
        active_agents: agent_count,
    })
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

/// POST /api/agents/spawn — spawn a new hex-agent process via the hub API.
#[tauri::command]
pub async fn spawn_agent(
    state: tauri::State<'_, SharedHubState>,
    definition: String,
    project_path: String,
) -> Result<serde_json::Value, String> {
    let hub = state.lock().await;

    let body = serde_json::json!({
        "projectDir": project_path,
        "agentName": definition,
    });

    let resp = hub
        .http
        .post(format!("{}/api/agents/spawn", hub.base_url()))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if status.is_success() {
        Ok(json)
    } else {
        Err(json
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// DELETE /api/agents/{id} — terminate an agent.
#[tauri::command]
pub async fn kill_agent(
    state: tauri::State<'_, SharedHubState>,
    agent_id: String,
) -> Result<serde_json::Value, String> {
    let hub = state.lock().await;

    let resp = hub
        .http
        .delete(format!("{}/api/agents/{}", hub.base_url(), agent_id))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if status.is_success() {
        Ok(json)
    } else {
        Err(json
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// GET /api/agents — list all tracked agents.
#[tauri::command]
pub async fn list_agents(
    state: tauri::State<'_, SharedHubState>,
) -> Result<serde_json::Value, String> {
    let hub = state.lock().await;

    let resp = hub
        .http
        .get(format!("{}/api/agents", hub.base_url()))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if status.is_success() {
        Ok(json)
    } else {
        Err(json
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("Unknown error")
            .to_string())
    }
}
