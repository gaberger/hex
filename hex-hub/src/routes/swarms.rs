use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::persistence::{CreateSwarmRequest, UpdateTaskRequest};
use crate::state::SharedState;

/// POST /api/swarms — create a new swarm
pub async fn create_swarm(
    State(state): State<SharedState>,
    Json(body): Json<CreateSwarmRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let db = state.swarm_db.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Swarm persistence not available" })),
        )
    })?;

    match db.create_swarm(&body).await {
        Ok(swarm) => Ok((StatusCode::CREATED, Json(serde_json::to_value(swarm).unwrap()))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error: {}", e) })),
        )),
    }
}

/// GET /api/swarms/active — list all non-completed swarms
pub async fn list_active_swarms(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.swarm_db.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Swarm persistence not available" })),
        )
    })?;

    match db.list_active_swarms().await {
        Ok(swarms) => Ok(Json(serde_json::to_value(swarms).unwrap())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error: {}", e) })),
        )),
    }
}

/// GET /api/swarms/:id — get swarm with tasks and agents
pub async fn get_swarm(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.swarm_db.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Swarm persistence not available" })),
        )
    })?;

    match db.get_swarm(&id).await {
        Ok(Some(detail)) => Ok(Json(serde_json::to_value(detail).unwrap())),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Swarm not found" })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error: {}", e) })),
        )),
    }
}

/// PATCH /api/swarms/:id/tasks/:taskId — update task status/result
pub async fn update_task(
    State(state): State<SharedState>,
    Path((_swarm_id, task_id)): Path<(String, String)>,
    Json(body): Json<UpdateTaskRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.swarm_db.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Swarm persistence not available" })),
        )
    })?;

    match db.update_task(&task_id, &body).await {
        Ok(true) => Ok(Json(json!({ "ok": true }))),
        Ok(false) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Task not found or no fields to update" })),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error: {}", e) })),
        )),
    }
}

/// GET /api/work-items/incomplete — all in-flight work across all swarms
pub async fn get_incomplete_work(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.swarm_db.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "Swarm persistence not available" })),
        )
    })?;

    match db.get_incomplete_work().await {
        Ok(items) => Ok(Json(serde_json::to_value(items).unwrap())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Database error: {}", e) })),
        )),
    }
}
