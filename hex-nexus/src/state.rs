use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, OnceLock};
use tokio::sync::{broadcast, RwLock};

use crate::coordination::HexFlo;
use crate::orchestration::agent_manager::AgentManager;
use crate::orchestration::workplan_executor::WorkplanExecutor;
use crate::ports::session::ISessionPort;
use crate::ports::state::IStatePort;
use crate::remote::fleet::FleetManager;
use crate::adapters::spacetime_chat::SpacetimeChatClient;
use crate::adapters::spacetime_inference::SpacetimeInferenceClient;
use crate::adapters::spacetime_secrets::SpacetimeSecretClient;
// ── App State ───────────────────────────────────────────

pub type SharedState = Arc<AppState>;

pub const MAX_ACTIVITIES: usize = 500;

pub struct AppState {
    // Ephemeral command dispatch (NOT persistent state — keep per ADR-042)
    pub commands: RwLock<HashMap<String, HubCommand>>,       // commandId → command
    pub results: RwLock<HashMap<String, HubCommandResult>>,  // commandId → result
    // Ephemeral activity stream (bounded ring buffer, not persistent)
    pub activities: RwLock<VecDeque<ActivityEntry>>,
    // WebSocket broadcast channel (ephemeral)
    pub ws_tx: broadcast::Sender<WsEnvelope>,
    pub auth_token: Option<String>,
    pub fleet: FleetManager,
    pub anthropic_api_key: Option<String>,
    // Port-backed orchestration services (ADR-025 Phase 2)
    pub agent_manager: Option<Arc<AgentManager>>,
    pub workplan_executor: OnceLock<Arc<WorkplanExecutor>>,
    // Secret broker state (ADR-026) — SpacetimeDB only, no in-memory fallback
    pub spacetime_secrets: Option<Arc<SpacetimeSecretClient>>,
    // HexFlo coordination (ADR-027)
    pub hexflo: Option<Arc<HexFlo>>,
    // Unified state port (ADR-025 + ADR-042) — single source of truth for all persistent state
    pub state_port: Option<Arc<dyn IStatePort>>,
    // SpacetimeDB inference-gateway client (ADR-035)
    pub inference_stdb: Option<Arc<SpacetimeInferenceClient>>,
    // SpacetimeDB chat-relay client
    pub chat_stdb: Option<Arc<SpacetimeChatClient>>,
    // Session persistence (ADR-036 / ADR-042 P2.5) — chat conversation history
    // SpacetimeDB primary, SQLite fallback
    pub session_port: Option<Arc<dyn ISessionPort>>,
}

impl AppState {
    pub fn new(auth_token: Option<String>) -> Self {
        let (ws_tx, _) = broadcast::channel(512);
        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").ok();
        if anthropic_api_key.is_some() {
            tracing::info!("ANTHROPIC_API_KEY loaded — chat LLM bridge enabled");
        } else {
            tracing::warn!("ANTHROPIC_API_KEY not set — chat will relay only (no direct LLM)");
        }
        Self {
            commands: RwLock::new(HashMap::new()),
            results: RwLock::new(HashMap::new()),
            activities: RwLock::new(VecDeque::new()),
            ws_tx,
            auth_token,
            fleet: FleetManager::new(),
            anthropic_api_key,
            agent_manager: None,
            workplan_executor: OnceLock::new(),
            spacetime_secrets: None,
            hexflo: None,
            state_port: None,
            inference_stdb: None,
            chat_stdb: None,
            session_port: None,
        }
    }

    /// Helper: get a reference to the state port or return an error response.
    pub fn require_state_port(&self) -> Result<&Arc<dyn IStatePort>, (http::StatusCode, axum::Json<serde_json::Value>)> {
        self.state_port.as_ref().ok_or_else(|| {
            (
                http::StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({ "error": "State port not configured" })),
            )
        })
    }
}

// ── Project Entry ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub registered_at: i64,
    pub last_push_at: i64,
    pub state: ProjectState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    pub health: Option<serde_json::Value>,
    pub tokens: Option<serde_json::Value>,
    #[serde(default)]
    pub token_files: HashMap<String, serde_json::Value>,
    pub swarm: Option<serde_json::Value>,
    pub graph: Option<serde_json::Value>,
    pub project: Option<ProjectMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
    pub root_path: String,
    pub name: String,
    #[serde(default)]
    pub ast_is_stub: bool,
}

// ── WebSocket Envelope ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEnvelope {
    pub topic: String,
    pub event: String,
    pub data: serde_json::Value,
}

// ── Command Types (Hub → Project) ───────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCommand {
    pub command_id: String,
    pub project_id: String,
    #[serde(rename = "type")]
    pub command_type: String,
    pub payload: serde_json::Value,
    pub issued_at: String,
    pub source: String,
    pub status: String,  // pending, dispatched, running, completed, failed
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubCommandResult {
    pub command_id: String,
    pub status: String,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
    pub completed_at: String,
}

// ── Request/Response Types ──────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub project_id: String,
    #[serde(rename = "type")]
    pub push_type: String,
    pub data: Option<serde_json::Value>,
    pub file_path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRequest {
    pub project_id: String,
    pub event: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequest {
    pub selected_option: String,
}


// ── Coordination Types ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceInfo {
    pub instance_id: String,
    pub project_id: String,
    pub pid: u32,
    pub session_label: String,
    pub registered_at: String,
    pub last_seen: String,
    pub agent_count: Option<u32>,
    pub active_task_count: Option<u32>,
    pub completed_task_count: Option<u32>,
    pub topology: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterInstanceRequest {
    pub project_id: String,
    pub pid: u32,
    pub session_label: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeartbeatRequest {
    pub instance_id: String,
    pub project_id: String,
    pub unstaged_files: Option<Vec<UnstagedFile>>,
    pub agent_count: Option<u32>,
    pub active_task_count: Option<u32>,
    pub completed_task_count: Option<u32>,
    pub topology: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeLock {
    pub instance_id: String,
    pub project_id: String,
    pub feature: String,
    pub layer: String,
    pub acquired_at: String,
    pub heartbeat_at: String,
    pub ttl_secs: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockRequest {
    pub instance_id: String,
    pub project_id: String,
    pub feature: String,
    pub layer: String,
    pub ttl_secs: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskClaim {
    pub task_id: String,
    pub instance_id: String,
    pub claimed_at: String,
    pub heartbeat_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // Fields read via Deserialize, not direct access
pub struct TaskClaimRequest {
    pub instance_id: String,
    pub project_id: String,
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub instance_id: String,
    pub project_id: String,
    pub action: String,
    pub details: serde_json::Value,
    pub timestamp: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRequest {
    pub instance_id: String,
    pub project_id: String,
    pub action: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnstagedFile {
    pub path: String,
    pub status: String,
    pub layer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnstagedState {
    pub instance_id: String,
    pub project_id: String,
    pub files: Vec<UnstagedFile>,
    pub captured_at: String,
}

// ── Project ID (must match TypeScript implementation) ───

/// Deterministic project ID: basename + DJB2 hash in base-36.
/// Must produce identical output to the TypeScript `makeProjectId`.
pub fn make_project_id(root_path: &str) -> String {
    let basename = root_path.rsplit('/').next().unwrap_or("unknown");
    let hash = root_path
        .chars()
        .fold(0u32, |h, c| {
            (h.wrapping_shl(5)).wrapping_sub(h).wrapping_add(c as u32)
        });
    format!("{}-{}", basename, radix_36(hash))
}

fn radix_36(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let chars: Vec<char> = "0123456789abcdefghijklmnopqrstuvwxyz".chars().collect();
    let mut result = Vec::new();
    while n > 0 {
        result.push(chars[(n % 36) as usize]);
        n /= 36;
    }
    result.into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_is_deterministic() {
        let id1 = make_project_id("/Users/gary/projects/my-app");
        let id2 = make_project_id("/Users/gary/projects/my-app");
        assert_eq!(id1, id2);
    }

    #[test]
    fn project_id_uses_basename() {
        let id = make_project_id("/Users/gary/projects/my-app");
        assert!(id.starts_with("my-app-"));
    }

    #[test]
    fn project_id_different_paths_different_ids() {
        let id1 = make_project_id("/a/my-app");
        let id2 = make_project_id("/b/my-app");
        assert_ne!(id1, id2);
    }

    /// Cross-language compatibility: these values were computed by the
    /// TypeScript makeProjectId() in dashboard-hub.ts. If any assertion
    /// fails, the TS DashboardAdapter and Rust hex-hub will disagree
    /// on project IDs, breaking registration.
    #[test]
    fn project_id_matches_typescript_implementation() {
        let vectors = vec![
            ("/Users/gary/projects/my-app", "my-app-1v7n98d"),
            ("/tmp/test", "test-14nsdrt"),
            ("/a/b/c/d/e", "e-1cqbqw4"),
            ("/Users/gary/hex-intf", "hex-intf-1x2ydj5"),
            ("/", "-1b"),
            ("/single", "single-zng5yv"),
        ];
        for (path, expected) in vectors {
            assert_eq!(
                make_project_id(path), expected,
                "DJB2 hash mismatch for path '{}': Rust produced '{}', TypeScript expects '{}'",
                path, make_project_id(path), expected
            );
        }
    }
}
