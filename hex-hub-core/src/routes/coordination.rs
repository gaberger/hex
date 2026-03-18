use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::state::{
    ActivityEntry, ActivityRequest, HeartbeatRequest, InstanceInfo, LockRequest,
    RegisterInstanceRequest, SharedState, TaskClaim, TaskClaimRequest, UnstagedState,
    WorktreeLock, MAX_ACTIVITIES,
};

// ── Instance Management ─────────────────────────────────

pub async fn register_instance(
    State(state): State<SharedState>,
    Json(req): Json<RegisterInstanceRequest>,
) -> Json<serde_json::Value> {
    let instance_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let info = InstanceInfo {
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
    state.instances.write().await.insert(instance_id.clone(), info);
    Json(json!({ "instanceId": instance_id }))
}

pub async fn heartbeat_instance(
    State(state): State<SharedState>,
    Json(req): Json<HeartbeatRequest>,
) -> Json<serde_json::Value> {
    let now = chrono::Utc::now().to_rfc3339();

    // Update instance last_seen and swarm state
    let mut instances = state.instances.write().await;
    if let Some(inst) = instances.get_mut(&req.instance_id) {
        inst.last_seen = now.clone();
        if req.agent_count.is_some() {
            inst.agent_count = req.agent_count;
        }
        if req.active_task_count.is_some() {
            inst.active_task_count = req.active_task_count;
        }
        if req.completed_task_count.is_some() {
            inst.completed_task_count = req.completed_task_count;
        }
        if req.topology.is_some() {
            inst.topology = req.topology.clone();
        }
    } else {
        return Json(json!({ "error": "instance not found" }));
    }
    drop(instances);

    // Refresh heartbeat on all locks held by this instance
    let mut locks = state.worktree_locks.write().await;
    for lock in locks.values_mut() {
        if lock.instance_id == req.instance_id {
            lock.heartbeat_at = now.clone();
        }
    }
    drop(locks);

    // Refresh heartbeat on all task claims held by this instance
    let mut claims = state.task_claims.write().await;
    for claim in claims.values_mut() {
        if claim.instance_id == req.instance_id {
            claim.heartbeat_at = now.clone();
        }
    }
    drop(claims);

    // Update unstaged files
    if let Some(files) = req.unstaged_files {
        let unstaged = UnstagedState {
            instance_id: req.instance_id.clone(),
            project_id: req.project_id,
            files,
            captured_at: now,
        };
        state.unstaged.write().await.insert(req.instance_id, unstaged);
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
    let instances = state.instances.read().await;
    let list: Vec<_> = instances
        .values()
        .filter(|i| q.project_id.as_ref().map_or(true, |pid| &i.project_id == pid))
        .cloned()
        .collect();
    Json(json!(list))
}

pub async fn cleanup_stale_sessions(
    State(state): State<SharedState>,
) -> Json<serde_json::Value> {
    match crate::cleanup::cleanup_stale_sessions(&state).await {
        Ok(removed) => Json(json!({ "removed": removed })),
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
    let key = format!("{}:{}:{}", req.project_id, req.feature, req.layer);
    let now = chrono::Utc::now().to_rfc3339();
    let ttl = req.ttl_secs.unwrap_or(300); // default 5 minutes

    let mut locks = state.worktree_locks.write().await;

    if let Some(existing) = locks.get(&key) {
        // Already held — conflict
        Json(json!({
            "acquired": false,
            "lock": null,
            "conflict": existing,
        }))
    } else {
        let lock = WorktreeLock {
            instance_id: req.instance_id,
            project_id: req.project_id,
            feature: req.feature,
            layer: req.layer,
            acquired_at: now.clone(),
            heartbeat_at: now,
            ttl_secs: ttl,
        };
        let result = lock.clone();
        locks.insert(key, lock);
        Json(json!({
            "acquired": true,
            "lock": result,
            "conflict": null,
        }))
    }
}

pub async fn release_lock(
    State(state): State<SharedState>,
    Path(key): Path<String>,
) -> Json<serde_json::Value> {
    let mut locks = state.worktree_locks.write().await;
    let removed = locks.remove(&key).is_some();
    Json(json!({ "released": removed }))
}

pub async fn list_locks(
    State(state): State<SharedState>,
    Query(q): Query<ProjectQuery>,
) -> Json<serde_json::Value> {
    let locks = state.worktree_locks.read().await;
    let list: Vec<_> = locks
        .values()
        .filter(|l| q.project_id.as_ref().map_or(true, |pid| &l.project_id == pid))
        .cloned()
        .collect();
    Json(json!(list))
}

// ── Task Claims ─────────────────────────────────────────

pub async fn claim_task(
    State(state): State<SharedState>,
    Json(req): Json<TaskClaimRequest>,
) -> Json<serde_json::Value> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut claims = state.task_claims.write().await;

    if let Some(existing) = claims.get(&req.task_id) {
        Json(json!({
            "claimed": false,
            "claim": null,
            "conflict": existing,
        }))
    } else {
        let claim = TaskClaim {
            task_id: req.task_id.clone(),
            instance_id: req.instance_id,
            claimed_at: now.clone(),
            heartbeat_at: now,
        };
        let result = claim.clone();
        claims.insert(req.task_id, claim);
        Json(json!({
            "claimed": true,
            "claim": result,
            "conflict": null,
        }))
    }
}

pub async fn release_task(
    State(state): State<SharedState>,
    Path(task_id): Path<String>,
) -> Json<serde_json::Value> {
    let mut claims = state.task_claims.write().await;
    let removed = claims.remove(&task_id).is_some();
    Json(json!({ "released": removed }))
}

pub async fn list_claims(
    State(state): State<SharedState>,
    Query(q): Query<ProjectQuery>,
) -> Json<serde_json::Value> {
    let claims = state.task_claims.read().await;
    let instances = state.instances.read().await;

    let list: Vec<_> = claims
        .values()
        .filter(|c| {
            q.project_id.as_ref().map_or(true, |pid| {
                instances
                    .get(&c.instance_id)
                    .map_or(false, |i| &i.project_id == pid)
            })
        })
        .cloned()
        .collect();
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
        .filter(|a| q.project_id.as_ref().map_or(true, |pid| &a.project_id == pid))
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
    let unstaged = state.unstaged.read().await;
    let list: Vec<_> = unstaged
        .values()
        .filter(|u| q.project_id.as_ref().map_or(true, |pid| &u.project_id == pid))
        .cloned()
        .collect();
    Json(json!(list))
}

// ── Background Eviction ─────────────────────────────────

/// Called from the main eviction loop (every 60s).
/// Removes dead instances, their locks, claims, and unstaged state.
#[allow(dead_code)] // Will be wired into background eviction loop
pub async fn evict_stale(state: &SharedState) {
    let now = chrono::Utc::now();
    let heartbeat_timeout = chrono::Duration::seconds(60); // 2x the 30s heartbeat

    // 1. Find dead instances
    let dead_instances: Vec<String> = {
        let instances = state.instances.read().await;
        instances
            .iter()
            .filter_map(|(id, info)| {
                let last = chrono::DateTime::parse_from_rfc3339(&info.last_seen).ok()?;
                if now.signed_duration_since(last) > heartbeat_timeout {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    };

    if dead_instances.is_empty() {
        return;
    }

    let dead_count = dead_instances.len();

    // 2. Remove dead instances
    {
        let mut instances = state.instances.write().await;
        for id in &dead_instances {
            instances.remove(id);
        }
    }

    // 3. Release locks held by dead instances
    {
        let mut locks = state.worktree_locks.write().await;
        locks.retain(|_, lock| !dead_instances.contains(&lock.instance_id));
    }

    // 4. Release task claims held by dead instances
    {
        let mut claims = state.task_claims.write().await;
        claims.retain(|_, claim| !dead_instances.contains(&claim.instance_id));
    }

    // 5. Remove unstaged state for dead instances
    {
        let mut unstaged = state.unstaged.write().await;
        for id in &dead_instances {
            unstaged.remove(id);
        }
    }

    // 6. Evict expired locks (TTL exceeded even if instance is alive)
    {
        let mut locks = state.worktree_locks.write().await;
        locks.retain(|_, lock| {
            if let Ok(hb) = chrono::DateTime::parse_from_rfc3339(&lock.heartbeat_at) {
                let elapsed = now.signed_duration_since(hb);
                elapsed < chrono::Duration::seconds(lock.ttl_secs as i64)
            } else {
                false // can't parse timestamp — evict
            }
        });
    }

    tracing::debug!("Coordination eviction: removed {} dead instance(s)", dead_count);
}
