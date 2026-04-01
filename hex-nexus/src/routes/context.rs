use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::state::SharedState;

/// POST /api/context/reload — clear template caches and signal agents to reload context.
/// Stores a reload timestamp in HexFlo global memory so agents polling for updates
/// know to invalidate their local caches.
pub async fn reload_context(State(state): State<SharedState>) -> Json<Value> {
    let timestamp = chrono::Utc::now().to_rfc3339();
    if let Some(ref sp) = state.state_port {
        let _ = sp
            .hexflo_memory_store("context:reload:timestamp", &timestamp, "global")
            .await;
    }
    Json(json!({ "status": "ok", "reloaded_at": timestamp }))
}
