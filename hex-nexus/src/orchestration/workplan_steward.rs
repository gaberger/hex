//! workplan-steward — an in-nexus hex agent for the workplan format + lifecycle.
//!
//! The reliable counterpart to the persona/SOP path, mirroring adr-steward: runs
//! DIRECTLY in nexus. It validates workplan JSON against the format (every code
//! step needs an evidence predicate) and reconciles status — a workplan whose
//! every step is done is advanced to `completed`. Commits the batch and records
//! itself to the shared agent-runs feed (GET /api/direct/runs → the dashboard).

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::direct_exec::record_agent_run;

#[derive(Debug, Serialize)]
pub struct WorkplanStewardResult {
    pub agent: String,
    pub action: String,
    pub scanned: usize,
    pub advanced: Vec<String>,
    pub format_issues: Vec<String>,
    pub dry_run: bool,
    pub committed: Option<String>,
    pub error: Option<String>,
}

/// Validate workplan format + reconcile status. `dry_run` reports without mutating.
pub async fn run_reconcile_sweep(dry_run: bool) -> WorkplanStewardResult {
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    let mut res = WorkplanStewardResult {
        agent: "workplan-steward".into(),
        action: "workplan-reconcile-sweep".into(),
        scanned: 0,
        advanced: Vec::new(),
        format_issues: Vec::new(),
        dry_run,
        committed: None,
        error: None,
    };

    let root = repo_root();
    let base = root.join("docs/workplans");
    let mut changed_any = false;

    for d in [base.clone(), base.join("drafts")] {
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = match std::fs::read_to_string(&p) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let mut j: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => {
                    res.format_issues.push(format!("{}: invalid JSON", fname(&p)));
                    continue;
                }
            };
            res.scanned += 1;
            let id = j.get("id").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| fname(&p));

            // FORMAT — required status field.
            if j.get("status").and_then(|v| v.as_str()).is_none() {
                res.format_issues.push(format!("{}: missing status", id));
            }

            // FORMAT + RECONCILE over steps.
            if let Some(steps) = j.get("steps").and_then(|v| v.as_array()).cloned() {
                // code step = has non-empty `files`; needs an evidence predicate.
                let bad = steps
                    .iter()
                    .filter(|s| {
                        let has_files = s
                            .get("files")
                            .and_then(|v| v.as_array())
                            .map(|a| !a.is_empty())
                            .unwrap_or(false);
                        let has_ev = s.get("verify").is_some()
                            || s.get("done_command").is_some()
                            || s.get("evidence").is_some();
                        has_files && !has_ev
                    })
                    .count();
                if bad > 0 {
                    res.format_issues
                        .push(format!("{}: {} code step(s) without an evidence predicate", id, bad));
                }

                // RECONCILE — every step done → workplan completed.
                let status = j.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let all_done = !steps.is_empty()
                    && steps.iter().all(|s| {
                        matches!(s.get("status").and_then(|v| v.as_str()).unwrap_or(""), "done" | "completed")
                    });
                if all_done && !matches!(status, "completed" | "done") {
                    res.advanced.push(id.clone());
                    if !dry_run {
                        j["status"] = Value::String("completed".into());
                        if let Ok(out) = serde_json::to_string_pretty(&j) {
                            if std::fs::write(&p, format!("{}\n", out)).is_ok() {
                                tracing::info!(workplan = %id, "workplan-steward: all steps done → status=completed");
                                changed_any = true;
                            }
                        }
                    }
                }
            }
        }
    }

    if !dry_run && changed_any {
        match commit(&root, res.advanced.len()).await {
            Ok(h) => res.committed = Some(h),
            Err(e) => res.error = Some(format!("commit: {}", e)),
        }
    }

    finish(&res, &started_at, started.elapsed().as_millis() as u64);
    res
}

fn finish(res: &WorkplanStewardResult, started_at: &str, dur_ms: u64) {
    let detail = format!(
        "workplan-reconcile-sweep{}: {} advanced→completed, {} format issue(s) (of {} scanned){}",
        if res.dry_run { " (dry-run)" } else { "" },
        res.advanced.len(),
        res.format_issues.len(),
        res.scanned,
        res.committed.as_deref().map(|h| format!(" @ {}", h)).unwrap_or_default(),
    );
    record_agent_run(
        "workplan-steward",
        started_at.to_string(),
        detail,
        res.error.is_none(),
        res.committed.clone(),
        dur_ms,
        res.error.clone(),
    );
}

fn fname(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string()
}

async fn commit(root: &Path, n: usize) -> Result<String, String> {
    let add = tokio::process::Command::new("git")
        .args(["add", "docs/workplans"])
        .current_dir(root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !add.status.success() {
        return Err(format!("git add: {}", String::from_utf8_lossy(&add.stderr)));
    }
    let msg = format!(
        "chore(workplan): workplan-steward — reconcile {} completed workplan(s)\n\n\
         In-nexus workplan-steward sweep: workplans whose every step is done advanced\n\
         to status=completed.\n\nCo-Authored-By: workplan-steward <noreply@hex.local>",
        n
    );
    let c = tokio::process::Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !c.status.success() {
        return Err(format!("git commit: {}", String::from_utf8_lossy(&c.stderr)));
    }
    let rev = tokio::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&rev.stdout).trim().to_string())
}

fn repo_root() -> PathBuf {
    if let Ok(p) = std::env::var("HEX_PROJECT_ROOT") {
        return PathBuf::from(p);
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if dir.join(".git").exists() {
            return dir;
        }
        if !dir.pop() {
            return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
    }
}
