//! Agent guard middleware — enforces that HexFlo/swarm endpoints are called
//! only by registered hex-agents.
//!
//! Checks for `X-Hex-Agent-Id` header and verifies the agent exists in the
//! state backend (SpacetimeDB or SQLite). Only applies to mutating requests
//! on guarded path prefixes (/api/swarms, /api/hexflo).

use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::state::SharedState;

/// Header name agents must send to identify themselves.
pub const AGENT_ID_HEADER: &str = "x-hex-agent-id";

/// Path prefixes that require agent identity for mutations.
const GUARDED_PREFIXES: &[&str] = &["/api/swarms", "/api/hexflo"];

pub async fn agent_guard(
    State(state): State<SharedState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Only guard mutating requests on hexflo/swarm paths
    let is_guarded = method != http::Method::GET
        && method != http::Method::OPTIONS
        && GUARDED_PREFIXES.iter().any(|prefix| path.starts_with(prefix));

    if !is_guarded {
        return next.run(req).await;
    }

    let agent_id = req
        .headers()
        .get(AGENT_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if agent_id.is_empty() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "HexFlo requires a registered hex-agent",
                "hint": "Include X-Hex-Agent-Id header from a registered session",
            })),
        )
            .into_response();
    }

    // Verify agent exists in state backend
    if let Some(sp) = state.state_port.as_ref() {
        match sp.agent_get(agent_id).await {
            Ok(Some(_)) => return next.run(req).await,
            Ok(None) => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(json!({
                        "error": format!("Agent '{}' is not registered", agent_id),
                        "hint": "Run `hex hook session-start` or start a Claude Code session in a hex project",
                    })),
                )
                    .into_response();
            }
            Err(e) => {
                tracing::warn!(agent_id = %agent_id, error = %e, "Agent lookup failed — allowing through");
                return next.run(req).await;
            }
        }
    }

    // No state_port available — can't verify, allow through
    next.run(req).await
}
