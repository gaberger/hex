use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rl::patterns::PatternStore;
use crate::rl::q_learning::QLearningEngine;
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

fn db_unavailable() -> (StatusCode, Json<Value>) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "Database not available" })),
    )
}

// ── Handlers ───────────────────────────────────────────

/// POST /api/rl/action — select an action via epsilon-greedy Q-learning
pub async fn select_action(
    State(state): State<SharedState>,
    Json(body): Json<ActionRequest>,
) -> ApiResult {
    let db = state.swarm_db.as_ref().ok_or_else(db_unavailable)?;
    let conn = db.conn().clone();

    let action = tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        let engine = QLearningEngine::new();
        let state_key = QLearningEngine::discretize_state(
            &body.state.task_type,
            body.state.codebase_size,
            body.state.agent_count,
            body.state.token_usage,
        );
        let action = engine.select_action(&conn, &state_key);
        (action, state_key)
    })
    .await
    .expect("spawn_blocking join");

    Ok(Json(json!({
        "action": action.0,
        "stateKey": action.1,
    })))
}

/// POST /api/rl/reward — update Q-table with observed reward
pub async fn submit_reward(
    State(state): State<SharedState>,
    Json(body): Json<RewardRequest>,
) -> ApiResult {
    let db = state.swarm_db.as_ref().ok_or_else(db_unavailable)?;
    let conn = db.conn().clone();

    tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        let mut engine = QLearningEngine::new();
        engine.update(&conn, &body.state_key, &body.action, body.reward, &body.next_state_key);
        QLearningEngine::record_experience(
            &conn,
            &body.state_key,
            &body.action,
            body.reward,
            &body.next_state_key,
            "reward_update",
        );
    })
    .await
    .expect("spawn_blocking join");

    Ok(Json(json!({ "ok": true })))
}

/// GET /api/rl/stats — return Q-table aggregate stats
pub async fn get_stats(
    State(state): State<SharedState>,
) -> ApiResult {
    let db = state.swarm_db.as_ref().ok_or_else(db_unavailable)?;
    let conn = db.conn().clone();

    let stats = tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        let engine = QLearningEngine::new();
        QLearningEngine::get_stats(&conn, engine.epsilon)
    })
    .await
    .expect("spawn_blocking join");

    Ok(Json(serde_json::to_value(stats).unwrap()))
}

/// POST /api/rl/patterns — store a new pattern
pub async fn store_pattern(
    State(state): State<SharedState>,
    Json(body): Json<StorePatternRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let db = state.swarm_db.as_ref().ok_or_else(db_unavailable)?;
    let conn = db.conn().clone();

    let id = tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        PatternStore::store(&conn, &body.category, &body.content, body.confidence)
    })
    .await
    .expect("spawn_blocking join");

    Ok((StatusCode::CREATED, Json(json!({ "id": id }))))
}

/// GET /api/rl/patterns?category=X&query=Y&limit=N — search patterns
pub async fn search_patterns(
    State(state): State<SharedState>,
    Query(params): Query<PatternQuery>,
) -> ApiResult {
    let db = state.swarm_db.as_ref().ok_or_else(db_unavailable)?;
    let conn = db.conn().clone();

    let category = params.category.unwrap_or_default();
    let query = params.query.unwrap_or_default();
    let limit = params.limit.unwrap_or(10);

    let patterns = tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        PatternStore::search(&conn, &category, &query, limit)
    })
    .await
    .expect("spawn_blocking join");

    Ok(Json(serde_json::to_value(patterns).unwrap()))
}

/// POST /api/rl/patterns/:id/reinforce — adjust pattern confidence
pub async fn reinforce_pattern(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<ReinforceRequest>,
) -> ApiResult {
    let db = state.swarm_db.as_ref().ok_or_else(db_unavailable)?;
    let conn = db.conn().clone();

    tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        PatternStore::reinforce(&conn, &id, body.delta);
    })
    .await
    .expect("spawn_blocking join");

    Ok(Json(json!({ "ok": true })))
}

/// POST /api/rl/decay — apply temporal decay to all patterns
pub async fn decay_patterns(
    State(state): State<SharedState>,
) -> ApiResult {
    let db = state.swarm_db.as_ref().ok_or_else(db_unavailable)?;
    let conn = db.conn().clone();

    tokio::task::spawn_blocking(move || {
        let conn = conn.blocking_lock();
        PatternStore::decay_all(&conn);
    })
    .await
    .expect("spawn_blocking join");

    Ok(Json(json!({ "ok": true })))
}
