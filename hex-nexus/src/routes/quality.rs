use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ports::state::IStatePort;
use crate::state::SharedState;

// ── Helpers (same pattern as swarms.rs) ─────────────────

fn state_port(
    state: &SharedState,
) -> Result<&dyn IStatePort, (StatusCode, Json<Value>)> {
    state
        .state_port
        .as_deref()
        .ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "State port not available" })),
            )
        })
}

fn state_err(e: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": format!("{}", e) })),
    )
}

// ── Request types ───────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateQualityGateRequest {
    pub swarm_id: String,
    pub tier: u32,
    pub gate_type: String,
    pub target_dir: String,
    pub language: String,
    #[serde(default = "default_iteration")]
    pub iteration: u32,
}

fn default_iteration() -> u32 {
    1
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteQualityGateRequest {
    pub status: String,
    #[serde(default)]
    pub score: u32,
    #[serde(default)]
    pub grade: String,
    #[serde(default)]
    pub violations_count: u32,
    #[serde(default)]
    pub error_output: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityGateQuery {
    pub swarm_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFixTaskRequest {
    pub gate_task_id: String,
    pub swarm_id: String,
    pub fix_type: String,
    #[serde(default)]
    pub target_file: String,
    #[serde(default)]
    pub error_context: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteFixTaskRequest {
    pub status: String,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub model_used: String,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub cost_usd: String,
}

// ── Handlers ────────────────────────────────────────────

/// POST /api/hexflo/quality-gate — create a quality gate task
pub async fn create_quality_gate(
    State(state): State<SharedState>,
    Json(body): Json<CreateQualityGateRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let id = uuid::Uuid::new_v4().to_string();

    port.quality_gate_create(
        &id,
        &body.swarm_id,
        body.tier,
        &body.gate_type,
        &body.target_dir,
        &body.language,
        body.iteration,
    )
    .await
    .map_err(state_err)?;

    let val = json!({
        "id": id,
        "status": "pending",
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "quality_gate_created".into(),
        data: json!({
            "id": id,
            "swarmId": body.swarm_id,
            "tier": body.tier,
            "gateType": body.gate_type,
        }),
    });

    Ok((StatusCode::CREATED, Json(val)))
}

/// PATCH /api/hexflo/quality-gate/:id — update gate result
pub async fn complete_quality_gate(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<CompleteQualityGateRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    port.quality_gate_complete(
        &id,
        &body.status,
        body.score,
        &body.grade,
        body.violations_count,
        &body.error_output,
    )
    .await
    .map_err(state_err)?;

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "quality_gate_completed".into(),
        data: json!({
            "id": id,
            "status": body.status,
            "score": body.score,
            "grade": body.grade,
        }),
    });

    Ok(Json(json!({
        "ok": true,
        "id": id,
        "status": body.status,
        "score": body.score,
        "grade": body.grade,
        "violationsCount": body.violations_count,
    })))
}

/// GET /api/hexflo/quality-gate?swarm_id=X — list gates for a swarm
pub async fn list_quality_gates(
    State(state): State<SharedState>,
    Query(params): Query<QualityGateQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let gates = port
        .quality_gate_list(&params.swarm_id)
        .await
        .map_err(state_err)?;

    Ok(Json(serde_json::to_value(gates).unwrap()))
}

/// POST /api/hexflo/fix-task — create a fix task
pub async fn create_fix_task(
    State(state): State<SharedState>,
    Json(body): Json<CreateFixTaskRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let id = uuid::Uuid::new_v4().to_string();

    port.fix_task_create(
        &id,
        &body.gate_task_id,
        &body.swarm_id,
        &body.fix_type,
        &body.target_file,
        &body.error_context,
    )
    .await
    .map_err(state_err)?;

    let val = json!({
        "id": id,
        "status": "pending",
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "fix_task_created".into(),
        data: json!({
            "id": id,
            "gateTaskId": body.gate_task_id,
            "fixType": body.fix_type,
        }),
    });

    Ok((StatusCode::CREATED, Json(val)))
}

/// PATCH /api/hexflo/fix-task/:id — update fix result
pub async fn complete_fix_task(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<CompleteFixTaskRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    port.fix_task_complete(
        &id,
        &body.status,
        &body.result,
        &body.model_used,
        body.tokens,
        &body.cost_usd,
    )
    .await
    .map_err(state_err)?;

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "hexflo".into(),
        event: "fix_task_completed".into(),
        data: json!({
            "id": id,
            "status": body.status,
            "result": body.result,
        }),
    });

    Ok(Json(json!({
        "ok": true,
        "id": id,
        "status": body.status,
        "result": body.result,
    })))
}

/// GET /api/hexflo/quality-gate/:id/fixes — list fixes for a gate
pub async fn list_fixes_for_gate(
    State(state): State<SharedState>,
    Path(gate_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let fixes = port
        .fix_task_list_by_gate(&gate_id)
        .await
        .map_err(state_err)?;

    Ok(Json(serde_json::to_value(fixes).unwrap()))
}
