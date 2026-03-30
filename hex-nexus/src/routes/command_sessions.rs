//! Command session proxy routes.
//!
//! These routes act as a visibility/proxy layer for hex-agent's CommandSessionAdapter.
//! Actual execution happens in hex-agent (step-2). Nexus proxies the request or
//! returns 503 when no running hex-agent is available.
//!
//! POST /api/command-sessions        — run a batch of commands and index output
//! GET  /api/command-sessions/{id}/search — search indexed output by query

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::SharedState;

// ── Request / Response types ──────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub commands: Vec<String>,
    pub working_dir: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub commands_run: usize,
    pub total_lines: usize,
    pub exit_codes: Vec<i32>,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub max_results: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub command: String,
    pub line_number: usize,
    pub line: String,
    pub score: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
}

// ── Handlers ──────────────────────────────────────────────

/// POST /api/command-sessions
///
/// Accepts a list of shell commands, forwards to hex-agent for execution and
/// context indexing. Returns 503 stub until hex-agent forwarding is wired in
/// step-5 (composition root).
pub async fn create_session(
    State(_state): State<SharedState>,
    Json(_body): Json<CreateSessionRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "command sessions require a running hex-agent"
        })),
    )
}

/// GET /api/command-sessions/{session_id}/search
///
/// Searches indexed command output for a session. Returns 503 stub until
/// hex-agent forwarding is wired in step-5 (composition root).
pub async fn search_session(
    State(_state): State<SharedState>,
    Path(_session_id): Path<String>,
    Query(_params): Query<SearchParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "command sessions require a running hex-agent"
        })),
    )
}
