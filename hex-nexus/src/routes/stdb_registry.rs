//! SpacetimeDB database registry endpoint
//! 
//! Returns current database identities for agent-comms, chat-relay, etc.
//! so clients don't need hardcoded identities.

use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};
use crate::state::SharedState;

/// GET /api/stdb/registry — returns SpacetimeDB database identities
pub async fn get_registry(
    State(_state): State<SharedState>,
) -> (StatusCode, Json<Value>) {
    let registry = json!({
        "agent_comms": "c200a65681232ad58e2bc33eefb64d8ff72804348c58f2ca074733b53b266ed4",
        "chat_relay": "chat-relay",
        "hex": "hex",
        "inference_gateway": "inference-gateway",
    });

    (StatusCode::OK, Json(registry))
}
