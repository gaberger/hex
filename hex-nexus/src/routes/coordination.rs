use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::ports::state::{
    InstanceHeartbeat, InstanceRecord, TaskClaimRecord, UnstagedFileRecord,
    UnstagedRecord, WorktreeLockRecord,
};
use crate::state::{
    ActivityEntry, ActivityRequest, HeartbeatRequest,
    RegisterInstanceRequest, SharedState, TaskClaimRequest, LockRequest,
    MAX_ACTIVITIES,
};

// ── Instance Management ─────────────────────────────────

pub async fn register_instance(
    State(state): State<SharedState>,
    Json(req): Json<RegisterInstanceRequest>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "error": "State port not configured" })),
    };

    let instance_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let info = InstanceRecord {
        instance_id: instance_id.clone(),
        project_id: req.project_id.clone(),
        pid: req.pid,
        session_label: req.session_label.unwrap_or_default(),
        registered_at: now.clone(),
        last_seen: now,
        agent_count: None,
        active_task_count: None,
        completed_task_count: None,
        topology: None,
    };

    if let Err(e) = sp.instance_register(info).await {
        tracing::error!("Failed to register instance: {}", e);
        return Json(json!({ "error": e.to_string() }));
    }

    Json(json!({ "instanceId": instance_id }))
}

pub async fn heartbeat_instance(
    State(state): State<SharedState>,
    Json(req): Json<HeartbeatRequest>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "error": "State port not configured" })),
    };

    let now = chrono::Utc::now().to_rfc3339();

    // Update instance last_seen and swarm state
    let update = InstanceHeartbeat {
        agent_count: req.agent_count,
        active_task_count: req.active_task_count,
        completed_task_count: req.completed_task_count,
        topology: req.topology.clone(),
    };

    if let Err(e) = sp.instance_heartbeat(&req.instance_id, update).await {
        return Json(json!({ "error": format!("instance not found: {}", e) }));
    }

    // Refresh heartbeat on all locks held by this instance
    let _ = sp.worktree_lock_refresh(&req.instance_id, &now).await;

    // Refresh heartbeat on all task claims held by this instance
    let _ = sp.task_claim_refresh(&req.instance_id, &now).await;

    // Update unstaged files
    if let Some(files) = req.unstaged_files {
        let unstaged = UnstagedRecord {
            instance_id: req.instance_id.clone(),
            project_id: req.project_id,
            files: files.into_iter().map(|f| UnstagedFileRecord {
                path: f.path,
                status: f.status,
                layer: f.layer,
            }).collect(),
            captured_at: now,
        };
        let _ = sp.unstaged_update(&req.instance_id, unstaged).await;
    }

    Json(json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectQuery {
    pub project_id: Option<String>,
}

pub async fn list_instances(
    State(state): State<SharedState>,
    Query(q): Query<ProjectQuery>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!([])),
    };

    let list = sp.instance_list(q.project_id.as_deref()).await.unwrap_or_default();
    Json(json!(list))
}

pub async fn cleanup_stale_sessions(
    State(state): State<SharedState>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "error": "State port not configured", "removed": 0 })),
    };

    match sp.coordination_cleanup_stale(360).await {
        Ok(report) => Json(json!({ "removed": report.instances_removed })),
        Err(e) => {
            tracing::error!("Cleanup failed: {}", e);
            Json(json!({ "error": "Cleanup failed", "removed": 0 }))
        }
    }
}

// ── Worktree Locks ──────────────────────────────────────

pub async fn acquire_lock(
    State(state): State<SharedState>,
    Json(req): Json<LockRequest>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "error": "State port not configured" })),
    };

    let key = format!("{}:{}:{}", req.project_id, req.feature, req.layer);
    let now = chrono::Utc::now().to_rfc3339();
    let ttl = req.ttl_secs.unwrap_or(300);

    let lock = WorktreeLockRecord {
        key: key.clone(),
        instance_id: req.instance_id,
        project_id: req.project_id,
        feature: req.feature,
        layer: req.layer,
        acquired_at: now.clone(),
        heartbeat_at: now,
        ttl_secs: ttl,
    };
    let lock_result = lock.clone();

    match sp.worktree_lock_acquire(lock).await {
        Ok(true) => Json(json!({
            "acquired": true,
            "lock": lock_result,
            "conflict": null,
        })),
        Ok(false) => Json(json!({
            "acquired": false,
            "lock": null,
            "conflict": { "key": key },
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

pub async fn release_lock(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "error": "State port not configured" })),
    };

    let released = sp.worktree_lock_release(&key).await.unwrap_or(false);
    Json(json!({ "released": released }))
}

pub async fn list_locks(
    State(state): State<SharedState>,
    Query(q): Query<ProjectQuery>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!([])),
    };

    let list = sp.worktree_lock_list(q.project_id.as_deref()).await.unwrap_or_default();
    Json(json!(list))
}

// ── Task Claims ─────────────────────────────────────────

pub async fn claim_task(
    State(state): State<SharedState>,
    Json(req): Json<TaskClaimRequest>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "error": "State port not configured" })),
    };

    let now = chrono::Utc::now().to_rfc3339();
    let claim = TaskClaimRecord {
        task_id: req.task_id.clone(),
        instance_id: req.instance_id,
        claimed_at: now.clone(),
        heartbeat_at: now,
    };
    let claim_result = claim.clone();

    match sp.task_claim_acquire(claim).await {
        Ok(true) => Json(json!({
            "claimed": true,
            "claim": claim_result,
            "conflict": null,
        })),
        Ok(false) => Json(json!({
            "claimed": false,
            "claim": null,
            "conflict": { "taskId": req.task_id },
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

pub async fn release_task(
    State(state): State<SharedState>,
    Path(task_id): Path<String>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!({ "error": "State port not configured" })),
    };

    let released = sp.task_claim_release(&task_id).await.unwrap_or(false);
    Json(json!({ "released": released }))
}

pub async fn list_claims(
    State(state): State<SharedState>,
    Query(q): Query<ProjectQuery>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!([])),
    };

    let list = sp.task_claim_list(q.project_id.as_deref()).await.unwrap_or_default();
    Json(json!(list))
}

// ── Activity Stream ─────────────────────────────────────

pub async fn publish_activity(
    State(state): State<SharedState>,
    Json(req): Json<ActivityRequest>,
) -> Json<serde_json::Value> {
    let now = chrono::Utc::now().to_rfc3339();
    let entry = ActivityEntry {
        instance_id: req.instance_id,
        project_id: req.project_id.clone(),
        action: req.action.clone(),
        details: req.details.unwrap_or(serde_json::Value::Object(Default::default())),
        timestamp: now,
    };

    let mut activities = state.activities.write().await;
    if activities.len() >= MAX_ACTIVITIES {
        activities.pop_front();
    }
    activities.push_back(entry);
    drop(activities);

    // Broadcast to WebSocket subscribers
    let _ = state.ws_tx.send(crate::state::WsEnvelope {
        topic: format!("project:{}:coordination", req.project_id),
        event: "activity".to_string(),
        data: json!({ "action": req.action }),
    });

    Json(json!({ "ok": true }))
}

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    #[serde(rename = "projectId")]
    pub project_id: Option<String>,
    pub limit: Option<usize>,
}

pub async fn get_activities(
    State(state): State<SharedState>,
    Query(q): Query<ActivityQuery>,
) -> Json<serde_json::Value> {
    let activities = state.activities.read().await;
    let limit = q.limit.unwrap_or(50).min(MAX_ACTIVITIES);
    let list: Vec<_> = activities
        .iter()
        .rev()
        .filter(|a| q.project_id.as_ref().is_none_or(|pid| &a.project_id == pid))
        .take(limit)
        .cloned()
        .collect();
    Json(json!(list))
}

// ── Unstaged Files ──────────────────────────────────────

pub async fn get_unstaged(
    State(state): State<SharedState>,
    Query(q): Query<ProjectQuery>,
) -> Json<serde_json::Value> {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return Json(json!([])),
    };

    let list = sp.unstaged_list(q.project_id.as_deref()).await.unwrap_or_default();
    Json(json!(list))
}

// ── Background Eviction ─────────────────────────────────

/// Called from the main eviction loop (every 60s).
/// Removes dead instances, their locks, claims, and unstaged state.
#[allow(dead_code)] // Will be wired into background eviction loop
pub async fn evict_stale(state: &SharedState) {
    let sp = match state.state_port.as_ref() {
        Some(sp) => sp,
        None => return,
    };

    match sp.coordination_cleanup_stale(60).await {
        Ok(report) => {
            if report.instances_removed > 0 {
                tracing::debug!(
                    "Coordination eviction: removed {} dead instance(s), {} locks, {} claims",
                    report.instances_removed,
                    report.locks_released,
                    report.claims_released,
                );
            }
        }
        Err(e) => {
            tracing::error!("Coordination eviction failed: {}", e);
        }
    }

    // Also evict expired locks (TTL exceeded even if instance is alive)
    let _ = sp.worktree_lock_evict_expired().await;
}
