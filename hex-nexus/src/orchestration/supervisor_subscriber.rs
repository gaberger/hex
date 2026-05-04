//! Supervisor-event subscriber (wp-stdb-supervisor P3).
//!
//! Polls the STDB `supervisor_event` table for unhandled rows and acts on
//! them. The supervisor's brain (the `supervisor_tick` scheduled reducer)
//! emits `spawn_request` / `crash_loop` rows; this subscriber turns those
//! decisions into actual side-effects (process spawns, inbox alerts,
//! worker_process row registration).
//!
//! Why poll instead of a real STDB subscription: the IStatePort surface
//! exposes `query_table` and `call_reducer` but not push-style row
//! subscriptions. Polling at 5s is plenty — the supervisor tick itself
//! runs at 10s, so polling at half that catches every event within one
//! cycle of latency.
//!
//! Idempotency: each event row is marked `handled=true` once acted on.
//! If the subscriber crashes mid-handler, the event stays unhandled and
//! gets retried on next poll.

use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{debug, error, info, warn};

use crate::state::SharedState;

const POLL_INTERVAL_SECS: u64 = 5;

pub struct SupervisorSubscriber {
    state: SharedState,
}

impl SupervisorSubscriber {
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        tokio::spawn(async move {
            Self { state }.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        info!("supervisor subscriber started (poll interval {}s)", POLL_INTERVAL_SECS);

        // Startup reconciliation (P3.4): any worker_process row with empty
        // exited_at at this point is an orphan — the watchdog tokio task
        // that was tracking it died with the previous nexus process. Mark
        // them all exited so the supervisor's alive_count is honest from
        // the first tick. On a clean restart, all in-flight hex-agents
        // are either dead or orphaned to init anyway — assume dead.
        if let Some(port) = &self.state.state_port {
            match port.worker_process_orphans().await {
                Ok(ids) if !ids.is_empty() => {
                    info!("supervisor: reconciling {} orphaned worker_process rows from prior nexus", ids.len());
                    for id in ids {
                        if let Err(e) = port.worker_process_record_exit(&id, "orphaned").await {
                            warn!("orphan reconcile for {}: {}", id, e);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("orphan reconcile query failed: {}", e),
            }
        }

        // Auto-seed pool placeholders from the embedded persona YAML registry.
        // Each persona gets a worker_pool_intent row with desired=0, paused=true
        // so it appears in `hex pool list` (discoverable) but doesn't spawn any
        // workers until the operator opts in via `hex pool resume <id>` and
        // a desired_count bump. Idempotent — existing pool ids are skipped, so
        // operator-customized pools are never overwritten.
        if let Some(port) = &self.state.state_port {
            if let Err(e) = self.seed_persona_pools(port.as_ref()).await {
                warn!("auto-seed persona pools skipped: {}", e);
            }
        }

        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                warn!("supervisor subscriber tick error: {}", e);
            }
        }
    }

    /// Walk every persona YAML embedded via rust-embed and create a
    /// worker_pool_intent row with desired=0, paused=true for each one whose
    /// pool id (`{role}-default`) doesn't already exist. Lets operators see
    /// the full menu of available roles in `hex pool list` without forcing
    /// them to declare each one manually.
    async fn seed_persona_pools(
        &self,
        port: &dyn crate::ports::state::IStatePort,
    ) -> Result<(), String> {
        // Existing pool ids — skip these.
        let existing: std::collections::HashSet<String> = port
            .pool_status_all()
            .await
            .map_err(|e| format!("pool_status_all: {}", e))?
            .into_iter()
            .map(|t| t.0)
            .collect();

        let mut roles: Vec<String> = crate::templates::AgentTemplates::iter()
            .filter_map(|p| {
                let s = p.as_ref();
                let prefix = "agents/hex/hex/";
                if !s.starts_with(prefix) || !s.ends_with(".yml") { return None; }
                let stem = &s[prefix.len()..s.len() - 4];
                if stem == "adversarial-reviewer" { return None; } // deprecated stub
                Some(stem.to_string())
            })
            .collect();
        roles.sort();
        roles.dedup();

        let mut seeded = 0;
        for role in &roles {
            let pool_id = format!("{}-default", role);
            if existing.contains(&pool_id) { continue; }
            if let Err(e) = port
                .pool_create(&pool_id, role, 0, "permanent", 5, 60, true, "system-seed")
                .await
            {
                warn!("seed pool '{}' skipped: {}", pool_id, e);
                continue;
            }
            seeded += 1;
        }
        if seeded > 0 {
            info!("supervisor: seeded {} pool placeholders from persona YAMLs (desired=0, paused=true)", seeded);
        }
        Ok(())
    }

    async fn tick(&self) -> Result<(), String> {
        let port = match &self.state.state_port {
            Some(p) => p,
            None => return Ok(()), // STDB not configured — nothing to do
        };

        // Query unhandled events. Cap to 50 per tick to avoid runaway
        // processing if a backlog accumulates.
        let mut events = port
            .supervisor_event_unhandled()
            .await
            .map_err(|e| format!("supervisor_event_unhandled: {}", e))?;
        events.truncate(50);
        if events.is_empty() {
            return Ok(());
        }
        info!("supervisor subscriber: processing {} unhandled events", events.len());

        for (id, kind, pool_id, payload) in events {
            match kind.as_str() {
                "spawn_request" => {
                    if let Err(e) = self.handle_spawn_request(&pool_id, &payload).await {
                        warn!("spawn_request handler failed for pool {}: {}", pool_id, e);
                        continue; // leave unhandled; supervisor_tick will re-emit if still needed
                    }
                }
                "crash_loop" => {
                    if let Err(e) = self.handle_crash_loop(&pool_id, &payload).await {
                        warn!("crash_loop handler failed for pool {}: {}", pool_id, e);
                    }
                }
                other => {
                    debug!("supervisor subscriber: ignoring event kind '{}'", other);
                }
            }

            // Mark handled regardless of side-effect outcome — we don't want a
            // permanently-failing handler to spin forever. crash_loop alerts are
            // re-emitted on subsequent ticks anyway if the pool is still in_crash_loop.
            if let Err(e) = port.supervisor_event_mark_handled(id, "nexus-supervisor").await {
                warn!("failed to mark event {} as handled: {}", id, e);
            }
        }
        Ok(())
    }

    /// Spawn a worker for the requested role and register a worker_process row.
    async fn handle_spawn_request(&self, pool_id: &str, payload: &str) -> Result<(), String> {
        let mgr = self
            .state
            .agent_manager
            .as_ref()
            .ok_or("agent_manager not initialized")?;
        let port = self
            .state
            .state_port
            .as_ref()
            .ok_or("state_port not initialized")?;

        // Parse payload JSON for the role (the supervisor_tick emits it as
        // `{"role":"hex-coder","needed":N,"alive":N,"desired":N}`). If parse
        // fails, fall back to looking up the pool's role via SQL.
        let role: String = serde_json::from_str::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| v.get("role").and_then(|r| r.as_str()).map(|s| s.to_string()))
            .unwrap_or_default();

        let role = if role.is_empty() {
            port.worker_pool_role(pool_id)
                .await
                .map_err(|e| format!("worker_pool_role: {}", e))?
                .ok_or_else(|| format!("pool '{}' not found and payload had no role", pool_id))?
        } else {
            role
        };

        let project_dir = std::env::current_dir()
            .map_err(|e| format!("cwd: {}", e))?;
        let hub_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:5555".to_string());

        info!(
            "supervisor: spawning hex-agent for pool '{}' (role={})",
            pool_id, role
        );
        let pid = mgr
            .spawn_local_agent(&hub_url, &project_dir)
            .await
            .map_err(|e| format!("spawn_local_agent: {}", e))?;

        // Register the worker_process row so the supervisor_tick stops asking.
        let process_id = uuid::Uuid::new_v4().to_string();
        let host = gethostname::gethostname().to_string_lossy().to_string();
        if let Err(e) = port
            .worker_process_register(&process_id, pool_id, &role, &host, pid as i64)
            .await
        {
            warn!(
                "spawn succeeded (pid {}) but worker_process_register failed: {} — supervisor will respawn",
                pid, e
            );
            return Ok(());
        }

        // P3.2 — exit watchdog. Polls /proc/<pid> every 2s; when the process
        // is gone, calls worker_process_record_exit so the supervisor's
        // alive_count is honest. Without this, ghost workers (exited
        // hex-agent processes whose row still says alive) prevent the
        // supervisor from spawning replacements.
        let port = std::sync::Arc::clone(port);
        let pid_u = pid;
        let pid_path = format!("/proc/{}", pid_u);
        let process_id_clone = process_id.clone();
        let pool_id_clone = pool_id.to_string();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                interval.tick().await;
                if !std::path::Path::new(&pid_path).exists() {
                    // Process is gone. Without exit-status (Linux drops it after parent's
                    // SIGCHLD handler), we report "unknown" by default. agent_manager's
                    // wait()-based path can override this with the real status if it
                    // beats us to the punch (idempotent reducer).
                    let exit_reason = "unknown";
                    if let Err(e) = port
                        .worker_process_record_exit(&process_id_clone, exit_reason)
                        .await
                    {
                        warn!("worker_process_record_exit({}) failed: {}", process_id_clone, e);
                    } else {
                        info!(
                            "supervisor: worker {} (pid {}) of pool {} exited",
                            process_id_clone, pid_u, pool_id_clone
                        );
                    }
                    break;
                }
            }
        });
        Ok(())
    }

    /// Surface a crash-loop as an operator priority-2 inbox notification (ADR-060).
    async fn handle_crash_loop(&self, pool_id: &str, payload: &str) -> Result<(), String> {
        let port = self
            .state
            .state_port
            .as_ref()
            .ok_or("state_port not initialized")?;

        let msg = format!(
            "Supervisor: pool '{}' entered CRASH LOOP and is paused. {}",
            pool_id, payload
        );
        warn!("{}", msg);

        // Notify all agents in the project — this is a global concern.
        // Project_id "*" is the supervisor's own scope; nexus consumers
        // surface ALL priority-2 inbox entries on the next operator poll.
        if let Err(e) = port
            .inbox_notify_all(
                "*",
                2, // priority 2 = override per ADR-060
                "supervisor_crash_loop",
                &msg,
            )
            .await
        {
            return Err(format!("inbox_notify_all: {}", e));
        }
        Ok(())
    }
}
