//! Workplan conductor — top-down autonomous driver for `docs/workplans/feat-*.json`.
//!
//! Closes the structural gap surfaced 2026-05-28 during the ebay-mvp scaling
//! test: a 32-step workplan was dispatched once and the persona produced ONE
//! tool plan covering ~2 files, then went idle. None of the ~50 existing
//! supervisors owned "drive this workplan to completion" — gap_dispatcher
//! handles ad-hoc gap:* memory entries on a 30-min tick with 6-hour cooldowns,
//! pool_autopause actively kills idle persona pools, workplan_executor is
//! API-invoked rather than self-driving.
//!
//! Behavior per tick:
//!   1. Glob `docs/workplans/feat-*.json`
//!   2. For each workplan, parse `steps[]`
//!   3. For each step in order:
//!      - If all `files_to_create` exist with non-empty content → step done
//!      - Else if any `dependencies` step is incomplete → skip (dep order)
//!      - Else if step was dispatched within COOLDOWN → skip (rate limit)
//!      - Else dispatch a brief to the persona via /api/org/send-message
//!        and record the dispatch timestamp; break out (one dispatch/workplan/tick)
//!   4. Record progress under `feature/<workplan_id>/progress`
//!   5. If a workplan made no progress for `STALL_TICKS`, route an
//!      escalation DM to engineering-lead with the stuck step and last
//!      dispatch time
//!
//! Defaults (override via env):
//!   HEX_WORKPLAN_CONDUCTOR_INTERVAL_SECS   60    — tick every 60s
//!   HEX_WORKPLAN_CONDUCTOR_COOLDOWN_SECS   300   — 5min per-step cooldown
//!   HEX_WORKPLAN_CONDUCTOR_STALL_TICKS     10    — escalate after 10 no-progress ticks
//!   HEX_WORKPLAN_CONDUCTOR_STARTUP_SECS    45    — settle period before first tick
//!   HEX_DISABLE_WORKPLAN_CONDUCTOR         (any) — disable entirely

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const DEFAULT_INTERVAL_SECS: u64 = 60;
const DEFAULT_COOLDOWN_SECS: u64 = 300;
const DEFAULT_STALL_TICKS: u64 = 10;
const DEFAULT_STARTUP_SECS: u64 = 45;
const SEND_AGENT_ID: &str = "nexus-workplan-conductor";

fn parse_env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(default)
}

#[derive(Default)]
struct ConductorState {
    /// Per-step last dispatch timestamps for cooldown enforcement.
    dispatched: HashMap<String, Instant>,
    /// Per-workplan tick count since last observed progress.
    stall_count: HashMap<String, u64>,
    /// Per-workplan signature of last-known completion state (used to
    /// detect "made progress this tick").
    last_progress_sig: HashMap<String, String>,
}

fn state() -> &'static Mutex<ConductorState> {
    static S: OnceLock<Mutex<ConductorState>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(ConductorState::default()))
}

pub fn spawn(repo_root: PathBuf) {
    if std::env::var("HEX_DISABLE_WORKPLAN_CONDUCTOR").is_ok() {
        tracing::info!("workplan_conductor disabled via HEX_DISABLE_WORKPLAN_CONDUCTOR");
        return;
    }

    let interval_secs = parse_env_u64("HEX_WORKPLAN_CONDUCTOR_INTERVAL_SECS", DEFAULT_INTERVAL_SECS);
    let cooldown_secs = parse_env_u64("HEX_WORKPLAN_CONDUCTOR_COOLDOWN_SECS", DEFAULT_COOLDOWN_SECS);
    let stall_ticks = parse_env_u64("HEX_WORKPLAN_CONDUCTOR_STALL_TICKS", DEFAULT_STALL_TICKS);
    let startup_secs = parse_env_u64("HEX_WORKPLAN_CONDUCTOR_STARTUP_SECS", DEFAULT_STARTUP_SECS);

    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!(error = %e, "workplan_conductor: http client build failed");
                return;
            }
        };

        tracing::info!(
            interval_secs,
            cooldown_secs,
            stall_ticks,
            startup_secs,
            repo_root = %repo_root.display(),
            "workplan_conductor: spawning (top-down driver for feat-*.json workplans)"
        );

        tokio::time::sleep(Duration::from_secs(startup_secs)).await;

        let nexus_port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
        let nexus_base = format!("http://127.0.0.1:{}", nexus_port);
        let cooldown = Duration::from_secs(cooldown_secs);

        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if let Err(e) = run_tick(&http, &nexus_base, &repo_root, cooldown, stall_ticks).await {
                tracing::warn!(error = %e, "workplan_conductor: tick failed");
            }
        }
    });
}

async fn run_tick(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    repo_root: &Path,
    cooldown: Duration,
    stall_ticks: u64,
) -> Result<(), String> {
    let workplans_dir = repo_root.join("docs").join("workplans");
    let entries = match std::fs::read_dir(&workplans_dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(dir = %workplans_dir.display(), error = %e, "workplan_conductor: no workplans dir");
            return Ok(());
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("feat-") || !name.ends_with(".json") {
            continue;
        }
        if let Err(e) = drive_workplan(http, nexus_base, repo_root, &path, cooldown, stall_ticks).await {
            tracing::warn!(workplan = %path.display(), error = %e, "workplan_conductor: drive failed");
        }
    }

    Ok(())
}

async fn drive_workplan(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    repo_root: &Path,
    workplan_path: &Path,
    cooldown: Duration,
    stall_ticks: u64,
) -> Result<(), String> {
    let raw = std::fs::read_to_string(workplan_path)
        .map_err(|e| format!("read workplan: {}", e))?;
    let plan: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("parse workplan: {}", e))?;

    let workplan_id = plan
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("workplan missing id")?
        .to_string();

    let steps = plan
        .get("steps")
        .and_then(|v| v.as_array())
        .ok_or("workplan missing steps[]")?;

    // Compute per-step completion. A step is "done" iff every file in
    // files_to_create exists with non-zero size. Edits are not gated here
    // (we'd need git-blame to detect them reliably); the cargo gate in
    // action_executor catches semantic failures.
    let mut done: HashMap<String, bool> = HashMap::new();
    let mut step_index: HashMap<String, &Value> = HashMap::new();
    for step in steps {
        let id = match step.get("id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let mut all_present = true;
        let mut any_listed = false;
        if let Some(files) = step.get("files_to_create").and_then(|v| v.as_array()) {
            for f in files {
                if let Some(rel) = f.as_str() {
                    any_listed = true;
                    let abs = repo_root.join(rel);
                    let exists_nonempty = std::fs::metadata(&abs)
                        .map(|m| m.is_file() && m.len() > 0)
                        .unwrap_or(false);
                    if !exists_nonempty {
                        all_present = false;
                        break;
                    }
                }
            }
        }
        let step_done = any_listed && all_present;
        done.insert(id.clone(), step_done);
        step_index.insert(id, step);
    }

    // Build progress signature so we can detect stall.
    let mut completed: Vec<&String> = done.iter().filter(|(_, v)| **v).map(|(k, _)| k).collect();
    completed.sort();
    let progress_sig = completed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(",");
    let completed_count = completed.len();
    let total = done.len();

    // Persist progress to memory (best-effort).
    let progress_value = json!({
        "workplan_id": workplan_id,
        "workplan_path": workplan_path.to_string_lossy(),
        "completed": completed,
        "completed_count": completed_count,
        "total_steps": total,
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });
    let _ = memory_store(http, nexus_base, &format!("feature/{}/progress", workplan_id), &progress_value).await;

    // Update stall counter.
    let stalled = {
        let mut s = state().lock().unwrap();
        let prev = s.last_progress_sig.get(&workplan_id).cloned().unwrap_or_default();
        if prev != progress_sig {
            s.last_progress_sig.insert(workplan_id.clone(), progress_sig.clone());
            s.stall_count.insert(workplan_id.clone(), 0);
            false
        } else {
            let n = s.stall_count.entry(workplan_id.clone()).or_insert(0);
            *n += 1;
            *n >= stall_ticks
        }
    };

    // If the workplan is complete, log + return without dispatching.
    if total > 0 && completed_count == total {
        tracing::info!(
            workplan = %workplan_id,
            completed = completed_count,
            total,
            "workplan_conductor: workplan complete"
        );
        return Ok(());
    }

    // Find first step that is incomplete, dep-satisfied, and out of cooldown.
    let now = Instant::now();
    let mut next_step: Option<&Value> = None;
    for step in steps {
        let id = match step.get("id").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        if *done.get(id).unwrap_or(&false) {
            continue;
        }
        // Dep check: every dep id must be marked done.
        let deps_satisfied = step
            .get("dependencies")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .all(|d| *done.get(d).unwrap_or(&false))
            })
            .unwrap_or(true);
        if !deps_satisfied {
            continue;
        }
        // Cooldown check.
        let cooldown_key = format!("{}::{}", workplan_id, id);
        let in_cooldown = {
            let s = state().lock().unwrap();
            s.dispatched
                .get(&cooldown_key)
                .map(|t| now.duration_since(*t) < cooldown)
                .unwrap_or(false)
        };
        if in_cooldown {
            continue;
        }
        next_step = Some(step);
        break;
    }

    let Some(step) = next_step else {
        // Nothing dispatchable this tick (either all done, in cooldown, or
        // blocked by deps). If stalled, escalate.
        if stalled {
            escalate_stall(http, nexus_base, &workplan_id, completed_count, total).await;
            // Reset counter so we don't re-escalate every tick.
            let mut s = state().lock().unwrap();
            s.stall_count.insert(workplan_id.clone(), 0);
        }
        return Ok(());
    };

    let step_id = step.get("id").and_then(|v| v.as_str()).unwrap_or("???").to_string();
    let cooldown_key = format!("{}::{}", workplan_id, step_id);

    // Build the brief.
    let brief = build_brief(&workplan_id, workplan_path, step);

    // Dispatch via /api/org/send-message.
    let target = step
        .get("assignee")
        .and_then(|v| v.as_str())
        .unwrap_or("hex-coder")
        .to_string();
    let subject = format!("{} {} (workplan_conductor)", workplan_id, step_id);
    let dispatched = send_dm(http, nexus_base, &target, &subject, &brief).await;

    match dispatched {
        Ok(()) => {
            let mut s = state().lock().unwrap();
            s.dispatched.insert(cooldown_key, now);
            tracing::info!(
                workplan = %workplan_id,
                step = %step_id,
                target = %target,
                completed_count,
                total,
                "workplan_conductor: dispatched step"
            );
        }
        Err(e) => {
            tracing::warn!(
                workplan = %workplan_id,
                step = %step_id,
                target = %target,
                error = %e,
                "workplan_conductor: dispatch failed"
            );
        }
    }

    Ok(())
}

fn build_brief(workplan_id: &str, workplan_path: &Path, step: &Value) -> String {
    let step_id = step.get("id").and_then(|v| v.as_str()).unwrap_or("???");
    let description = step.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let done_condition = step.get("done_condition").and_then(|v| v.as_str()).unwrap_or("");
    let files: Vec<&str> = step
        .get("files_to_create")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let edits: Vec<&str> = step
        .get("files_to_edit")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let spec_ids: Vec<&str> = step
        .get("spec_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str(&format!(
        "Execute {} {} of workplan {}.\n\n",
        workplan_id,
        step_id,
        workplan_path.display()
    ));
    out.push_str(&format!("Description: {}\n\n", description));
    if !done_condition.is_empty() {
        out.push_str(&format!("Done condition: {}\n\n", done_condition));
    }
    if !spec_ids.is_empty() {
        out.push_str(&format!(
            "Spec references in docs/specs/ (look in the workplan's specs field for the file path): {}\n\n",
            spec_ids.join(", ")
        ));
    }
    out.push_str("Emit your reply as code_patch tool calls, one per file, fully-qualified paths from workspace root (e.g. examples/ebay-clone/backend/src/...). Do not wrap content in markdown fences. Do not include trailing prose after content.\n\n");
    if !files.is_empty() {
        out.push_str("Files to create:\n");
        for f in &files {
            out.push_str(&format!("- code_patch: create {}\n", f));
        }
        out.push('\n');
    }
    if !edits.is_empty() {
        out.push_str("Files to edit:\n");
        for f in &edits {
            out.push_str(&format!("- code_patch: edit {}\n", f));
        }
        out.push('\n');
    }
    out.push_str("Each file must be valid syntax (Rust must pass cargo check; TOML must parse). Implement the spec conventions exactly. If cargo_check rejects, the executor will roll back; rewrite and re-emit.");
    out
}

async fn send_dm(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    target: &str,
    subject: &str,
    content: &str,
) -> Result<(), String> {
    let url = format!("{}/api/org/send-message", nexus_base);
    let body = json!({
        "from": SEND_AGENT_ID,
        "to": target,
        "subject": subject,
        "content": content,
    });
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send transport: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(format!("send HTTP {}: {}", status, txt.chars().take(200).collect::<String>()));
    }
    Ok(())
}

async fn escalate_stall(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    workplan_id: &str,
    completed_count: usize,
    total: usize,
) {
    let content = format!(
        "Workplan {} has stalled. Progress: {}/{} steps. The conductor has dispatched the next dep-satisfied step repeatedly without seeing new files committed. Investigate hex-coder pool state, twin escalations, or workplan validity.",
        workplan_id, completed_count, total
    );
    let _ = send_dm(
        http,
        nexus_base,
        "engineering-lead",
        &format!("STALLED: {}", workplan_id),
        &content,
    )
    .await;
    tracing::warn!(
        workplan = %workplan_id,
        completed_count,
        total,
        "workplan_conductor: stall escalation sent to engineering-lead"
    );
}

async fn memory_store(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    key: &str,
    value: &Value,
) -> Result<(), String> {
    let url = format!("{}/api/hexflo/memory/store", nexus_base);
    let body = json!({
        "key": key,
        "value": value.to_string(),
    });
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("memory store transport: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("memory store HTTP {}", resp.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn make_workplan(tmp: &Path, files: &[&str]) -> PathBuf {
        let wp_dir = tmp.join("docs").join("workplans");
        fs::create_dir_all(&wp_dir).unwrap();
        let plan = json!({
            "id": "feat-test",
            "title": "test",
            "steps": [
                {
                    "id": "step-1",
                    "description": "test step",
                    "files_to_create": files,
                    "dependencies": [],
                    "done_condition": "files exist"
                }
            ]
        });
        let path = wp_dir.join("feat-test.json");
        fs::write(&path, serde_json::to_string_pretty(&plan).unwrap()).unwrap();
        path
    }

    #[test]
    fn build_brief_includes_files_and_spec_refs() {
        let step = json!({
            "id": "step-2",
            "description": "domain value types",
            "files_to_create": ["a.rs", "b.rs"],
            "spec_ids": ["s-1", "s-2"],
            "done_condition": "cargo check passes"
        });
        let brief = build_brief("feat-x", Path::new("docs/workplans/feat-x.json"), &step);
        assert!(brief.contains("feat-x"));
        assert!(brief.contains("step-2"));
        assert!(brief.contains("domain value types"));
        assert!(brief.contains("code_patch: create a.rs"));
        assert!(brief.contains("code_patch: create b.rs"));
        assert!(brief.contains("s-1, s-2"));
        assert!(brief.contains("Do not wrap content in markdown fences"));
        assert!(brief.contains("cargo check passes"));
    }

    #[test]
    fn step_is_done_when_all_files_exist_nonempty() {
        let tmp = TempDir::new().unwrap();
        let f1 = tmp.path().join("examples").join("a.rs");
        let f2 = tmp.path().join("examples").join("b.rs");
        fs::create_dir_all(f1.parent().unwrap()).unwrap();
        fs::write(&f1, "fn main() {}").unwrap();
        fs::write(&f2, "pub mod x;").unwrap();
        let _ = make_workplan(tmp.path(), &["examples/a.rs", "examples/b.rs"]);

        // Manually run the completion check used in drive_workplan.
        let all_present = ["examples/a.rs", "examples/b.rs"].iter().all(|p| {
            tmp.path().join(p).metadata().map(|m| m.len() > 0).unwrap_or(false)
        });
        assert!(all_present);
    }

    #[test]
    fn step_is_not_done_when_file_is_zero_bytes() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("a.rs");
        fs::write(&f, "").unwrap();
        let nonempty = std::fs::metadata(&f).map(|m| m.len() > 0).unwrap_or(false);
        assert!(!nonempty);
    }

    #[test]
    fn step_is_not_done_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let nonempty = std::fs::metadata(tmp.path().join("missing.rs"))
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        assert!(!nonempty);
    }
}
