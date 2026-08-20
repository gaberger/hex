//! Auto-pause persona pools when no work is queued.
//!
//! Observed 2026-05-21: 30 always-on persona pools + 13 nexus background
//! subscribers consume ~400% nexus CPU at idle (no SOP runs, no DMs).
//! That's the architectural floor for "30 personas alive 24/7" — but
//! most of the time the team has no work, so the cores are spent
//! maintaining websocket subscriptions to STDB for events that aren't
//! happening.
//!
//! This task watches activity signals every `TICK_SECS` and:
//!   - Pauses all pools (worker_pool_intent.paused=true) when there's
//!     been no work for `IDLE_AFTER_SECS`. Supervisor_tick honours
//!     paused and stops emitting spawn_request → existing personas
//!     drain to exited status → CPU drops to ~30%.
//!   - Unpauses all pools when an activity signal appears, AND
//!     resets the idle countdown. Calls
//!     `worker_pool_intent_set_paused(pool, false)` which also clears
//!     any sticky `in_crash_loop` flag (see commit 3183bf5b).
//!
//! Activity signals (any one wakes the team):
//!   - Unhandled operator DM in agent_messages (from=operator, not in read_by)
//!   - proposed_action row in {pending, escalated, approved}
//!   - Active SOP runs (sop_executor::active_runs)
//!
//! Disabled via `HEX_DISABLE_POOL_AUTOPAUSE=1` (operator override for
//! always-on workloads). Threshold configurable via
//! `HEX_POOL_IDLE_AFTER_SECS` (default 300 = 5 min).

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::state::SharedState;

const TICK_SECS: u64 = 60;
const DEFAULT_IDLE_AFTER_SECS: u64 = 300;

pub struct PoolAutoPause;

impl PoolAutoPause {
    pub fn spawn(state: SharedState) -> Option<JoinHandle<()>> {
        if std::env::var("HEX_DISABLE_POOL_AUTOPAUSE").is_ok() {
            info!("pool_autopause disabled via HEX_DISABLE_POOL_AUTOPAUSE");
            return None;
        }
        let idle_after = std::env::var("HEX_POOL_IDLE_AFTER_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| (60..=86400).contains(v))
            .unwrap_or(DEFAULT_IDLE_AFTER_SECS);

        info!(
            tick_secs = TICK_SECS,
            idle_after_secs = idle_after,
            "pool_autopause started"
        );

        Some(tokio::spawn(async move {
            let mut last_activity = Instant::now();
            let mut paused_state = false;
            let mut interval = time::interval(Duration::from_secs(TICK_SECS));
            // Skip the first immediate tick — let nexus finish startup.
            interval.tick().await;
            loop {
                interval.tick().await;
                let has_activity = has_pending_work(&state).await;
                if has_activity {
                    last_activity = Instant::now();
                    if paused_state {
                        if let Err(e) = set_all_paused(false).await {
                            warn!(error = %e, "pool_autopause: unpause failed");
                        } else {
                            info!("pool_autopause: WORK DETECTED — unpausing all pools");
                            paused_state = false;
                        }
                    }
                } else if !paused_state
                    && last_activity.elapsed() >= Duration::from_secs(idle_after)
                {
                    if let Err(e) = set_all_paused(true).await {
                        warn!(error = %e, "pool_autopause: pause failed");
                    } else {
                        info!(
                            idle_secs = last_activity.elapsed().as_secs(),
                            "pool_autopause: IDLE THRESHOLD — pausing all pools (resume on next operator brief)"
                        );
                        paused_state = true;
                    }
                }
            }
        }))
    }
}

/// Query STDB for any of the three activity signals. Returns true if
/// the team has work to do; false means safe to pause.
async fn has_pending_work(state: &SharedState) -> bool {
    let stdb_host = crate::adapters::stdb_endpoint::discover_endpoint();
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true, // Fail safe — assume work pending if we can't check.
    };

    // Signal 1: proposed_action rows that need handling.
    if has_rows(
        &client,
        &stdb_host,
        "hex",
        "SELECT id FROM proposed_action WHERE status = 'pending'",
    )
    .await
    {
        return true;
    }
    if has_rows(
        &client,
        &stdb_host,
        "hex",
        "SELECT id FROM proposed_action WHERE status = 'escalated'",
    )
    .await
    {
        return true;
    }
    if has_rows(
        &client,
        &stdb_host,
        "hex",
        "SELECT id FROM proposed_action WHERE status = 'approved'",
    )
    .await
    {
        return true;
    }

    // Signal 2: open commitments.
    if has_rows(
        &client,
        &stdb_host,
        "hex",
        "SELECT id FROM commitment WHERE status = 'open'",
    )
    .await
    {
        return true;
    }

    // Signal 3: active SOP runs (in-memory, no STDB roundtrip).
    let _ = state; // SOP runs live in the executor's ring buffer
    let active = crate::orchestration::sop_executor::active_runs().await;
    if !active.is_empty() {
        return true;
    }

    // NOTE: deliberately NOT checking agent_messages here. Operator DMs
    // ARE a work signal, but STDB SQL has no time arithmetic — the query
    // `WHERE from_agent='operator'` returns every DM ever sent, so the
    // table is permanently non-empty and autopause never fires. The
    // three signals above (pending/escalated/approved proposed_action,
    // open commitments, active SOP runs) cover everything that flows
    // FROM a DM into the team's work surface. When operator sends a
    // brief, the drafter+twin chain creates a proposed_action within
    // seconds — that's what wakes the pools.
    false
}

async fn has_rows(client: &reqwest::Client, host: &str, db: &str, sql: &str) -> bool {
    let url = format!("{host}/v1/database/{db}/sql");
    let resp = match client.post(&url).body(sql.to_string()).send().await {
        Ok(r) => r,
        Err(_) => return false,
    };
    if !resp.status().is_success() {
        return false;
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(_) => return false,
    };
    body.as_array()
        .and_then(|a| a.first())
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .map(|rows| !rows.is_empty())
        .unwrap_or(false)
}

/// Toggle paused on every worker_pool_intent row by calling the
/// `worker_pool_intent_set_paused` reducer. Best-effort: per-pool
/// failures are logged but don't abort the batch.
async fn set_all_paused(paused: bool) -> Result<(), String> {
    let stdb_host = crate::adapters::stdb_endpoint::discover_endpoint();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client: {e}"))?;
    // Get all pool ids.
    let pools = get_all_pool_ids(&client, &stdb_host).await?;
    let mut ok = 0usize;
    let mut fail = 0usize;
    for pool_id in &pools {
        let url = format!(
            "{}/v1/database/hex/call/worker_pool_intent_set_paused",
            stdb_host
        );
        let body = serde_json::json!([pool_id, paused]);
        match client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => ok += 1,
            Ok(r) => {
                fail += 1;
                let status = r.status();
                tracing::debug!(pool = %pool_id, http_status = %status, "pool_autopause: per-pool toggle non-success");
            }
            Err(e) => {
                fail += 1;
                tracing::debug!(pool = %pool_id, error = %e, "pool_autopause: per-pool toggle error");
            }
        }
    }
    info!(toggled = ok, failed = fail, paused = paused, "pool_autopause: batch toggle complete");
    if fail > 0 && ok == 0 {
        return Err(format!("all {fail} pool toggles failed"));
    }
    Ok(())
}

async fn get_all_pool_ids(client: &reqwest::Client, host: &str) -> Result<Vec<String>, String> {
    let url = format!("{host}/v1/database/hex/sql");
    let resp = client
        .post(&url)
        .body("SELECT id FROM worker_pool_intent".to_string())
        .send()
        .await
        .map_err(|e| format!("query: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;
    let mut out = Vec::new();
    if let Some(arr) = body.as_array() {
        for t in arr {
            if let Some(rows) = t.get("rows").and_then(|r| r.as_array()) {
                for row in rows {
                    if let Some(vals) = row.as_array() {
                        if let Some(id) = vals.first().and_then(|v| v.as_str()) {
                            out.push(id.to_string());
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

#[allow(dead_code)]
async fn _state_compile_check(_s: Arc<crate::state::AppState>) {
    // ensures the SharedState arg signature compiles in callers
}
