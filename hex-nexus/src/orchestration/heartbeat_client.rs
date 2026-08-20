//! Heartbeat client — spawn a background tokio task that beats every
//! 15 s on behalf of a long-running component (ADR-2026-05-19-0900 P1.4).
//!
//! Usage from a startup path:
//!
//! ```ignore
//! HeartbeatClient::spawn("nexus-server", "nexus-default");
//! ```
//!
//! The task registers a `worker_process` row on first beat (the reducer
//! is upsert by id so no separate register call is needed), then beats
//! every 15 s with status=Healthy. STDB-side `supervisor_tick`
//! (ADR-2026-05-19-0900 P3.2) reaps any row whose `last_heartbeat` is older
//! than 60 s — so as long as this client is alive it stays "alive" in
//! the supervision layer's eyes.
//!
//! No Drop deregister — the supervisor's stale-heartbeat reaper handles
//! the missed-deregister case. Adding a Drop hook would require either
//! a blocking call from `Drop::drop` (forbidden) or a separate shutdown
//! channel; we accept the ~60 s lag for now and let the reaper clean up.

use std::time::Duration;

use hex_core::domain::heartbeat::HeartbeatStatus;
use hex_core::ports::heartbeat::IHeartbeatPort;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::adapters::spacetime_heartbeat::SpacetimeHeartbeatAdapter;

/// Cadence for `beat()` calls. Must be <= supervisor's STALE_HEARTBEAT_SECS
/// (60s) / 3 to leave headroom for one missed beat. 15 s = 4× headroom.
const BEAT_INTERVAL_SECS: u64 = 15;

pub struct HeartbeatClient;

impl HeartbeatClient {
    /// Spawn a tokio task that beats on behalf of `role` until the
    /// process exits. `role` is the worker_process.role field — e.g.
    /// "nexus-server", "sched-daemon", "workplan-executor". `pool_id`
    /// groups workers of the same role for supervisor accounting; the
    /// nexus convention is `<role>-default` when there's no pool intent
    /// already configured.
    ///
    /// `worker_id` is computed from `<role>-<host>-<pid>`. Re-registers
    /// on every beat (the reducer is upsert by id), so a brief STDB
    /// outage just produces a gap in last_heartbeat — no permanent state
    /// loss.
    pub fn spawn(role: &str, pool_id: &str) -> JoinHandle<()> {
        let role = role.to_string();
        let pool_id = pool_id.to_string();
        let pid = std::process::id();
        let host = hostname();
        let worker_id = format!("{role}-{host}-{pid}");

        tokio::spawn(async move {
            let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
                .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
            let database = std::env::var("HEX_STDB_DATABASE")
                .unwrap_or_else(|_| "hex".to_string());
            let adapter = SpacetimeHeartbeatAdapter::new(stdb_host, database);

            // First-time register. Best-effort — if STDB is down at
            // startup, the per-beat upsert below will heal once it's
            // reachable.
            match adapter.register(&worker_id, &pool_id, &role, pid, &host).await {
                Ok(_) => info!(
                    role = %role,
                    worker_id = %worker_id,
                    "heartbeat client: registered with worker_process"
                ),
                Err(e) => warn!(
                    role = %role,
                    worker_id = %worker_id,
                    error = %e,
                    "heartbeat client: initial register failed — will retry on first beat"
                ),
            }

            let mut ticker = time::interval(Duration::from_secs(BEAT_INTERVAL_SECS));
            // Skip missed ticks during nexus shutdown / GC pauses — better
            // to land the next on-time beat than to fire a backlog.
            ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
            // The first tick fires immediately; we already registered above
            // so consume it without a beat.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = adapter.beat(&worker_id, HeartbeatStatus::Healthy, None).await {
                    warn!(
                        role = %role,
                        worker_id = %worker_id,
                        error = %e,
                        "heartbeat client: beat failed — will retry next tick"
                    );
                }
            }
        })
    }
}

fn hostname() -> String {
    // The `hostname` crate isn't a workspace dep; read /etc/hostname
    // directly. Falls back to "unknown" if unreadable.
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
