use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{error, info, debug};
use chrono::{DateTime, Utc};

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

        loop {
            interval.tick().await;

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
        }
    }
}

/// Clean up stale coordination sessions
///
/// Process:
/// 1. Check each instance's last_seen timestamp
/// 2. Mark/remove instances with no heartbeat for 60s
/// 3. Validate PIDs - remove dead processes immediately
/// 4. Remove instances that have been stale for 5+ minutes
///
/// Returns the number of sessions removed
pub async fn cleanup_stale_sessions(state: &SharedState) -> Result<usize, Box<dyn std::error::Error>> {
    let now = Utc::now();
    let stale_threshold = now - chrono::Duration::seconds(60);
    let remove_threshold = now - chrono::Duration::seconds(360); // 6 minutes total

    let mut instances = state.instances.write().await;
    let mut to_remove = Vec::new();

    for (instance_id, inst) in instances.iter() {
        let last_seen = DateTime::parse_from_rfc3339(&inst.last_seen)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());

        // Check if PID is still alive
        if !is_pid_alive(inst.pid) {
            debug!("Marking instance {} for removal (dead PID: {})", instance_id, inst.pid);
            to_remove.push(instance_id.clone());
            continue;
        }

        // Check if instance is stale (no heartbeat for 60s)
        if last_seen < stale_threshold {
            // Check if it's been stale long enough to remove (5+ minutes)
            let registered_at = DateTime::parse_from_rfc3339(&inst.registered_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            // If registered more than 6 minutes ago and no recent heartbeat, remove
            if registered_at < remove_threshold && last_seen < stale_threshold {
                debug!("Marking instance {} for removal (stale since {})", instance_id, last_seen);
                to_remove.push(instance_id.clone());
            }
        }
    }

    // Remove stale instances
    let removed_count = to_remove.len();
    for instance_id in to_remove {
        instances.remove(&instance_id);

        // Also remove associated locks
        let mut locks = state.worktree_locks.write().await;
        locks.retain(|_, lock| lock.instance_id != instance_id);
        drop(locks);

        // Also remove associated task claims
        let mut claims = state.task_claims.write().await;
        claims.retain(|_, claim| claim.instance_id != instance_id);
        drop(claims);

        // Also remove unstaged files
        let mut unstaged = state.unstaged.write().await;
        unstaged.remove(&instance_id);
        drop(unstaged);
    }

    drop(instances);

    Ok(removed_count)
}

/// Check if a process with the given PID is alive
///
/// On Unix: Uses kill(pid, 0) via libc which returns success if process exists
/// On Windows: Always returns true (requires sysinfo crate for proper check)
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

#[cfg(test)]
mod tests {
    use super::*;

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
