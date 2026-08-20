//! POST /api/agent/run — invoke the simple agent loop.
//!
//! Body:
//!   {
//!     "intent": "<natural language operator intent>",
//!     "max_iterations": 10,              # optional, default 10
//!     "max_tokens": 4096,                # optional, default 4096
//!     "model": "qwen2.5-coder:14b"       # optional, default $HEX_AGENT_MODEL or qwen2.5-coder:14b
//!   }
//!
//! Returns RunSummary (see simple_agent.rs).
//!
//! This is the deliberately-flat alternative to `/api/org/send-message`
//! (which goes through persona rephrasing + atomic-claim + Confirm:
//! contract + drafter + twin). The agent loop here just calls the
//! local inference endpoint with the full typed-tool catalogue and
//! lets the LLM drive. Same safety gates downstream (twin auto-approve
//! for tool:* + operator-passthrough, executor cargo_check, autonomous
//! commit step).

use crate::orchestration::simple_agent::{run as simple_run, RunConfig};
use crate::tools::ToolRegistry;
use axum::{http::StatusCode, response::Json};
use serde_json::{json, Value};
use std::sync::Arc;

/// POST /api/agent/run
pub async fn run(Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let intent = match body.get("intent").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "missing or empty 'intent'" })),
            );
        }
    };
    let max_iterations = body
        .get("max_iterations")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);
    let max_tokens = body
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Same /api/inference/complete the SOP path uses. Co-located with nexus
    // (the daemon hosting this very route), so 127.0.0.1:5555 by default.
    let inference_url = std::env::var("HEX_AGENT_INFERENCE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:5555/api/inference/complete".to_string());

    let cfg = RunConfig {
        intent,
        max_iterations,
        max_tokens,
        model,
    };
    let registry = Arc::new(ToolRegistry::default());

    match simple_run(cfg, registry, inference_url).await {
        Ok(summary) => (
            StatusCode::OK,
            Json(serde_json::to_value(&summary).unwrap_or(Value::Null)),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        ),
    }
}
