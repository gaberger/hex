//! Neural Lab REST API endpoints.
//!
//! Bridges HTTP requests to the `neural-lab` SpacetimeDB WASM module
//! for neural architecture search: configs, experiments, frontiers,
//! and mutation strategies.

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

// ── Query parameter types ───────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ConfigListParams {
    pub status: Option<String>,
    pub lineage: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExperimentListParams {
    pub lineage: Option<String>,
    pub status: Option<String>,
}

// ── Request body types ──────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConfigRequest {
    pub name: String,
    #[serde(default)]
    pub parent_id: String,
    pub n_layer: u32,
    pub n_head: u32,
    #[serde(default)]
    pub n_kv_head: u32,
    pub n_embd: u32,
    pub vocab_size: u32,
    #[serde(default = "default_sequence_len")]
    pub sequence_len: u32,
    #[serde(default)]
    pub window_pattern: String,
    #[serde(default = "default_activation")]
    pub activation: String,
    #[serde(default = "default_optimizer_config")]
    pub optimizer_config: String,
    #[serde(default = "default_batch_size")]
    pub total_batch_size: u32,
    #[serde(default = "default_time_budget")]
    pub time_budget_secs: u32,
    #[serde(default)]
    pub created_by: String,
}

fn default_sequence_len() -> u32 { 1024 }
fn default_activation() -> String { "gelu".to_string() }
fn default_optimizer_config() -> String { "{}".to_string() }
fn default_batch_size() -> u32 { 524288 }
fn default_time_budget() -> u32 { 300 }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExperimentRequest {
    pub config_id: String,
    pub hypothesis: String,
    #[serde(default)]
    pub mutation_diff: String,
    pub lineage_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartExperimentRequest {
    pub gpu_node_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteExperimentRequest {
    pub val_bpb: String,
    pub train_loss_final: String,
    pub tokens_processed: u64,
    pub wall_time_secs: u32,
    #[serde(default)]
    pub git_commit: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailExperimentRequest {
    pub error_message: String,
}

// ── Config Handlers ─────────────────────────────────────

/// GET /api/neural-lab/configs — list NetworkConfigs.
/// Optional query params: ?status=candidate&lineage=main
pub async fn list_configs(
    State(state): State<SharedState>,
    Query(params): Query<ConfigListParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let configs = port
        .neural_lab_config_list(params.status.as_deref())
        .await
        .map_err(state_err)?;

    // Client-side lineage filter: configs whose id appears as parent_id
    // in experiments of the given lineage. For now, return all and let
    // the caller filter — the SpacetimeDB table has no lineage column
    // on configs (lineage lives on experiments).
    if let Some(ref _lineage) = params.lineage {
        // TODO: cross-table join via SQL or post-filter with experiment data
        let _ = _lineage; // suppress unused warning
    }

    Ok(Json(Value::Array(configs)))
}

/// GET /api/neural-lab/configs/:id — get single config with LayerSpecs.
pub async fn get_config(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let config = port
        .neural_lab_config_get(&id)
        .await
        .map_err(state_err)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Config '{}' not found", id) })),
            )
        })?;

    let layers = port
        .neural_lab_layer_specs(&id)
        .await
        .map_err(state_err)?;

    Ok(Json(json!({
        "config": config,
        "layers": layers,
    })))
}

/// POST /api/neural-lab/configs — create a new NetworkConfig.
/// Delegates to SpacetimeDB `config_create` reducer.
pub async fn create_config(
    State(state): State<SharedState>,
    Json(body): Json<CreateConfigRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let args = json!([
        body.name,
        body.parent_id,
        body.n_layer,
        body.n_head,
        body.n_kv_head,
        body.n_embd,
        body.vocab_size,
        body.sequence_len,
        body.window_pattern,
        body.activation,
        body.optimizer_config,
        body.total_batch_size,
        body.time_budget_secs,
        body.created_by,
    ]);

    port.neural_lab_config_create(args)
        .await
        .map_err(state_err)?;

    let val = json!({
        "ok": true,
        "name": body.name,
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "neural-lab".into(),
        event: "config_created".into(),
        data: val.clone(),
    });

    Ok((StatusCode::CREATED, Json(val)))
}

// ── Experiment Handlers ─────────────────────────────────

/// GET /api/neural-lab/experiments — list experiments.
/// Optional query params: ?lineage=main&status=training
pub async fn list_experiments(
    State(state): State<SharedState>,
    Query(params): Query<ExperimentListParams>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let experiments = port
        .neural_lab_experiment_list(params.lineage.as_deref(), params.status.as_deref())
        .await
        .map_err(state_err)?;

    Ok(Json(Value::Array(experiments)))
}

/// GET /api/neural-lab/experiments/:id — get single experiment.
pub async fn get_experiment(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let experiment = port
        .neural_lab_experiment_get(&id)
        .await
        .map_err(state_err)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Experiment '{}' not found", id) })),
            )
        })?;

    Ok(Json(experiment))
}

/// POST /api/neural-lab/experiments — create a new experiment.
/// Delegates to SpacetimeDB `experiment_create` reducer.
pub async fn create_experiment(
    State(state): State<SharedState>,
    Json(body): Json<CreateExperimentRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let args = json!([
        body.config_id,
        body.hypothesis,
        body.mutation_diff,
        body.lineage_name,
    ]);

    port.neural_lab_experiment_create(args)
        .await
        .map_err(state_err)?;

    let val = json!({
        "ok": true,
        "configId": body.config_id,
        "lineage": body.lineage_name,
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "neural-lab".into(),
        event: "experiment_created".into(),
        data: val.clone(),
    });

    Ok((StatusCode::CREATED, Json(val)))
}

/// PATCH /api/neural-lab/experiments/:id/start — start an experiment.
/// Delegates to SpacetimeDB `experiment_start` reducer.
pub async fn start_experiment(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<StartExperimentRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    port.neural_lab_experiment_start(&id, &body.gpu_node_id)
        .await
        .map_err(state_err)?;

    let val = json!({
        "ok": true,
        "id": id,
        "status": "training",
        "gpuNodeId": body.gpu_node_id,
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "neural-lab".into(),
        event: "experiment_started".into(),
        data: val.clone(),
    });

    Ok(Json(val))
}

/// PATCH /api/neural-lab/experiments/:id/complete — complete an experiment.
/// Delegates to SpacetimeDB `experiment_complete` reducer.
pub async fn complete_experiment(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<CompleteExperimentRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let args = json!([
        id,
        body.val_bpb,
        body.train_loss_final,
        body.tokens_processed,
        body.wall_time_secs,
        body.git_commit,
    ]);

    port.neural_lab_experiment_complete(args)
        .await
        .map_err(state_err)?;

    let val = json!({
        "ok": true,
        "id": id,
        "valBpb": body.val_bpb,
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "neural-lab".into(),
        event: "experiment_completed".into(),
        data: val.clone(),
    });

    Ok(Json(val))
}

/// PATCH /api/neural-lab/experiments/:id/fail — fail an experiment.
/// Delegates to SpacetimeDB `experiment_fail` reducer.
pub async fn fail_experiment(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<FailExperimentRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    port.neural_lab_experiment_fail(&id, &body.error_message)
        .await
        .map_err(state_err)?;

    let val = json!({
        "ok": true,
        "id": id,
        "status": "failed",
    });

    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: "neural-lab".into(),
        event: "experiment_failed".into(),
        data: val.clone(),
    });

    Ok(Json(val))
}

// ── Frontier & Strategy Handlers ────────────────────────

/// GET /api/neural-lab/frontier/:lineage — get ResearchFrontier with best config.
pub async fn get_frontier(
    State(state): State<SharedState>,
    Path(lineage): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;

    let frontier = port
        .neural_lab_frontier_get(&lineage)
        .await
        .map_err(state_err)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("No frontier for lineage '{}'", lineage) })),
            )
        })?;

    // Enrich with the best config details
    let best_config_id = frontier
        .get("best_config_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let best_config = if !best_config_id.is_empty() {
        port.neural_lab_config_get(best_config_id)
            .await
            .map_err(state_err)?
    } else {
        None
    };

    Ok(Json(json!({
        "frontier": frontier,
        "bestConfig": best_config,
    })))
}

/// GET /api/neural-lab/strategies — list MutationStrategy entries.
pub async fn list_strategies(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let port = state_port(&state)?;
    let strategies = port
        .neural_lab_strategies_list()
        .await
        .map_err(state_err)?;

    Ok(Json(Value::Array(strategies)))
}

// ── WebSocket Subscription Bridge (Step 7) ──────────────
//
// TODO(step-7): WebSocket subscription bridge for real-time experiment lifecycle.
//
// hex-nexus should subscribe to the neural-lab SpacetimeDB module's Experiment
// table changes and react to status transitions:
//
//   - status="queued"     → Dispatch experiment to GPU fleet via fleet manager.
//                            POST /api/fleet/dispatch with config_id + experiment_id.
//
//   - status="kept"       → Archive the model checkpoint to persistent storage.
//                            The checkpoint path is derived from git_commit + config_id.
//                            Notify the research coordinator that a new best was found.
//
//   - status="discarded"  → Schedule cleanup of ephemeral training artifacts.
//                            The GPU node can reclaim disk space for the next run.
//
//   - status="failed"     → Log error, optionally retry if error is transient
//                            (OOM, network timeout). Decrement strategy confidence.
//
// Implementation requires SpacetimeDB Rust SDK client subscription support
// (currently only available in the TypeScript SDK). When the Rust SDK adds
// `subscribe("SELECT * FROM experiment")` callback support, implement this as
// a background task in hex-nexus that bridges STDB events → WsEnvelope broadcasts
// so the dashboard receives real-time updates without polling.
//
// For now, the REST endpoints above serve as the synchronous API, and the
// dashboard can poll GET /api/neural-lab/experiments?status=training periodically.
