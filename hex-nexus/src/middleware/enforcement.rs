//! Enforcement middleware — provider-agnostic validation for hex-nexus REST API (ADR-2603221959 P4).
//!
//! Checks mutating requests (POST/PATCH/PUT/DELETE) against hex lifecycle rules.
//! GET requests and health endpoints are always allowed.
//! Uses the same `IEnforcementPort` trait as MCP tool guards.

use axum::{
    body::Body,
    extract::State,
    http::{Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use hex_core::domain::enforcement::DefaultEnforcer;
use hex_core::ports::enforcement::{EnforcementContext, EnforcementMode, EnforcementResult, IEnforcementPort};

use crate::state::SharedState;

/// Paths exempt from enforcement (health, version, auth, lifecycle bootstrap).
const EXEMPT_PATHS: &[&str] = &[
    "/api/health",
    "/api/version",
    "/api/openapi.json",
    "/api/docs",
    "/api/hex-agents/connect",     // Must allow agent registration
    "/api/hex-agents/evict",       // Cleanup
    "/api/stdb/",                  // SpacetimeDB admin
    "/api/config/sync",            // Config bootstrap
    "/ws",                         // WebSocket upgrade
    "/secrets/",                   // Secret broker (has own auth)
];

/// Enforcement middleware for hex-nexus REST API.
///
/// Checks mutating requests against hex lifecycle rules:
/// - Agent must be registered (X-Hex-Agent-Id header)
/// - Workplan should be active (X-Hex-Workplan-Id header)
/// - Task should be assigned (X-Hex-Task-Id header)
///
/// In mandatory mode, missing headers block the request (403).
/// In advisory mode, missing headers produce a warning header but allow the request.
pub async fn enforcement_layer(
    State(state): State<SharedState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    // GET requests are always allowed (read-only)
    if method == Method::GET || method == Method::OPTIONS {
        return next.run(request).await;
    }

    // Exempt paths are always allowed
    if EXEMPT_PATHS.iter().any(|p| path.starts_with(p)) {
        return next.run(request).await;
    }

    // Static assets are always allowed
    if path.starts_with("/assets/") || path == "/" {
        return next.run(request).await;
    }

    // Extract enforcement context from headers
    let agent_id = request
        .headers()
        .get("x-hex-agent-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let workplan_id = request
        .headers()
        .get("x-hex-workplan-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let task_id = request
        .headers()
        .get("x-hex-task-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // Determine enforcement mode from project config
    let mode = resolve_enforcement_mode();

    let ctx = EnforcementContext {
        agent_id,
        workplan_id,
        task_id,
        operation: format!("{} {}", method, path),
        ..Default::default()
    };

    let enforcer = DefaultEnforcer::new(mode);
    match enforcer.check(&ctx) {
        EnforcementResult::Block(reason) => {
            let _ = state; // consumed by State extractor
            (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": reason,
                    "enforcement": "mandatory",
                    "hint": "Register with hex_session_start, activate a workplan, then retry",
                })),
            )
                .into_response()
        }
        EnforcementResult::Warn(msg) => {
            let mut response = next.run(request).await;
            // Add warning header so clients can see it
            if let Ok(val) = http::HeaderValue::from_str(&msg) {
                response.headers_mut().insert("x-hex-enforcement-warning", val);
            }
            response
        }
        EnforcementResult::Allow => next.run(request).await,
    }
}

/// Read enforcement mode from .hex/project.json in cwd or HEX_PROJECT_DIR.
fn resolve_enforcement_mode() -> EnforcementMode {
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .or_else(|_| std::env::var("HEX_PROJECT_DIR"))
        .unwrap_or_else(|_| ".".to_string());
    let project_json = std::path::Path::new(&project_dir).join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(mode) = project["lifecycle_enforcement"].as_str() {
                return EnforcementMode::from_str(mode);
            }
        }
    }
    EnforcementMode::Mandatory
}
