use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{error, info, debug};

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

            // ── Inbox expiry (ADR-060) ──────────────────────
            if let Some(sp) = &self.state.state_port {
                if let Err(e) = sp.inbox_expire(86400).await {
                    debug!("Inbox expiry failed: {}", e);
                }
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
                                    let project_id = std::env::var("HEX_PROJECT_ID").unwrap_or_default();
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
