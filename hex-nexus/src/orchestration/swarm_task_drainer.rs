//! Drain orphaned swarm_task rows that no worker is claiming.
//!
//! Two task pipelines coexist in this codebase:
//!
//!   inference_task  — newer; created by brain-chat @-mention enqueue,
//!                     polled by the supervisor pool's hex-agent workers.
//!                     Drains correctly.
//!
//!   swarm_task      — older; created by the workplan executor + a few
//!                     legacy "hex brain enqueue" paths. NOTHING currently
//!                     polls these. They accumulate forever.
//!
//! The Kanban panel reads from swarm_task. Operators see thousands of
//! pending rows that will never run, no way to know why. Until the
//! workplan executor migrates to inference_task, this drainer is the
//! release valve: every 5min it marks unassigned pending swarm_tasks
//! older than 24h as `failed` so the Ready lane doesn't grow unbounded.
//!
//! Conservative: only drains tasks with NO agent_id. If an agent has
//! claimed and is taking long, that's not orphaned — leave it.

use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::state::SharedState;

const POLL_INTERVAL_SECS: u64 = 300; // 5 minutes
const ORPHAN_AGE_HOURS: i64 = 24;

pub struct SwarmTaskDrainer {
    state: SharedState,
}

impl SwarmTaskDrainer {
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        tokio::spawn(async move {
            Self { state }.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        info!(
            "swarm_task drainer started (poll {}s, orphan age {}h)",
            POLL_INTERVAL_SECS, ORPHAN_AGE_HOURS
        );
        // Skip the immediate first tick — let nexus warm up before scanning.
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                warn!("swarm_task drainer tick error: {}", e);
            }
        }
    }

    async fn tick(&self) -> Result<(), String> {
        let port = match self.state.state_port.as_ref() {
            Some(p) => p,
            None => return Ok(()),
        };
        let tasks = port
            .swarm_task_list(None)
            .await
            .map_err(|e| format!("swarm_task_list: {}", e))?;
        let now = chrono::Utc::now();
        let cutoff = now - chrono::Duration::hours(ORPHAN_AGE_HOURS);
        let orphans: Vec<_> = tasks
            .iter()
            .filter(|t| t.status == "pending")
            .filter(|t| t.agent_id.is_empty())
            .filter(|t| {
                let created = chrono::DateTime::parse_from_rfc3339(&t.created_at)
                    .map(|dt| dt.with_timezone(&chrono::Utc));
                match created {
                    Ok(c) => c < cutoff,
                    Err(_) => true, // unparseable timestamp = old
                }
            })
            .collect();
        if orphans.is_empty() {
            return Ok(());
        }
        info!(
            "swarm_task drainer: marking {} orphaned pending tasks as failed (age >= {}h, no agent claim)",
            orphans.len(),
            ORPHAN_AGE_HOURS
        );
        let mut ok = 0u32;
        let mut errs = 0u32;
        for t in &orphans {
            // Reuse swarm_task_fail (CAS-checked at the WASM layer).
            match port.swarm_task_fail(&t.id, "auto-drained: orphaned (no agent claim within 24h)").await {
                Ok(()) => ok += 1,
                Err(e) => {
                    warn!("swarm_task_fail({}): {}", t.id, e);
                    errs += 1;
                }
            }
        }
        info!("swarm_task drainer: drained ok={} err={}", ok, errs);
        Ok(())
    }
}
