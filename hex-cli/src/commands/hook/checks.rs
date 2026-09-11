//! Pre-flight checks the `route` hook runs on every interaction.
//!
//! These lived inside the `sched` verb — the scheduler daemon's command
//! module — and were imported from it by the hook. `sched` is gone with the
//! daemon (ADR-2608241500 P6.1), but the checks are not daemon work: each one
//! reads the local repository and reports what it finds. They moved here
//! unchanged rather than being deleted with their old home.
//!
//! `check_mcp_cli_parity` did NOT come across. It compared the CLI's verbs
//! against the MCP server's tool list, and the MCP server is deleted.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum FreshnessStatus {
    /// Binary is newer than or equal to the latest commit — no rebuild needed.
    Fresh,
    /// Binary is older than the latest commit — background rebuild spawned.
    Stale { binary_age_secs: u64, commit_age_secs: u64 },
    /// Binary does not exist at the expected path (never built).
    Missing,
    /// Could not determine freshness (git not available, etc.).
    Unknown(String),
}

/// Summary of a single workplan's reconciliation status.
#[derive(Debug)]
pub(crate) struct WorkplanSummary {
    pub(crate) id: String,
    pub(crate) feature: String,
    pub(crate) status: String,
    pub(crate) total_tasks: usize,
    pub(crate) done_tasks: usize,
    /// Tasks still marked "todo" but with git evidence (commit messages mentioning the task id).
    pub(crate) stale_tasks: Vec<String>,
    /// Path to the workplan JSON file (needed for auto-fix writes).
    pub(crate) path: PathBuf,
}

/// A stale git worktree detected during validation.
#[derive(Debug)]
pub(crate) struct StaleWorktree {
    pub(crate) path: String,
    pub(crate) branch: String,
    /// Seconds since the last commit on this worktree's branch.
    pub(crate) age_secs: u64,
}

/// Compare `target/release/hex` mtime against `git log -1 --format=%ct HEAD`.
/// If the binary is older, spawn `cargo build --release` in the background and
/// return [`FreshnessStatus::Stale`].
pub fn check_binary_freshness() -> FreshnessStatus {
    // Locate binary relative to the workspace root (one level up from hex-cli/).
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let binary_path = workspace_root.join("target/release/hex");

    // 1. Stat the binary
    let binary_mtime = match std::fs::metadata(&binary_path) {
        Ok(meta) => match meta.modified() {
            Ok(t) => t,
            Err(e) => return FreshnessStatus::Unknown(format!("mtime error: {e}")),
        },
        Err(_) => return FreshnessStatus::Missing,
    };

    // 2. Get latest commit timestamp via git
    let git_output = match std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct", "HEAD"])
        .current_dir(&workspace_root)
        .output()
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            return FreshnessStatus::Unknown(format!(
                "git exited {}",
                o.status.code().unwrap_or(-1)
            ))
        }
        Err(e) => return FreshnessStatus::Unknown(format!("git not available: {e}")),
    };

    let commit_ts: u64 = match String::from_utf8_lossy(&git_output.stdout)
        .trim()
        .parse()
    {
        Ok(ts) => ts,
        Err(e) => return FreshnessStatus::Unknown(format!("parse commit ts: {e}")),
    };

    // 3. Convert binary mtime to epoch seconds
    let binary_epoch = binary_mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // 4. Compare
    if binary_epoch >= commit_ts {
        return FreshnessStatus::Fresh;
    }

    // 5. Stale — spawn background rebuild
    let _ = std::process::Command::new("cargo")
        .args(["build", "--release", "-p", "hex-cli"])
        .current_dir(&workspace_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn(); // fire-and-forget

    FreshnessStatus::Stale {
        binary_age_secs: binary_epoch,
        commit_age_secs: commit_ts,
    }
}

/// Scan `docs/workplans/*.json` for active (non-completed) workplans, reconcile
/// each task against repo evidence, and return per-workplan summaries.
///
/// A pending task is "stale" (reconcilable to done) when ALL three hold:
///   1. Every file declared in `task.files[]` exists on disk
///   2. Any symbols declared in `task.name` (struct/enum/trait/fn/impl Names)
///      are found in those files
///   3. At least one git commit touches one of those files AND its message
///      references BOTH the task id (e.g. `P1.2`) AND the workplan id
///
/// Replaces the prior substring-match heuristic which produced ~70% false
/// positives by treating any commit mentioning a generic task id like `P1.1`
/// as evidence for every workplan's `P1.1` (ADR-2026-04-14-2201 closes this).
pub(crate) fn check_workplan_status() -> anyhow::Result<Vec<WorkplanSummary>> {
    use crate::commands::plan::reconcile_evidence::{
        collect_evidence_strict, verify, VerifyResult, WorkplanTask,
    };

    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let workplans_dir = workspace_root.join("docs/workplans");

    if !workplans_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut summaries = Vec::new();

    for entry in std::fs::read_dir(&workplans_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let wp: serde_json::Value = match serde_json::from_str(&contents) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let wp_status = wp.get("status").and_then(|s| s.as_str()).unwrap_or("unknown");
        if wp_status == "completed" {
            continue;
        }

        let id = wp.get("id").and_then(|s| s.as_str()).unwrap_or("unknown").to_string();
        let feature = wp.get("feature").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let adr_scope = wp.get("adr").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let created_at = wp
            .get("created_at")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        let mut total_tasks = 0usize;
        let mut done_tasks = 0usize;
        let mut stale_tasks = Vec::new();

        if let Some(phases) = wp.get("phases").and_then(|p| p.as_array()) {
            for phase in phases {
                if let Some(tasks) = phase.get("tasks").and_then(|t| t.as_array()) {
                    for task in tasks {
                        total_tasks += 1;
                        let task_status = task
                            .get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or("todo");
                        if task_status == "done" {
                            done_tasks += 1;
                            continue;
                        }

                        let task_id = task
                            .get("id")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        if task_id.is_empty() {
                            continue;
                        }

                        let description = task
                            .get("name")
                            .and_then(|s| s.as_str())
                            .or_else(|| task.get("description").and_then(|s| s.as_str()))
                            .unwrap_or("")
                            .to_string();
                        let files: Vec<String> = task
                            .get("files")
                            .and_then(|f| f.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();

                        // No files declared — strict verify requires at least one,
                        // so this task is not reconcilable without explicit evidence.
                        if files.is_empty() {
                            continue;
                        }

                        let wp_task = WorkplanTask {
                            id: task_id.clone(),
                            description,
                            files,
                            done_command: String::new(),
                            created_at: created_at.clone(),
                            adr_scope: adr_scope.clone(),
                        };

                        let evidence =
                            collect_evidence_strict(&wp_task, &workspace_root, "", Some(&id));
                        if matches!(verify(&evidence), VerifyResult::Promote) {
                            stale_tasks.push(task_id);
                        }
                    }
                }
            }
        }

        summaries.push(WorkplanSummary {
            id,
            feature,
            status: wp_status.to_string(),
            total_tasks,
            done_tasks,
            stale_tasks,
            path: path.clone(),
        });
    }

    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(summaries)
}

/// Detect git worktrees that have had no commits for over 24 hours.
pub(crate) fn check_stale_worktrees() -> anyhow::Result<Vec<StaleWorktree>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&workspace_root)
        .output()?;

    if !output.status.success() {
        return Ok(vec![]);
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut stale = Vec::new();
    let mut current_path = String::new();
    let mut current_branch = String::new();

    for line in text.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = p.to_string();
            current_branch.clear();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            current_branch = b.to_string();
        } else if line.is_empty() && !current_path.is_empty() && !current_branch.is_empty() {
            // Skip the main worktree (no branch prefix pattern like feat/, hex/, worktree-, claude/)
            let is_feature_branch = current_branch.starts_with("feat/")
                || current_branch.starts_with("hex/")
                || current_branch.starts_with("worktree-")
                || current_branch.starts_with("claude/");
            if !is_feature_branch {
                current_path.clear();
                current_branch.clear();
                continue;
            }

            // Check last commit age on this branch
            let commit_ts = std::process::Command::new("git")
                .args(["log", "-1", "--format=%ct", &current_branch])
                .current_dir(&workspace_root)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .trim()
                        .parse::<u64>()
                        .ok()
                });

            if let Some(ts) = commit_ts {
                let age = now.saturating_sub(ts);
                // Stale if no commits for 24+ hours
                if age > 86400 {
                    stale.push(StaleWorktree {
                        path: current_path.clone(),
                        branch: current_branch.clone(),
                        age_secs: age,
                    });
                }
            }

            current_path.clear();
            current_branch.clear();
        }
    }

    Ok(stale)
}

/// Auto-fix: reconcile stale workplan tasks by marking them "done" in the JSON.
/// Returns the number of tasks fixed.
pub(crate) fn autofix_workplan(wp: &WorkplanSummary) -> anyhow::Result<usize> {
    if wp.stale_tasks.is_empty() {
        return Ok(0);
    }

    let contents = std::fs::read_to_string(&wp.path)?;
    let mut doc: serde_json::Value = serde_json::from_str(&contents)?;

    let stale_set: std::collections::HashSet<&str> =
        wp.stale_tasks.iter().map(|s| s.as_str()).collect();
    let mut fixed = 0usize;

    if let Some(phases) = doc.get_mut("phases").and_then(|p| p.as_array_mut()) {
        for phase in phases {
            if let Some(tasks) = phase.get_mut("tasks").and_then(|t| t.as_array_mut()) {
                for task in tasks {
                    let task_id = task
                        .get("id")
                        .and_then(|s| s.as_str())
                        .unwrap_or("");
                    if stale_set.contains(task_id) {
                        task["status"] = serde_json::Value::String("done".to_string());
                        fixed += 1;
                    }
                }
            }
        }
    }

    if fixed > 0 {
        let out = serde_json::to_string_pretty(&doc)?;
        std::fs::write(&wp.path, out)?;
    }

    Ok(fixed)
}
