use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

// ── App State ───────────────────────────────────────────

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub projects: RwLock<HashMap<String, ProjectEntry>>,
    pub sse_tx: broadcast::Sender<SseEvent>,
    pub ws_tx: broadcast::Sender<WsEnvelope>,
    pub auth_token: Option<String>,
}

impl AppState {
    pub fn new(auth_token: Option<String>) -> Self {
        let (sse_tx, _) = broadcast::channel(256);
        let (ws_tx, _) = broadcast::channel(256);
        Self {
            projects: RwLock::new(HashMap::new()),
            sse_tx,
            ws_tx,
            auth_token,
        }
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

// ── SSE Event ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SseEvent {
    pub project_id: Option<String>,
    pub event_type: String,
    pub data: serde_json::Value,
}

// ── WebSocket Envelope ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsEnvelope {
    pub topic: String,
    pub event: String,
    pub data: serde_json::Value,
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

#[derive(Debug, Deserialize)]
pub struct SseParams {
    pub project: Option<String>,
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
