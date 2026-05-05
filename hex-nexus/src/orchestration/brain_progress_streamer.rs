//! Mid-flight progress reporting for brain-chat dispatches.
//!
//! Operator pain: between "task enqueued" and "task completed" (which can
//! be 30s-5min for hex-coder), the chat thread is silent. The worker is
//! running but the operator doesn't know it. This poller bridges that gap.
//!
//! Every 10s:
//!   - Find inference_task rows where workplan_id starts with "brain-chat:"
//!     AND status is "Pending" or "InProgress".
//!   - For each, look up the originating thread.
//!   - Post a one-line progress heartbeat:
//!       "▶ hex-coder claimed dispatch abc12345 — running…"     (first time after claim)
//!       "⏱ hex-coder dispatch abc12345 — 45s elapsed"          (every 30s while running)
//!   - On Completed/Failed, the brain_dispatch_reconciler already posts
//!     the final result; we don't duplicate.
//!
//! State is tracked via two memory keys per dispatch:
//!   "brain-stream-claimed:<dispatch_id>"  — first heartbeat already posted
//!   "brain-stream-last-tick:<dispatch_id>" — RFC3339 of last tick post
//!
//! Net result: operator opens the chat thread mid-flight and sees progress
//! without leaving the surface or polling the dispatch panel.

use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::ports::state::IStatePort;
use crate::state::SharedState;

const POLL_INTERVAL_SECS: u64 = 10;
const HEARTBEAT_INTERVAL_SECS: i64 = 30;
const CLAIMED_KEY_PREFIX: &str = "brain-stream-claimed:";
const LAST_TICK_KEY_PREFIX: &str = "brain-stream-last-tick:";

pub struct BrainProgressStreamer {
    state: SharedState,
}

impl BrainProgressStreamer {
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        tokio::spawn(async move {
            Self { state }.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        info!("brain progress streamer started (poll {}s, heartbeat every {}s)",
            POLL_INTERVAL_SECS, HEARTBEAT_INTERVAL_SECS);
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                warn!("brain progress streamer tick error: {}", e);
            }
        }
    }

    async fn tick(&self) -> Result<(), String> {
        let port: &Arc<dyn IStatePort> = match self.state.state_port.as_ref() {
            Some(p) => p,
            None => return Ok(()),
        };

        let rows = port
            .inference_task_list_all()
            .await
            .map_err(|e| format!("inference_task_list_all: {}", e))?;

        let in_flight: Vec<_> = rows
            .iter()
            .filter(|r| r.workplan_id.starts_with("brain-chat:"))
            .filter(|r| r.status == "Pending" || r.status == "InProgress")
            .collect();

        if in_flight.is_empty() {
            return Ok(());
        }

        for t in in_flight {
            let thread_id = match extract_thread_id(&t.workplan_id) {
                Some(id) if !id.is_empty() && id != "global" => id,
                _ => continue, // global / no-thread dispatches: nothing to post to
            };

            let claimed_key = format!("{}{}", CLAIMED_KEY_PREFIX, t.id);
            let already_claimed_post = port
                .hexflo_memory_retrieve(&claimed_key)
                .await
                .ok()
                .flatten()
                .is_some();

            // FIRST-CONTACT POST: when status flips to InProgress and we
            // haven't posted a "claimed" message yet.
            if t.status == "InProgress" && !already_claimed_post {
                let body = format!(
                    "▶ **@{}** claimed dispatch `{}` — running now…",
                    t.role,
                    &t.id[..t.id.len().min(8)],
                );
                if append_to_thread(port.as_ref(), &thread_id, &t.role, &body).await.is_ok() {
                    let _ = port.hexflo_memory_store(
                        &claimed_key,
                        &serde_json::json!({"ts": chrono::Utc::now().to_rfc3339()}).to_string(),
                        "system",
                    ).await;
                    info!("brain streamer: posted claim heartbeat for {} to thread {}",
                        &t.id[..8], thread_id);
                }
                continue;
            }

            // ELAPSED HEARTBEAT: every HEARTBEAT_INTERVAL_SECS while
            // InProgress. Skips Pending (worker hasn't claimed yet).
            if t.status != "InProgress" { continue; }

            let last_tick_key = format!("{}{}", LAST_TICK_KEY_PREFIX, t.id);
            let last_tick = port.hexflo_memory_retrieve(&last_tick_key).await.ok().flatten();
            let now = chrono::Utc::now();
            let should_post = match last_tick {
                None => true,
                Some(s) => {
                    let parsed: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::Value::Null);
                    let last_str = parsed.get("ts").and_then(|v| v.as_str()).unwrap_or("");
                    match chrono::DateTime::parse_from_rfc3339(last_str) {
                        Ok(dt) => (now - dt.with_timezone(&chrono::Utc)).num_seconds() >= HEARTBEAT_INTERVAL_SECS,
                        Err(_) => true,
                    }
                }
            };
            if !should_post { continue; }

            // Compute elapsed since the task was claimed (use updated_at as proxy).
            let elapsed_str = chrono::DateTime::parse_from_rfc3339(&t.updated_at)
                .or_else(|_| chrono::DateTime::parse_from_rfc3339(&t.created_at))
                .map(|dt| {
                    let secs = (now - dt.with_timezone(&chrono::Utc)).num_seconds().max(0);
                    if secs < 60 { format!("{}s", secs) }
                    else if secs < 3600 { format!("{}m{}s", secs / 60, secs % 60) }
                    else { format!("{}h{}m", secs / 3600, (secs % 3600) / 60) }
                })
                .unwrap_or_else(|_| "?".to_string());
            let body = format!(
                "⏱ **@{}** dispatch `{}` — still running ({} elapsed)",
                t.role,
                &t.id[..t.id.len().min(8)],
                elapsed_str,
            );
            if append_to_thread(port.as_ref(), &thread_id, &t.role, &body).await.is_ok() {
                let _ = port.hexflo_memory_store(
                    &last_tick_key,
                    &serde_json::json!({"ts": now.to_rfc3339()}).to_string(),
                    "system",
                ).await;
            }
        }

        Ok(())
    }
}

/// Extract the thread id from a brain-chat workplan_id.
/// Format: "brain-chat:<thread>" or "brain-chat:<thread>:auto-followup".
fn extract_thread_id(workplan_id: &str) -> Option<String> {
    let stripped = workplan_id.strip_prefix("brain-chat:")?;
    let id = stripped.split(':').next().unwrap_or(stripped);
    if id.is_empty() { None } else { Some(id.to_string()) }
}

/// Append a synthetic message to a chat thread record. Mirrors the structure
/// used by brain_dispatch_reconciler::post_result_to_thread but lighter
/// (no result truncation, just a one-liner).
async fn append_to_thread(
    port: &dyn IStatePort,
    thread_id: &str,
    role: &str,
    body: &str,
) -> Result<(), String> {
    let thread_key = format!("chat:thread:{}", thread_id);
    let raw = match port.hexflo_memory_retrieve(&thread_key).await {
        Ok(Some(s)) => s,
        _ => return Err("thread not found".to_string()),
    };
    let mut record: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| format!("parse thread: {}", e))?;
    let new_msg = serde_json::json!({
        "from": role,
        "text": body,
        "ts": chrono::Utc::now().to_rfc3339(),
        "model": "worker-progress",
    });
    if let Some(messages) = record.get_mut("messages").and_then(|m| m.as_array_mut()) {
        messages.push(new_msg);
    } else {
        record["messages"] = serde_json::json!([new_msg]);
    }
    record["last_active_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
    let serialized = serde_json::to_string(&record).map_err(|e| format!("serialize: {}", e))?;
    port.hexflo_memory_store(&thread_key, &serialized, "system")
        .await
        .map_err(|e| format!("store: {}", e))?;
    Ok(())
}
