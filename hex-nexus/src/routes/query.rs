use axum::{
    extract::{Path, State},
    Json,
};
use http::StatusCode;
use serde_json::json;

use crate::state::SharedState;

pub async fn get_health(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };
    match sp.project_get(&project_id).await {
        Ok(Some(entry)) => {
            let data = entry.health.unwrap_or_else(|| {
                json!({
                    "summary": {
                        "healthScore": 0,
                        "totalFiles": 0,
                        "totalExports": 0,
                        "deadExportCount": 0,
                        "violationCount": 0,
                        "circularCount": 0
                    }
                })
            });
            (StatusCode::OK, Json(data))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

pub async fn get_tokens_overview(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };
    match sp.project_get(&project_id).await {
        Ok(Some(entry)) => {
            let data = entry.tokens.unwrap_or(json!({ "files": [] }));
            (StatusCode::OK, Json(data))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

pub async fn get_token_file(
    State(state): State<SharedState>,
    Path((project_id, file)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };
    match sp.project_get(&project_id).await {
        Ok(Some(entry)) => {
            let decoded = urlencoding::decode(&file).unwrap_or_default().into_owned();
            // Reject path traversal attempts, absolute paths, and null bytes
            if decoded.contains("..") || decoded.starts_with('/') || decoded.contains('\0') {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Invalid file path" })));
            }
            match entry.token_files.get(&decoded) {
                Some(data) => (StatusCode::OK, Json(data.clone())),
                None => (StatusCode::NOT_FOUND, Json(json!({ "error": "File not found" }))),
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

pub async fn get_swarm(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };
    match sp.project_get(&project_id).await {
        Ok(Some(entry)) => {
            let data = entry.swarm.unwrap_or(json!({
                "status": { "status": "idle", "agentCount": 0, "activeTaskCount": 0, "completedTaskCount": 0 },
                "tasks": [],
                "agents": []
            }));
            (StatusCode::OK, Json(data))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

pub async fn get_graph(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };
    match sp.project_get(&project_id).await {
        Ok(Some(entry)) => {
            let data = entry.graph.unwrap_or(json!({ "nodes": [], "edges": [] }));
            (StatusCode::OK, Json(data))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

pub async fn get_project(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let sp = match state.require_state_port() {
        Ok(sp) => sp.clone(),
        Err(e) => return e,
    };
    match sp.project_get(&project_id).await {
        Ok(Some(entry)) => {
            let data = json!({
                "rootPath": entry.root_path,
                "name": entry.name,
                "astIsStub": entry.ast_is_stub,
            });
            (StatusCode::OK, Json(data))
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}
