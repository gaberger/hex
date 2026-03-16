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
    let projects = state.projects.read().await;
    match projects.get(&project_id) {
        Some(entry) => {
            let data = entry.state.health.clone().unwrap_or_else(|| {
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
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
    }
}

pub async fn get_tokens_overview(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let projects = state.projects.read().await;
    match projects.get(&project_id) {
        Some(entry) => {
            let data = entry.state.tokens.clone().unwrap_or(json!({ "files": [] }));
            (StatusCode::OK, Json(data))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
    }
}

pub async fn get_token_file(
    State(state): State<SharedState>,
    Path((project_id, file)): Path<(String, String)>,
) -> (StatusCode, Json<serde_json::Value>) {
    let projects = state.projects.read().await;
    match projects.get(&project_id) {
        Some(entry) => {
            let decoded = urlencoding::decode(&file).unwrap_or_default().into_owned();
            match entry.state.token_files.get(&decoded) {
                Some(data) => (StatusCode::OK, Json(data.clone())),
                None => (StatusCode::NOT_FOUND, Json(json!({ "error": "File not found" }))),
            }
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
    }
}

pub async fn get_swarm(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let projects = state.projects.read().await;
    match projects.get(&project_id) {
        Some(entry) => {
            let data = entry.state.swarm.clone().unwrap_or(json!({
                "status": { "status": "idle", "agentCount": 0, "activeTaskCount": 0, "completedTaskCount": 0 },
                "tasks": [],
                "agents": []
            }));
            (StatusCode::OK, Json(data))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
    }
}

pub async fn get_graph(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let projects = state.projects.read().await;
    match projects.get(&project_id) {
        Some(entry) => {
            let data = entry.state.graph.clone().unwrap_or(json!({ "nodes": [], "edges": [] }));
            (StatusCode::OK, Json(data))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
    }
}

pub async fn get_project(
    State(state): State<SharedState>,
    Path(project_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let projects = state.projects.read().await;
    match projects.get(&project_id) {
        Some(entry) => {
            let data = match &entry.state.project {
                Some(meta) => json!({
                    "rootPath": meta.root_path,
                    "name": meta.name,
                    "astIsStub": meta.ast_is_stub
                }),
                None => json!({
                    "rootPath": entry.root_path,
                    "name": entry.name
                }),
            };
            (StatusCode::OK, Json(data))
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not found" }))),
    }
}
