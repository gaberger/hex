use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::sync::Arc;

use crate::remote::ssh::SshConfig;
use crate::state::AppState;

/// Build fleet management routes.
pub fn fleet_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/fleet", get(list_nodes))
        .route("/api/fleet/register", post(register_node))
        .route("/api/fleet/{id}", get(get_node).delete(unregister_node))
        .route("/api/fleet/{id}/deploy", post(deploy_to_node))
        .route("/api/fleet/health", post(check_health))
        .route("/api/fleet/select", get(select_best_node))
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    id: String,
    host: String,
    port: Option<u16>,
    username: String,
    key_path: String,
    max_agents: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct DeployRequest {
    binary_path: String,
    install_dir: Option<String>,
}

pub async fn list_nodes(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let nodes = state.fleet.list().await;
    Json(serde_json::json!({
        "ok": true,
        "nodes": nodes,
        "count": nodes.len()
    }))
}

pub async fn register_node(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let config = SshConfig {
        host: req.host,
        port: req.port.unwrap_or(22),
        username: req.username,
        key_path: req.key_path,
    };

    state
        .fleet
        .register(req.id.clone(), config, req.max_agents.unwrap_or(4))
        .await;

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "ok": true,
            "node_id": req.id
        })),
    )
}

pub async fn get_node(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.fleet.get(&id).await {
        Some(node) => Ok(Json(serde_json::json!({ "ok": true, "node": node }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

pub async fn unregister_node(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let removed = state.fleet.unregister(&id).await;
    Json(serde_json::json!({ "ok": removed }))
}

pub async fn deploy_to_node(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<DeployRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let node = state.fleet.get(&id).await.ok_or((
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "ok": false, "error": "Node not found" })),
    ))?;

    let install_dir = req.install_dir.unwrap_or_else(|| "/usr/local/bin".into());

    match crate::remote::deployer::Deployer::deploy_full(
        &node.config,
        &req.binary_path,
        &install_dir,
    )
    .await
    {
        Ok(result) => Ok(Json(serde_json::json!({ "ok": true, "result": result }))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        )),
    }
}

pub async fn check_health(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    let results = state.fleet.check_all_health().await;
    Json(serde_json::json!({
        "ok": true,
        "results": results.iter().map(|(id, ok)| {
            serde_json::json!({ "id": id, "healthy": ok })
        }).collect::<Vec<_>>()
    }))
}

pub async fn select_best_node(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.fleet.select_node().await {
        Some(node) => Ok(Json(serde_json::json!({ "ok": true, "node": node }))),
        None => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}
