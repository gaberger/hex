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
    pub format_fixed: usize,
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
        format_fixed: 0,
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
            let j: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => {
                    res.format_issues.push(format!("{}: invalid JSON", fname(&p)));
                    continue;
                }
            };
            res.scanned += 1;
            let id = j.get("id").and_then(|v| v.as_str()).map(String::from).unwrap_or_else(|| fname(&p));

            // Edit at the string level (serde_json has no preserve_order here — a
            // reserialize would scramble every key). Both fixes are line-precise.
            let mut new_text = text.clone();
            let mut dirty = false;

            // FORMAT FIX — every workplan needs a status; default a missing one to
            // "pending" (the neutral unstarted default). Inserts after the opening brace.
            if j.get("status").and_then(|v| v.as_str()).is_none() {
                res.format_issues.push(format!("{}: missing status → set pending", id));
                if !dry_run {
                    if let Some(b) = new_text.find('{') {
                        let after = new_text[b + 1..]
                            .find('\n')
                            .map(|n| b + 1 + n + 1)
                            .unwrap_or(b + 1);
                        new_text.insert_str(after, "  \"status\": \"pending\",\n");
                        res.format_fixed += 1;
                        dirty = true;
                    }
                }
            }

            // FORMAT (report) + RECONCILE over steps.
            if let Some(steps) = j.get("steps").and_then(|v| v.as_array()) {
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

                // RECONCILE — every step done → workplan completed (replace the value).
                let status = j.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let all_done = !steps.is_empty()
                    && steps.iter().all(|s| {
                        matches!(s.get("status").and_then(|v| v.as_str()).unwrap_or(""), "done" | "completed")
                    });
                if all_done && !status.is_empty() && !matches!(status, "completed" | "done") {
                    res.advanced.push(id.clone());
                    if !dry_run {
                        new_text = set_json_status(&new_text, "completed");
                        tracing::info!(workplan = %id, "workplan-steward: all steps done → status=completed");
                        dirty = true;
                    }
                }
            }

            if !dry_run && dirty && std::fs::write(&p, &new_text).is_ok() {
                changed_any = true;
            }
        }
    }

    if !dry_run && changed_any {
        match commit(&root, res.format_fixed, res.advanced.len()).await {
            Ok(h) => res.committed = Some(h),
            Err(e) => res.error = Some(format!("commit: {}", e)),
        }
    }

    finish(&res, &started_at, started.elapsed().as_millis() as u64);
    res
}

fn finish(res: &WorkplanStewardResult, started_at: &str, dur_ms: u64) {
    let detail = format!(
        "workplan-reconcile-sweep{}: {} format-fixed, {} reconciled→completed, {} issue(s) (of {} scanned){}",
        if res.dry_run { " (dry-run)" } else { "" },
        res.format_fixed,
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

/// Replace the first `"status": "<old>"` value with `new`, preserving the rest of
/// the file byte-for-byte (no reserialize → no key reordering).
fn set_json_status(text: &str, new: &str) -> String {
    let Some(key) = text.find("\"status\"") else { return text.to_string() };
    let after_key = &text[key + "\"status\"".len()..];
    let Some(colon) = after_key.find(':') else { return text.to_string() };
    // first quote after the colon opens the value
    let Some(open_rel) = after_key[colon..].find('"') else { return text.to_string() };
    let val_start = key + "\"status\"".len() + colon + open_rel + 1;
    let Some(close_rel) = text[val_start..].find('"') else { return text.to_string() };
    let val_end = val_start + close_rel;
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..val_start]);
    out.push_str(new);
    out.push_str(&text[val_end..]);
    out
}

async fn commit(root: &Path, fixed: usize, reconciled: usize) -> Result<String, String> {
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
        "chore(workplan): workplan-steward — {} format-fixed, {} reconciled\n\n\
         In-nexus workplan-steward sweep: workplans missing a status got the neutral\n\
         `pending` default; workplans whose every step is done advanced to completed.\n\n\
         Co-Authored-By: workplan-steward <noreply@hex.local>",
        fixed, reconciled
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
