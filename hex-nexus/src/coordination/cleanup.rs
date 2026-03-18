//! Agent heartbeat timeout and task reclamation for HexFlo.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::HexFlo;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub stale_count: u32,
    pub dead_count: u32,
    pub reclaimed_tasks: u32,
}

// ── Cleanup operations on HexFlo ───────────────────────

/// Thresholds for agent staleness.
const STALE_THRESHOLD_SECS: i64 = 45;
const DEAD_THRESHOLD_SECS: i64 = 120;

impl HexFlo {
    /// Mark agents as stale (45s no heartbeat) or dead (120s).
    /// Reclaim tasks assigned to dead agents by resetting them to "pending".
    pub async fn cleanup_stale_agents(&self) -> Result<CleanupReport, String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();

        // Also run health check on agent_manager if available
        if let Some(ref mgr) = self.agent_manager {
            let _ = mgr.check_health().await;
        }

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let now = chrono::Utc::now();
            let stale_cutoff =
                (now - chrono::Duration::seconds(STALE_THRESHOLD_SECS)).to_rfc3339();
            let dead_cutoff =
                (now - chrono::Duration::seconds(DEAD_THRESHOLD_SECS)).to_rfc3339();

            // Count agents that are running but have no recent heartbeat.
            // We use swarm_agents table — agents with status 'idle' or 'running'
            // whose swarm's updated_at is older than the thresholds.

            // Mark stale: agents idle/running in swarms not updated recently
            let stale_count = conn
                .execute(
                    "UPDATE swarm_agents SET status = 'stale'
                     WHERE status IN ('idle', 'running')
                     AND swarm_id IN (
                         SELECT id FROM swarms WHERE updated_at < ?1 AND status = 'active'
                     )",
                    params![stale_cutoff],
                )
                .map_err(|e| e.to_string())? as u32;

            // Mark dead: agents that have been stale and swarm not updated for even longer
            let dead_count = conn
                .execute(
                    "UPDATE swarm_agents SET status = 'dead'
                     WHERE status = 'stale'
                     AND swarm_id IN (
                         SELECT id FROM swarms WHERE updated_at < ?1 AND status = 'active'
                     )",
                    params![dead_cutoff],
                )
                .map_err(|e| e.to_string())? as u32;

            // Reclaim tasks assigned to dead agents — set back to pending
            let reclaimed_tasks = conn
                .execute(
                    "UPDATE swarm_tasks SET status = 'pending', agent_id = NULL
                     WHERE status = 'running'
                     AND agent_id IN (
                         SELECT id FROM swarm_agents WHERE status = 'dead'
                     )",
                    [],
                )
                .map_err(|e| e.to_string())? as u32;

            Ok(CleanupReport {
                stale_count,
                dead_count,
                reclaimed_tasks,
            })
        })
        .await
        .expect("spawn_blocking join")
    }
}
