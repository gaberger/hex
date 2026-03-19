use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ports::session::{MessagePart, NewMessage, Role, TokenUsage};
use crate::state::SharedState;

// ── Request Types ───────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub project_id: String,
    pub model: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTitleRequest {
    pub title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendMessageRequest {
    pub role: String,
    pub parts: Vec<MessagePart>,
    pub model: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkRequest {
    pub at_sequence: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactRequest {
    pub summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertRequest {
    pub to_sequence: u32,
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub project_id: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MessageListParams {
    pub limit: Option<u32>,
    pub before: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub project_id: Option<String>,
    pub q: Option<String>,
    pub limit: Option<u32>,
}

// ── Helper ──────────────────────────────────────────────

macro_rules! require_session_port {
    ($state:expr) => {
        match &$state.session_port {
            Some(port) => port.clone(),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "session persistence not enabled (sqlite-session feature)" })),
                );
            }
        }
    };
}

// ── Handlers ────────────────────────────────────────────

/// POST /api/sessions
pub async fn create_session(
    State(state): State<SharedState>,
    Json(req): Json<CreateSessionRequest>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    let model = req.model.as_deref().unwrap_or("claude-sonnet-4-20250514");
    match port.session_create(&req.project_id, model, req.title.as_deref()).await {
        Ok(session) => (StatusCode::CREATED, Json(serde_json::to_value(session).unwrap())),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// GET /api/sessions
pub async fn list_sessions(
    State(state): State<SharedState>,
    Query(params): Query<ListParams>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    let project_id = params.project_id.as_deref().unwrap_or("");
    let limit = params.limit.unwrap_or(20);
    let offset = params.offset.unwrap_or(0);
    match port.session_list(project_id, limit, offset).await {
        Ok(sessions) => (StatusCode::OK, Json(serde_json::to_value(sessions).unwrap())),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// GET /api/sessions/:id
pub async fn get_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    match port.session_get(&id).await {
        Ok(Some(session)) => (StatusCode::OK, Json(serde_json::to_value(session).unwrap())),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "session not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// PATCH /api/sessions/:id
pub async fn update_session_title(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTitleRequest>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    match port.session_update_title(&id, &req.title).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// DELETE /api/sessions/:id
pub async fn delete_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    match port.session_delete(&id).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// POST /api/sessions/:id/archive
pub async fn archive_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    match port.session_archive(&id).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// GET /api/sessions/:id/messages
pub async fn list_messages(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(params): Query<MessageListParams>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    let limit = params.limit.unwrap_or(100);
    match port.message_list(&id, limit, params.before).await {
        Ok(messages) => (StatusCode::OK, Json(serde_json::to_value(messages).unwrap())),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// POST /api/sessions/:id/messages
pub async fn append_message(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<AppendMessageRequest>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    let role: Role = req.role.parse().unwrap_or(Role::User);
    let token_usage = match (req.input_tokens, req.output_tokens) {
        (Some(i), Some(o)) => Some(TokenUsage { input_tokens: i, output_tokens: o }),
        _ => None,
    };
    let msg = NewMessage {
        role,
        parts: req.parts,
        model: req.model,
        token_usage,
    };
    match port.message_append(&id, msg).await {
        Ok(message) => (StatusCode::CREATED, Json(serde_json::to_value(message).unwrap())),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// POST /api/sessions/:id/fork
pub async fn fork_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<ForkRequest>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    match port.session_fork(&id, req.at_sequence).await {
        Ok(session) => (StatusCode::CREATED, Json(serde_json::to_value(session).unwrap())),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// POST /api/sessions/:id/compact
pub async fn compact_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<CompactRequest>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    match port.session_compact(&id, &req.summary).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// POST /api/sessions/:id/revert
pub async fn revert_session(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<RevertRequest>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    match port.session_revert(&id, req.to_sequence).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// GET /api/sessions/search
pub async fn search_sessions(
    State(state): State<SharedState>,
    Query(params): Query<SearchParams>,
) -> (StatusCode, Json<Value>) {
    let port = require_session_port!(state);
    let project_id = params.project_id.as_deref().unwrap_or("");
    let query = params.q.as_deref().unwrap_or("");
    let limit = params.limit.unwrap_or(10);
    match port.session_search(project_id, query, limit).await {
        Ok(results) => (StatusCode::OK, Json(serde_json::to_value(results).unwrap())),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}
