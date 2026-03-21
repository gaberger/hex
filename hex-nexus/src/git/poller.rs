//! Background git status poller (ADR-044 Phase 2).
//!
//! Polls git status for each registered project every N seconds.
//! Broadcasts WebSocket events only when state changes (branch, dirty count).

use std::collections::HashMap;

use tokio::sync::broadcast;

use crate::state::{SharedState, WsEnvelope};

/// Cached git snapshot for change detection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GitSnapshot {
    branch: String,
    head_sha: String,
    dirty_count: usize,
    staged_count: usize,
    untracked_count: usize,
}

/// Spawn the background git poller task.
/// Polls every `interval_secs` (default 10s) and broadcasts changes.
pub fn spawn_git_poller(state: SharedState, interval_secs: u64) {
    let interval = std::time::Duration::from_secs(interval_secs);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        let mut snapshots: HashMap<String, GitSnapshot> = HashMap::new();

        loop {
            ticker.tick().await;

            // Snapshot current project list
            let project_list: Vec<(String, String)> = {
                let projects = state.projects.read().await;
                projects
                    .values()
                    .map(|p| (p.id.clone(), p.root_path.clone()))
                    .collect()
            };

            for (project_id, root_path) in project_list {
                let rp = root_path.clone();
                let status_result = tokio::task::spawn_blocking(move || {
                    super::status::get_status(std::path::Path::new(&rp))
                })
                .await;

                let status = match status_result {
                    Ok(Ok(s)) => s,
                    _ => continue, // Skip projects that fail (not git repos, etc.)
                };

                let new_snapshot = GitSnapshot {
                    branch: status.branch.clone(),
                    head_sha: status.head_sha.clone(),
                    dirty_count: status.dirty_count,
                    staged_count: status.staged_count,
                    untracked_count: status.untracked_count,
                };

                let changed = snapshots
                    .get(&project_id)
                    .map_or(true, |old| *old != new_snapshot);

                if changed {
                    let prev = snapshots.insert(project_id.clone(), new_snapshot);

                    // Determine event type
                    let event = if prev.as_ref().map_or(false, |p| p.head_sha != status.head_sha) {
                        "commit-pushed"
                    } else if prev.as_ref().map_or(false, |p| p.branch != status.branch) {
                        "branch-switched"
                    } else {
                        "status-changed"
                    };

                    broadcast_git_event(
                        &state.ws_tx,
                        &project_id,
                        event,
                        serde_json::json!({
                            "branch": status.branch,
                            "headSha": &status.head_sha[..7.min(status.head_sha.len())],
                            "dirty": status.dirty_count,
                            "staged": status.staged_count,
                            "untracked": status.untracked_count,
                        }),
                    );
                }
            }

            // Remove snapshots for unregistered projects
            let current_ids: std::collections::HashSet<String> = {
                let projects = state.projects.read().await;
                projects.keys().cloned().collect()
            };
            snapshots.retain(|id, _| current_ids.contains(id));
        }
    });
}

fn broadcast_git_event(
    ws_tx: &broadcast::Sender<WsEnvelope>,
    project_id: &str,
    event: &str,
    data: serde_json::Value,
) {
    let _ = ws_tx.send(WsEnvelope {
        topic: format!("project:{}:git", project_id),
        event: event.to_string(),
        data,
    });
}
