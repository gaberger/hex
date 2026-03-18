use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// Managed state shared across Tauri commands.
pub struct HubState {
    pub port: u16,
    pub start_time: Instant,
    pub http: reqwest::Client,
    /// Override base URL for testing (when set, `base_url()` returns this instead).
    base_url_override: Option<String>,
}

impl HubState {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            start_time: Instant::now(),
            http: reqwest::Client::new(),
            base_url_override: None,
        }
    }

    /// Create a HubState pointing at an arbitrary URL (for mock servers in tests).
    pub fn with_base_url(url: &str) -> Self {
        Self {
            port: 0,
            start_time: Instant::now(),
            http: reqwest::Client::new(),
            base_url_override: Some(url.to_string()),
        }
    }

    pub fn base_url(&self) -> String {
        if let Some(ref url) = self.base_url_override {
            url.clone()
        } else {
            format!("http://127.0.0.1:{}", self.port)
        }
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
        version: hex_nexus::version().to_string(),
        build_hash: hex_nexus::build_hash().to_string(),
        uptime_secs: uptime,
        active_agents: agent_count,
    })
}

/// Return version info.
#[tauri::command]
pub fn get_hub_version() -> String {
    format!(
        "hex-desktop {} ({})",
        hex_nexus::version(),
        hex_nexus::build_hash()
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

/// Helper: POST JSON to a hub endpoint, return JSON or error string.
pub async fn hub_post(
    hub: &HubState,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let resp = hub
        .http
        .post(format!("{}{}", hub.base_url(), path))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    parse_hub_response(resp).await
}

/// Helper: DELETE to a hub endpoint, return JSON or error string.
pub async fn hub_delete(
    hub: &HubState,
    path: &str,
) -> Result<serde_json::Value, String> {
    let resp = hub
        .http
        .delete(format!("{}{}", hub.base_url(), path))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    parse_hub_response(resp).await
}

/// Helper: GET from a hub endpoint, return JSON or error string.
pub async fn hub_get(
    hub: &HubState,
    path: &str,
) -> Result<serde_json::Value, String> {
    let resp = hub
        .http
        .get(format!("{}{}", hub.base_url(), path))
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {}", e))?;

    parse_hub_response(resp).await
}

/// Parse an HTTP response into JSON, mapping non-2xx to Err with the error field.
async fn parse_hub_response(resp: reqwest::Response) -> Result<serde_json::Value, String> {
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
    hub_post(&hub, "/api/agents/spawn", &body).await
}

/// DELETE /api/agents/{id} — terminate an agent.
#[tauri::command]
pub async fn kill_agent(
    state: tauri::State<'_, SharedHubState>,
    agent_id: String,
) -> Result<serde_json::Value, String> {
    let hub = state.lock().await;
    hub_delete(&hub, &format!("/api/agents/{}", agent_id)).await
}

/// GET /api/agents — list all tracked agents.
#[tauri::command]
pub async fn list_agents(
    state: tauri::State<'_, SharedHubState>,
) -> Result<serde_json::Value, String> {
    let hub = state.lock().await;
    hub_get(&hub, "/api/agents").await
}
