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

        CleanupReport::from_state_report(report)
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

/// Result of a single worktree cleanup attempt during task completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCleanupResult {
    pub branch: String,
    pub worktree_path: String,
    /// "removed" if merged and cleaned, "pending-merge" if stored for later.
    pub action: String,
}

/// Aggregate result from a batch worktree cleanup pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCleanupReport {
    pub removed: u32,
    pub pending_merge: u32,
    pub details: Vec<WorktreeCleanupResult>,
}

impl HexFlo {
    /// Auto-cleanup worktrees for completed tasks.
    ///
    /// Scans all worktrees and removes any whose branch is merged into main.
    /// Worktrees with unmerged branches are stored in HexFlo memory as
    /// `pending-merge:{branch}` for later manual merge or dashboard surfacing.
    pub async fn cleanup_completed_worktrees(
        &self,
        repo_root: &std::path::Path,
    ) -> Result<WorktreeCleanupReport, String> {
        let worktrees = crate::git::worktree::list_worktrees(repo_root)?;
        let mut report = WorktreeCleanupReport {
            removed: 0,
            pending_merge: 0,
            details: Vec::new(),
        };

        for wt in &worktrees {
            if wt.is_main || wt.is_bare || wt.branch.is_empty() || wt.branch == "(detached)" {
                continue;
            }

            match self
                .cleanup_single_worktree(repo_root, &wt.path, &wt.branch)
                .await
            {
                Ok(result) => {
                    match result.action.as_str() {
                        "removed" => report.removed += 1,
                        "pending-merge" => report.pending_merge += 1,
                        _ => {}
                    }
                    report.details.push(result);
                }
                Err(e) => {
                    tracing::warn!("Failed to cleanup worktree {}: {}", wt.path, e);
                }
            }
        }

        Ok(report)
    }

    /// Cleanup a single worktree after its task completes.
    ///
    /// If the branch is merged into main: removes worktree + deletes branch.
    /// If not merged: stores a `pending-merge:{branch}` entry in HexFlo memory
    /// so the developer can be reminded to merge manually.
    async fn cleanup_single_worktree(
        &self,
        repo_root: &std::path::Path,
        worktree_path: &str,
        branch: &str,
    ) -> Result<WorktreeCleanupResult, String> {
        let merged = crate::git::worktree::is_branch_merged(repo_root, branch)?;

        if merged {
            crate::git::worktree::remove_worktree(repo_root, worktree_path, false, true)?;
            tracing::info!(
                "Auto-cleaned worktree: {} (branch: {} merged)",
                worktree_path,
                branch
            );
            Ok(WorktreeCleanupResult {
                branch: branch.to_string(),
                worktree_path: worktree_path.to_string(),
                action: "removed".to_string(),
            })
        } else {
            // Store as pending-merge so it surfaces in briefings / dashboard
            let key = format!("pending-merge:{}", branch);
            let value = serde_json::json!({
                "branch": branch,
                "worktree_path": worktree_path,
                "detected_at": chrono::Utc::now().to_rfc3339(),
            })
            .to_string();

            if let Err(e) = self.memory_store(&key, &value, Some("global")).await {
                tracing::warn!("Failed to store pending-merge for {}: {}", branch, e);
            }

            tracing::info!(
                "Worktree {} (branch: {}) not merged — stored as pending-merge",
                worktree_path,
                branch
            );

            Ok(WorktreeCleanupResult {
                branch: branch.to_string(),
                worktree_path: worktree_path.to_string(),
                action: "pending-merge".to_string(),
            })
        }
    }

    /// Auto-cleanup a specific task's worktree after task completion.
    ///
    /// Called from `task_complete` — finds the worktree whose branch matches
    /// the task's branch pattern and cleans it up. Returns `None` if no
    /// matching worktree was found (task wasn't in a worktree).
    pub async fn cleanup_task_worktree(
        &self,
        repo_root: &std::path::Path,
        task_branch: &str,
    ) -> Result<Option<WorktreeCleanupResult>, String> {
        if task_branch.is_empty() {
            return Ok(None);
        }

        let worktrees = crate::git::worktree::list_worktrees(repo_root)?;
        let matching = worktrees
            .iter()
            .find(|wt| wt.branch == task_branch && !wt.is_main && !wt.is_bare);

        match matching {
            Some(wt) => {
                let result = self
                    .cleanup_single_worktree(repo_root, &wt.path, &wt.branch)
                    .await?;
                Ok(Some(result))
            }
            None => Ok(None),
        }
    }
}
