use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ports::state::RlState;
use crate::state::SharedState;

// ── Request / Response types ───────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRequest {
    pub state: ActionState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionState {
    pub task_type: String,
    pub codebase_size: u64,
    pub agent_count: u8,
    pub token_usage: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardRequest {
    pub state_key: String,
    pub action: String,
    pub reward: f64,
    pub next_state_key: String,
    /// Whether the request was rate-limited.
    #[serde(default)]
    pub rate_limited: bool,
    /// Actual cost from OpenRouter in USD (0.0 if not applicable).
    #[serde(default)]
    pub openrouter_cost_usd: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorePatternRequest {
    pub category: String,
    pub content: String,
    pub confidence: f64,
}

#[derive(Debug, Deserialize)]
pub struct PatternQuery {
    pub category: Option<String>,
    pub query: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReinforceRequest {
    pub delta: f64,
}

type ApiResult<T = Json<Value>> = Result<T, (StatusCode, Json<Value>)>;

fn port_unavailable() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "State port not available" })),
    )
}

fn state_err(e: crate::ports::state::StateError) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": e.to_string() })),
    )
}

// ── Handlers ───────────────────────────────────────────

/// POST /api/rl/action — select an action via epsilon-greedy Q-learning
pub async fn select_action(
    State(state): State<SharedState>,
    Json(body): Json<ActionRequest>,
) -> ApiResult {
    let port = state.state_port.as_ref().ok_or_else(port_unavailable)?;

    let rl_state = RlState {
        task_type: body.state.task_type,
        codebase_size: body.state.codebase_size,
        agent_count: body.state.agent_count,
        token_usage: body.state.token_usage,
    };

    let action = port.rl_select_action(&rl_state).await.map_err(state_err)?;

    Ok(Json(json!({
        "action": action,
        "stateKey": format!(
            "{}_{}_{}_{}", rl_state.task_type, rl_state.codebase_size,
            rl_state.agent_count, rl_state.token_usage
        ),
    })))
}

/// POST /api/rl/reward — update Q-table with observed reward
pub async fn submit_reward(
    State(state): State<SharedState>,
    Json(body): Json<RewardRequest>,
) -> ApiResult {
    let port = state.state_port.as_ref().ok_or_else(port_unavailable)?;

    port.rl_record_reward(
        &body.state_key,
        &body.action,
        body.reward,
        &body.next_state_key,
        body.rate_limited,
        body.openrouter_cost_usd,
    )
        .await
        .map_err(state_err)?;

    Ok(Json(json!({ "ok": true })))
}

/// GET /api/rl/stats — return Q-table aggregate stats
pub async fn get_stats(
    State(state): State<SharedState>,
) -> ApiResult {
    let port = state.state_port.as_ref().ok_or_else(port_unavailable)?;

    let stats = port.rl_get_stats().await.map_err(state_err)?;

    Ok(Json(serde_json::to_value(stats).unwrap()))
}

/// POST /api/rl/patterns — store a new pattern
pub async fn store_pattern(
    State(state): State<SharedState>,
    Json(body): Json<StorePatternRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let port = state.state_port.as_ref().ok_or_else(port_unavailable)?;

    let id = port
        .pattern_store(&body.category, &body.content, body.confidence)
        .await
        .map_err(state_err)?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// GET /api/rl/patterns?category=X&query=Y&limit=N — search patterns
pub async fn search_patterns(
    State(state): State<SharedState>,
    Query(params): Query<PatternQuery>,
) -> ApiResult {
    let port = state.state_port.as_ref().ok_or_else(port_unavailable)?;

    let category = params.category.unwrap_or_default();
    let query = params.query.unwrap_or_default();
    let limit = params.limit.unwrap_or(10);

    let patterns = port
        .pattern_search(&category, &query, limit)
        .await
        .map_err(state_err)?;

    Ok(Json(serde_json::to_value(patterns).unwrap()))
}

/// POST /api/rl/patterns/:id/reinforce — adjust pattern confidence
pub async fn reinforce_pattern(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<ReinforceRequest>,
) -> ApiResult {
    let port = state.state_port.as_ref().ok_or_else(port_unavailable)?;

    port.pattern_reinforce(&id, body.delta)
        .await
        .map_err(state_err)?;

    Ok(Json(json!({ "ok": true })))
}

/// POST /api/rl/decay — apply temporal decay to all patterns
pub async fn decay_patterns(
    State(state): State<SharedState>,
) -> ApiResult {
    let port = state.state_port.as_ref().ok_or_else(port_unavailable)?;

    let _count = port.pattern_decay_all().await.map_err(state_err)?;

    Ok(Json(json!({ "ok": true })))
}
