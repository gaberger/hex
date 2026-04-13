//! Agent heartbeat timeout and task reclamation for HexFlo.
//!
//! Delegates to IStatePort — works with both SQLite and SpacetimeDB backends.

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
const STALE_THRESHOLD_SECS: u64 = 45;
const DEAD_THRESHOLD_SECS: u64 = 120;

impl HexFlo {
    /// Mark agents as stale (45s no heartbeat) or dead (120s).
    /// Reclaim tasks assigned to dead agents by resetting them to "pending".
    pub async fn cleanup_stale_agents(&self) -> Result<CleanupReport, String> {
        // Run health check on agent_manager if available
        if let Some(ref mgr) = self.agent_manager {
            let _ = mgr.check_health().await;
        }

        let report = self.state
            .swarm_cleanup_stale(STALE_THRESHOLD_SECS, DEAD_THRESHOLD_SECS)
            .await
            .map_err(|e| e.to_string())?;

        let result = CleanupReport::from_state_report(report)?;

        // ADR-2604131500 P0.2: briefing events for agent death/stale detection
        if result.dead_count > 0 {
            self.log_briefing_event(
                "critical",
                "swarm",
                &format!("{} agent(s) declared dead", result.dead_count),
                &format!(
                    "{} dead, {} stale, {} tasks reclaimed",
                    result.dead_count, result.stale_count, result.reclaimed_tasks
                ),
            )
            .await;
        } else if result.stale_count > 0 {
            self.log_briefing_event(
                "notable",
                "swarm",
                &format!("{} agent(s) went stale", result.stale_count),
                &format!(
                    "{} stale agents detected (no heartbeat for {}s)",
                    result.stale_count, STALE_THRESHOLD_SECS
                ),
            )
            .await;
        }

        Ok(result)
    }
}

impl CleanupReport {
    fn from_state_report(
        r: crate::ports::state::CleanupReport,
    ) -> Result<Self, String> {
        Ok(Self {
            stale_count: r.stale_count,
            dead_count: r.dead_count,
            reclaimed_tasks: r.reclaimed_tasks,
        })
    }
}
