//! adr-steward — an in-nexus hex agent that drives the ADR lifecycle.
//!
//! The reliable counterpart to the persona/SOP path (which stalls at
//! commitment→action): this runs DIRECTLY in nexus. It advances Accepted ADRs
//! that hex has already confirmed implemented — those carrying an
//! `**Implementation-Present:**` auto-scan annotation — to Completed, commits the
//! batch, and records itself to the shared agent-runs feed
//! (GET /api/direct/runs → the dashboard), so it shows up as a hex agent doing work.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::direct_exec::record_agent_run;

#[derive(Debug, Serialize)]
pub struct StewardResult {
    pub agent: String,
    pub action: String,
    pub scanned: usize,
    pub accepted: usize,
    pub completed: Vec<String>,
    pub skipped_no_evidence: usize,
    pub dry_run: bool,
    pub committed: Option<String>,
    pub error: Option<String>,
}

/// Run the ADR lifecycle sweep. `dry_run` reports candidates without mutating.
pub async fn run_lifecycle_sweep(dry_run: bool) -> StewardResult {
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().to_rfc3339();
    let mut res = StewardResult {
        agent: "adr-steward".into(),
        action: "adr-lifecycle-sweep".into(),
        scanned: 0,
        accepted: 0,
        completed: Vec::new(),
        skipped_no_evidence: 0,
        dry_run,
        committed: None,
        error: None,
    };

    let root = repo_root();
    let dir = root.join("docs/adrs");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            res.error = Some(format!("read docs/adrs: {}", e));
            finish(&res, &started_at, started.elapsed().as_millis() as u64);
            return res;
        }
    };

    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("ADR-") {
            continue;
        }
        let content = match std::fs::read_to_string(&p) {
            Ok(c) => c,
            Err(_) => continue,
        };
        res.scanned += 1;
        if parse_status(&content) != "accepted" {
            continue;
        }
        res.accepted += 1;
        // hex-native confirmation: an Implementation-Present / -Verified annotation.
        let lc = content.to_lowercase();
        let confirmed = lc.contains("implementation-present:") || lc.contains("implementation-verified:");
        if !confirmed {
            res.skipped_no_evidence += 1;
            continue;
        }
        let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name).to_string();
        if dry_run {
            res.completed.push(id);
            continue;
        }
        if let Some(new) = rewrite_status_to_completed(&content) {
            if std::fs::write(&p, new).is_ok() {
                tracing::info!(adr = %id, "adr-steward: Accepted → Completed (Implementation-Present)");
                res.completed.push(id);
            }
        }
    }

    if !dry_run && !res.completed.is_empty() {
        match commit(&root, res.completed.len()).await {
            Ok(hash) => res.committed = Some(hash),
            Err(e) => res.error = Some(format!("commit: {}", e)),
        }
    }

    finish(&res, &started_at, started.elapsed().as_millis() as u64);
    res
}

fn finish(res: &StewardResult, started_at: &str, dur_ms: u64) {
    let detail = format!(
        "adr-lifecycle-sweep{}: {} Accepted→Completed (of {} accepted / {} scanned){}",
        if res.dry_run { " (dry-run)" } else { "" },
        res.completed.len(),
        res.accepted,
        res.scanned,
        res.committed.as_deref().map(|h| format!(" @ {}", h)).unwrap_or_default(),
    );
    record_agent_run(
        "adr-steward",
        started_at.to_string(),
        detail,
        res.error.is_none(),
        res.committed.clone(),
        dur_ms,
        res.error.clone(),
    );
}

fn parse_status(content: &str) -> String {
    for line in content.lines() {
        let t = line.trim();
        let lower = t.to_lowercase();
        let val = if lower.starts_with("**status:**") {
            t["**Status:**".len()..].trim().to_lowercase()
        } else if lower.starts_with("status:") && !lower.starts_with("status_") {
            t["status:".len()..].trim().to_lowercase()
        } else {
            continue;
        };
        for s in ["completed", "accepted", "proposed", "superseded", "rejected", "abandoned", "deprecated"] {
            if val.contains(s) {
                return s.to_string();
            }
        }
        return "unknown".to_string();
    }
    "unknown".to_string()
}

fn rewrite_status_to_completed(content: &str) -> Option<String> {
    let mut out = String::with_capacity(content.len() + 16);
    let mut found = false;
    for line in content.split_inclusive('\n') {
        let t = line.trim_start();
        let lower = t.to_lowercase();
        let nl = if line.ends_with('\n') { "\n" } else { "" };
        let indent = &line[..line.len() - t.len()];
        if !found && lower.starts_with("**status:**") {
            out.push_str(indent);
            out.push_str("**Status:** Completed");
            out.push_str(nl);
            found = true;
        } else if !found && lower.starts_with("status:") && !lower.starts_with("status_") {
            out.push_str(indent);
            out.push_str("Status: Completed");
            out.push_str(nl);
            found = true;
        } else {
            out.push_str(line);
        }
    }
    if found {
        Some(out)
    } else {
        None
    }
}

async fn commit(root: &Path, n: usize) -> Result<String, String> {
    let add = tokio::process::Command::new("git")
        .args(["add", "docs/adrs"])
        .current_dir(root)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !add.status.success() {
        return Err(format!("git add: {}", String::from_utf8_lossy(&add.stderr)));
    }
    let msg = format!(
        "chore(adr): adr-steward — advance {} implemented ADRs Accepted→Completed\n\n\
         In-nexus adr-steward lifecycle sweep: Accepted ADRs carrying an\n\
         Implementation-Present annotation (hex-confirmed implemented) advanced to\n\
         Completed.\n\nCo-Authored-By: adr-steward <noreply@hex.local>",
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
