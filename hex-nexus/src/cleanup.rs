use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{error, info, debug, warn};

use crate::state::SharedState;

/// Cleanup service that runs periodically to remove stale coordination instances
pub struct CleanupService {
    state: SharedState,
    interval: Duration,
}

impl CleanupService {
    /// Spawn the cleanup service as a background tokio task
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        let service = Self {
            state,
            interval: Duration::from_secs(60), // Run every 60 seconds
        };

        tokio::spawn(async move {
            service.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(self.interval);
        info!("Cleanup service started (interval: {}s)", self.interval.as_secs());

        // Snapshot own binary mtime at startup for update detection (ADR-060)
        let binary_path = std::env::current_exe().ok();
        let mut last_binary_mtime = binary_path.as_ref()
            .and_then(|p| p.metadata().ok())
            .and_then(|m| m.modified().ok());

        // ADR-060 step 9: Track last escalation attempt per agent for rate limiting
        let mut last_escalation: HashMap<String, Instant> = HashMap::new();
        let mut escalation_failures: HashMap<String, u32> = HashMap::new();

        loop {
            interval.tick().await;

            // ── Stale session cleanup ───────────────────────
            match cleanup_stale_sessions(&self.state).await {
                Ok(removed) if removed > 0 => {
                    info!("Cleaned up {} stale sessions", removed);
                }
                Ok(_) => {
                    debug!("No stale sessions to clean up");
                }
                Err(e) => {
                    error!("Cleanup failed: {}", e);
                }
            }

            // ── hex_agent cleanup (ADR-058 unified registry) ──
            // Step 1: mark online/idle agents as stale/dead based on heartbeat age
            // Step 2: evict dead agents (removes rows older than 1h)
            if let Some(sp) = &self.state.state_port {
                if let Err(e) = sp.hex_agent_mark_inactive().await {
                    debug!("hex_agent mark_inactive skipped: {}", e);
                }
                if let Err(e) = sp.hex_agent_evict_dead().await {
                    debug!("hex_agent evict_dead skipped: {}", e);
                } else {
                    debug!("hex_agent cleanup cycle completed");
                }
            }

            // ── Swarm agent cleanup (HexFlo swarm_agent table) ──
            if let Some(sp) = &self.state.state_port {
                match sp.swarm_cleanup_stale(45, 120).await {
                    Ok(report) if report.stale_count > 0 || report.dead_count > 0 => {
                        info!(
                            "Swarm agent cleanup: {} stale, {} dead, {} tasks reclaimed",
                            report.stale_count, report.dead_count, report.reclaimed_tasks
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        debug!("Swarm agent cleanup skipped: {}", e);
                    }
                }
            }

            // ── Inbox expiry (ADR-060) ──────────────────────
            if let Some(sp) = &self.state.state_port {
                if let Err(e) = sp.inbox_expire(86400).await {
                    debug!("Inbox expiry failed: {}", e);
                }
            }

            // ── Decision auto-resolution (ADR-2604131500 P3.3, P1.2) ──
            // Auto-acknowledge unresolved decision notifications past the
            // configurable deadline. Reads from env / .hex/project.json / default 2h.
            if let Some(sp) = &self.state.state_port {
                match sp.inbox_query("*", None, true).await {
                    Ok(notifications) => {
                        let now = chrono::Utc::now();
                        let deadline_secs = crate::state_config::resolve_decision_deadline_secs() as i64;
                        let mut auto_resolved = 0u32;
                        for notif in &notifications {
                            if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&notif.created_at) {
                                let age = (now - created.with_timezone(&chrono::Utc)).num_seconds();
                                if age > deadline_secs {
                                    if let Err(e) = sp.inbox_acknowledge(notif.id, "auto-expiry").await {
                                        debug!("Decision auto-resolve failed for {}: {}", notif.id, e);
                                    } else {
                                        auto_resolved += 1;
                                    }
                                }
                            }
                        }
                        if auto_resolved > 0 {
                            let hours = deadline_secs as f64 / 3600.0;
                            info!(
                                "Decision auto-resolution: {} decision(s) past {:.1}h deadline auto-resolved",
                                auto_resolved, hours
                            );
                        }
                    }
                    Err(e) => {
                        debug!("Decision auto-resolution query failed: {}", e);
                    }
                }
            }

            // ── Idle agent escalation (ADR-060 step 9) ──────
            if let Some(sp) = &self.state.state_port {
                escalate_idle_agents(sp.as_ref(), &mut last_escalation, &mut escalation_failures).await;
            }

            // ── Binary update detection (ADR-060) ───────────
            if let Some(ref bp) = binary_path {
                if let Ok(meta) = bp.metadata() {
                    if let Ok(current_mtime) = meta.modified() {
                        if let Some(prev_mtime) = last_binary_mtime {
                            if current_mtime != prev_mtime {
                                info!("Binary update detected — notifying all agents");
                                last_binary_mtime = Some(current_mtime);
                                if let Some(sp) = &self.state.state_port {
                                    let project_id = std::env::current_dir()
                                        .map(|p| crate::state::make_project_id(&p.to_string_lossy()))
                                        .unwrap_or_default();
                                    let payload = serde_json::json!({
                                        "reason": "hex-nexus binary updated",
                                        "binary": bp.to_string_lossy(),
                                    }).to_string();
                                    if let Err(e) = sp.inbox_notify_all(&project_id, 2, "restart", &payload).await {
                                        error!("Failed to notify agents of binary update: {}", e);
                                    }
                                }
                            }
                        } else {
                            last_binary_mtime = Some(current_mtime);
                        }
                    }
                }
            }
        }
    }
}

/// ADR-060 step 9: Escalate unacked critical notifications to idle agents.
///
/// Checks for priority-2 notifications older than 60s where the target agent's
/// heartbeat is also >60s stale. For each, attempts `claude --resume <session_id>`
/// to wake the agent. Rate-limited to 1 resume per agent per 5 minutes.
/// After 3 consecutive failures, marks the agent as dead.
async fn escalate_idle_agents(
    sp: &dyn crate::ports::state::IStatePort,
    last_escalation: &mut HashMap<String, Instant>,
    escalation_failures: &mut HashMap<String, u32>,
) {
    // Get all agents to find stale ones with session IDs
    let agents = match sp.hex_agent_list().await {
        Ok(a) => a,
        Err(_) => return,
    };

    let now = chrono::Utc::now();
    let rate_limit = Duration::from_secs(300); // 5 minutes

    for agent in &agents {
        let agent_id = match agent["id"].as_str() {
            Some(id) => id.to_string(),
            None => continue,
        };

        let session_id = match agent["session_id"].as_str() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue, // No session to resume
        };

        let status = agent["status"].as_str().unwrap_or("unknown");
        // Only escalate to stale or idle agents — online agents see notifications via hook
        if status != "stale" && status != "idle" {
            continue;
        }

        // Rate limit: skip if we escalated this agent recently
        if let Some(last) = last_escalation.get(&agent_id) {
            if last.elapsed() < rate_limit {
                continue;
            }
        }

        // Check for unacked priority-2 notifications for this agent
        let notifications = match sp
            .inbox_query(&agent_id, Some(2), true)
            .await
        {
            Ok(n) => n,
            Err(_) => continue,
        };

        if notifications.is_empty() {
            continue;
        }

        // Check if the oldest unacked notification is >60s old
        let oldest_created = notifications
            .iter()
            .filter_map(|n| chrono::DateTime::parse_from_rfc3339(&n.created_at).ok())
            .min();

        let is_old_enough = oldest_created
            .map(|created| (now - created.with_timezone(&chrono::Utc)).num_seconds() > 60)
            .unwrap_or(false);

        if !is_old_enough {
            continue;
        }

        // Check failure count — give up after 3 consecutive failures
        let failures = escalation_failures.get(&agent_id).copied().unwrap_or(0);
        if failures >= 3 {
            warn!(
                agent_id = %agent_id,
                "Giving up on escalation after 3 failures — agent may be unreachable"
            );
            continue;
        }

        // Attempt to resume the agent's Claude Code session
        info!(
            agent_id = %agent_id,
            session_id = %session_id,
            unacked = notifications.len(),
            "Escalating: resuming idle agent with unacked critical notifications"
        );

        last_escalation.insert(agent_id.clone(), Instant::now());

        let resume_result = tokio::process::Command::new("claude")
            .args([
                "--resume",
                &session_id,
                "-p",
                "Check your hex inbox for critical notifications: hex inbox list",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        match resume_result {
            Ok(_child) => {
                info!(agent_id = %agent_id, "Resume command dispatched");
                escalation_failures.remove(&agent_id);
            }
            Err(e) => {
                warn!(
                    agent_id = %agent_id,
                    error = %e,
                    "Failed to resume agent session"
                );
                *escalation_failures.entry(agent_id).or_insert(0) += 1;
            }
        }
    }
}

/// Clean up stale coordination sessions via IStatePort.
///
/// Delegates to the state port's coordination_cleanup_stale method,
/// which handles instance eviction, lock release, claim release,
/// and unstaged state removal.
///
/// Returns the number of sessions removed.
pub async fn cleanup_stale_sessions(state: &SharedState) -> Result<usize, Box<dyn std::error::Error>> {
    let sp = state.state_port.as_ref()
        .ok_or("State port not configured")?;

    let report = sp.coordination_cleanup_stale(360).await
        .map_err(|e| -> Box<dyn std::error::Error> { Box::new(e) })?;

    Ok(report.instances_removed)
}

#[cfg(test)]
mod tests {
    use super::is_pid_alive;

    #[test]
    fn test_is_pid_alive_current_process() {
        // Current process should always be alive
        let pid = std::process::id();
        assert!(is_pid_alive(pid));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_pid_alive_invalid_pid() {
        // PID 99999 should not exist (unless system has 100k+ processes)
        assert!(!is_pid_alive(99999));
    }
}

/// Check if a process with the given PID is alive
///
/// On Unix: Uses kill(pid, 0) via libc which returns success if process exists
/// On Windows: Always returns true (requires sysinfo crate for proper check)
#[allow(dead_code)]
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // Use libc kill(pid, 0) - returns 0 if process exists
        unsafe {
            libc::kill(pid as i32, 0) == 0
        }
    }

    #[cfg(not(unix))]
    {
        // Windows: Would need sysinfo crate or WinAPI
        // For now, assume alive (heartbeat timeout will catch it)
        true
    }
}
