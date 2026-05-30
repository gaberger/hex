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

use super::brain_dispatch_reconciler::verify_evidence;

const DEFAULT_INTERVAL_SECS: u64 = 60;
const DEFAULT_COOLDOWN_SECS: u64 = 300;
const DEFAULT_STALL_TICKS: u64 = 10;
const DEFAULT_STARTUP_SECS: u64 = 45;
/// Max times the conductor re-dispatches a single step before opening its
/// circuit. Bounds the re-hallucination loop: when a persona keeps producing a
/// step whose evidence never passes (or an operator deletes a hallucinated
/// file the persona keeps rewriting), the conductor stops dispatching it and
/// escalates instead of looping forever. 0 disables the breaker.
const DEFAULT_MAX_DISPATCH_ATTEMPTS: u64 = 5;
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
    /// Evidence-gate result cache, keyed `workplan_id::step_id` → (files
    /// mtime-signature, passed). Predicates only re-run when the step's
    /// produced files change, so a green step costs one stat/tick, not a
    /// `cargo test` storm.
    evidence_pass: HashMap<String, (String, bool)>,
    /// Per-step dispatch attempts, keyed `workplan_id::step_id`. Bounds the
    /// re-hallucination loop — cleared when the step reaches evidence-verified
    /// done (so a genuinely-fixed step resets).
    dispatch_attempts: HashMap<String, u64>,
    /// Steps whose circuit has opened and been escalated (escalate once).
    circuit_escalated: std::collections::HashSet<String>,
}

/// Circuit is open once a step has been dispatched `max` times without reaching
/// done. `max == 0` disables the breaker.
fn is_circuit_open(attempts: u64, max: u64) -> bool {
    max > 0 && attempts >= max
}

/// True when a majority of the workplan's `files_to_create` target `examples/`,
/// i.e. it builds a target/example project rather than hex itself. hex's dev
/// conductor refuses these to keep example work out of hex's control plane.
fn workplan_targets_examples(steps: &[Value]) -> bool {
    let (mut example_files, mut total_files) = (0usize, 0usize);
    for step in steps {
        if let Some(files) = step.get("files_to_create").and_then(|v| v.as_array()) {
            for f in files.iter().filter_map(|v| v.as_str()) {
                total_files += 1;
                if f.starts_with("examples/") || f.contains("/examples/") {
                    example_files += 1;
                }
            }
        }
    }
    total_files > 0 && example_files * 2 > total_files
}

fn state() -> &'static Mutex<ConductorState> {
    static S: OnceLock<Mutex<ConductorState>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(ConductorState::default()))
}

/// Signature of a step's produced files (relpath:mtime), used to invalidate the
/// evidence-gate cache only when the files actually change.
fn step_files_signature(step: &Value, repo_root: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(files) = step.get("files_to_create").and_then(|v| v.as_array()) {
        for f in files {
            if let Some(rel) = f.as_str() {
                let stamp = std::fs::metadata(repo_root.join(rel))
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                parts.push(format!("{rel}:{stamp}"));
            }
        }
    }
    parts.join("|")
}

/// Walks up from `rel_file`'s directory (within `repo_root`) to find the
/// nearest ancestor containing `marker`, returning that dir relative to
/// `repo_root` (or "." for the root). Used to locate the owning crate/tsconfig
/// for default evidence derivation.
fn nearest_ancestor_with(repo_root: &Path, rel_file: &str, marker: &str) -> Option<String> {
    let abs = repo_root.join(rel_file);
    let mut dir = abs.parent()?.to_path_buf();
    loop {
        if dir.join(marker).is_file() {
            let rel = dir.strip_prefix(repo_root).ok()?;
            return Some(if rel.as_os_str().is_empty() {
                ".".to_string()
            } else {
                rel.to_string_lossy().into_owned()
            });
        }
        if dir == *repo_root || !dir.pop() {
            return None;
        }
    }
}

/// "Evidence by default": derive language-aware compile predicates for a step
/// from the files it produces, when it declares no explicit `evidence`. Rust
/// files → `cargo check` of the owning crate; TypeScript → `tsc --noEmit` of
/// the owning tsconfig dir. Non-code files (docs/scripts/config) derive nothing
/// and keep files-exist semantics. These are ADVISORY (see [`step_evidence_gate`]).
fn derive_default_evidence(files: &[String], repo_root: &Path) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut crates: BTreeSet<String> = BTreeSet::new();
    let mut ts_dirs: BTreeSet<String> = BTreeSet::new();
    for f in files {
        if f.ends_with(".rs") {
            if let Some(dir) = nearest_ancestor_with(repo_root, f, "Cargo.toml") {
                crates.insert(dir);
            }
        } else if f.ends_with(".ts") || f.ends_with(".tsx") {
            if let Some(dir) = nearest_ancestor_with(repo_root, f, "tsconfig.json") {
                ts_dirs.insert(dir);
            }
        }
    }
    let mut preds = Vec::new();
    for c in crates {
        let cd = if c == "." { String::new() } else { format!("cd {c} && ") };
        preds.push(format!("{cd}PATH=\"$HOME/.cargo/bin:$PATH\" cargo check -q"));
    }
    for d in ts_dirs {
        let cd = if d == "." { String::new() } else { format!("cd {d} && ") };
        preds.push(format!("{cd}(bunx tsc --noEmit || npx --yes tsc --noEmit)"));
    }
    preds
}

/// Decides whether a step is "done" by running its acceptance predicates and
/// returns `(completes, reason)`. Predicate execution runs off the async
/// runtime (`spawn_blocking`) and is cached per step on [`step_files_signature`]
/// so it re-runs only when the produced files change.
///
/// Two authority levels:
///   * **explicit** `evidence[]` — AUTHORITATIVE: a failure blocks completion
///     (the step re-opens and is re-dispatched).
///   * **derived** defaults (from [`derive_default_evidence`]) — ADVISORY: a
///     failure is logged and penalised in RL but does NOT block completion,
///     because auto-guessed predicates have environmental false-negatives (a
///     wasm crate that can't `cargo check` on the host). Promote a derived
///     predicate to explicit `evidence` to hard-gate it.
///
/// Either way the RL reward reflects the ACTUAL verdict, so every code-producing
/// step feeds the learning signal by default — not just annotated ones.
async fn step_evidence_gate(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    workplan_id: &str,
    step_id: &str,
    step: &Value,
    repo_root: &Path,
) -> (bool, Option<String>) {
    let explicit: Vec<String> = step
        .get("evidence")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let authoritative = !explicit.is_empty();
    let predicates = if authoritative {
        explicit
    } else {
        let files: Vec<String> = step
            .get("files_to_create")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        derive_default_evidence(&files, repo_root)
    };
    if predicates.is_empty() {
        return (true, None);
    }

    let sig = step_files_signature(step, repo_root);
    let cache_key = format!("{workplan_id}::{step_id}");
    if let Some((cached_sig, passed)) = state().lock().unwrap().evidence_pass.get(&cache_key) {
        if *cached_sig == sig {
            return (if authoritative { *passed } else { true }, None);
        }
    }

    let synthetic = json!({ "evidence": predicates });
    let root_owned = repo_root.to_path_buf();
    let result = tokio::task::spawn_blocking(move || verify_evidence(&synthetic, &root_owned))
        .await
        .unwrap_or_else(|e| Err(format!("evidence task panicked: {e}")));
    let (ran_ok, reason) = match result {
        Ok(()) => (true, None),
        Err(why) => (false, Some(why)),
    };

    state()
        .lock()
        .unwrap()
        .evidence_pass
        .insert(cache_key, (sig, ran_ok));

    // RL reward reflects the ACTUAL verdict (explicit or derived) — ADR-024
    // "reward = test pass rate" / ADR-2026-03-24-0045 P4. Every code step feeds
    // the learning signal by default, redirecting the q-table off the
    // homeostasis loop that sat at mean reward -0.99.
    let state_key = evidence_state_key(step);
    let action = format!("persona:{}", route_step_to_persona(step));
    post_rl_reward(
        http,
        nexus_base,
        &state_key,
        &action,
        if ran_ok { 1.0 } else { -1.0 },
        if ran_ok { "accepted" } else { "rejected" },
    )
    .await;

    if !ran_ok {
        let why = reason.as_deref().unwrap_or("evidence predicate failed");
        if authoritative {
            tracing::warn!(
                workplan = %workplan_id, step = %step_id, reason = %why,
                "workplan_conductor: EVIDENCE GATE rejected completion (explicit, re-opening step)"
            );
        } else {
            tracing::warn!(
                workplan = %workplan_id, step = %step_id, reason = %why,
                "workplan_conductor: derived evidence FAILED (advisory — not blocking; \
                 add an explicit `evidence` predicate to hard-gate this step)"
            );
        }
    }

    // Derived evidence never blocks completion; explicit evidence is authoritative.
    (if authoritative { ran_ok } else { true }, reason)
}

/// RL state key for an evidence verdict: the *kind* of work (tier + layer),
/// so the q-table generalizes across steps of the same shape.
fn evidence_state_key(step: &Value) -> String {
    let tier = step.get("tier").and_then(|v| v.as_u64());
    let layer = step.get("layer").and_then(|v| v.as_str()).unwrap_or("");
    match (tier, layer.is_empty()) {
        (Some(t), false) => format!("evidence:tier{t}:{layer}"),
        (Some(t), true) => format!("evidence:tier{t}"),
        (None, false) => format!("evidence:{layer}"),
        (None, true) => "evidence:step".to_string(),
    }
}

/// Fire-and-forget RL reward to the local nexus rl-engine. Skipped when
/// `nexus_base` is empty (unit tests). Never blocks the conductor on failure.
async fn post_rl_reward(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    state_key: &str,
    action: &str,
    reward: f64,
    next_state_key: &str,
) {
    if nexus_base.is_empty() {
        return;
    }
    let url = format!("{nexus_base}/api/rl/reward");
    let body = json!({
        "stateKey": state_key,
        "action": action,
        "reward": reward,
        "nextStateKey": next_state_key,
    });
    match http.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                state_key, action, reward,
                "workplan_conductor: RL reward recorded from evidence gate"
            );
        }
        Ok(resp) => {
            tracing::debug!(status = %resp.status(), "workplan_conductor: RL reward non-success");
        }
        Err(e) => {
            tracing::debug!(error = %e, "workplan_conductor: RL reward post failed");
        }
    }
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

    let max_dispatch =
        parse_env_u64("HEX_WORKPLAN_CONDUCTOR_MAX_DISPATCH", DEFAULT_MAX_DISPATCH_ATTEMPTS);

    // De-conflation guard (2026-05-30): hex's dev conductor drives hex's OWN
    // feature workplans only — never an example app's. A workplan whose steps
    // target `examples/` belongs to a target project (hex is installed INTO it
    // and drives it from there), so driving it from the hex repo would absorb
    // example-app work into hex's own conductor / RL / git history. Refuse it.
    if workplan_targets_examples(steps) {
        tracing::debug!(
            workplan = %workplan_id,
            "workplan_conductor: skipping example-targeting workplan (de-conflation — \
             examples are driven as their own target project, not by hex's dev conductor)"
        );
        return Ok(());
    }

    // Compute per-step completion. A step is "done" iff every file in
    // files_to_create exists with non-zero size AND its `evidence` predicates
    // (if any) pass. Files-exist alone is necessary but not sufficient: it
    // marked a stub router + hallucinated tests "complete" on the 2026-05-28
    // ebay-mvp run. The evidence gate (ADR-2026-05-17-2030 "measurable
    // acceptance gate"; ADR-2026-04-11-1800 "reject vacuous completions")
    // closes that loop — a step that produces files but whose acceptance
    // predicate fails is NOT done and gets re-dispatched. Steps that declare
    // no `evidence` keep the legacy files-exist semantics.
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
        let files_present = any_listed && all_present;
        // Files present is necessary; the evidence gate (explicit hard-gate or
        // derived advisory) decides done + emits the RL verdict. It logs its own
        // rejections, so we just take the boolean here.
        let step_done = if files_present {
            step_evidence_gate(http, nexus_base, &workplan_id, &id, step, repo_root)
                .await
                .0
        } else {
            false
        };
        if step_done {
            // Step reached evidence-verified done → clear its circuit state so
            // a future regression gets a fresh dispatch budget.
            let key = format!("{}::{}", workplan_id, id);
            let mut s = state().lock().unwrap();
            s.dispatch_attempts.remove(&key);
            s.circuit_escalated.remove(&key);
        }
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
        let cooldown_key = format!("{}::{}", workplan_id, id);
        // Circuit breaker (re-hallucination guard): if this step has been
        // dispatched max times without reaching evidence-verified done, stop
        // dispatching it — escalate once and skip, so the persona can't keep
        // rewriting the same broken artifact in a loop.
        let attempts = state()
            .lock()
            .unwrap()
            .dispatch_attempts
            .get(&cooldown_key)
            .copied()
            .unwrap_or(0);
        if is_circuit_open(attempts, max_dispatch) {
            let first = {
                let mut s = state().lock().unwrap();
                s.circuit_escalated.insert(cooldown_key.clone())
            };
            if first {
                tracing::warn!(
                    workplan = %workplan_id, step = %id, attempts, max = max_dispatch,
                    "workplan_conductor: dispatch circuit OPEN — step failed to reach \
                     evidence-verified done after max attempts; NOT re-dispatching \
                     (re-hallucination guard), escalating to operator"
                );
                escalate_circuit(http, nexus_base, &workplan_id, id, attempts).await;
            }
            continue;
        }
        // Cooldown check.
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

    // Build the brief. Pass repo_root so the brief lists ONLY files
    // that are still missing — surfaces the self-poisoning recursion
    // fix: when a step is half-done (e.g. 4/5 integration tests) the
    // persona was reading the full files_to_create list, seeing 4 of
    // them already on disk, and "completing" the step by re-writing
    // one of the existing files instead of creating the missing one.
    // Showing only the missing artifacts forces the persona's tool
    // plan to target real gaps.
    let brief = build_brief(&workplan_id, workplan_path, step, repo_root);

    // Dispatch via /api/org/send-message. Routing precedence:
    //   1. explicit step.assignee field (planner can override)
    //   2. derived from step.layer / step.id semantics — so independent steps
    //      go to different personas and the fleet works in parallel
    //   3. fallback to hex-coder (matches pre-fleet behaviour)
    //
    // Surfaced 2026-05-28 ebay-mvp scaling test: every step was dispatched to
    // hex-coder regardless of role, so even when the pool fleet IS running the
    // conductor serializes everything through one persona. Per-step routing
    // lets hex-tester own integration tests, integrator own composition,
    // hex-reviewer own ADR-conformance, etc. — concurrently.
    let target = step
        .get("assignee")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| route_step_to_persona(step));
    let subject = format!("{} {} (workplan_conductor)", workplan_id, step_id);
    let dispatched = send_dm(http, nexus_base, &target, &subject, &brief).await;

    match dispatched {
        Ok(()) => {
            let mut s = state().lock().unwrap();
            s.dispatched.insert(cooldown_key.clone(), now);
            let attempts = s.dispatch_attempts.entry(cooldown_key).or_insert(0);
            *attempts += 1;
            tracing::info!(
                workplan = %workplan_id,
                step = %step_id,
                target = %target,
                attempt = *attempts,
                max = max_dispatch,
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

/// Route a workplan step to one of the 5 lean-fleet personas.
///
/// Lean fleet (2026-05-28 refactor):
///   hex-coder         — source files, ports, domain, use cases, adapters,
///                       reducer code, anything that is "write code now"
///   hex-tester        — tests, behavioral specs, validation, smoke gates,
///                       acceptance tests
///   hex-reviewer      — code review, ADR conformance, dead-code analysis,
///                       boundary checks, adversarial review
///   integrator        — composition root, merge worktrees, docs, runbooks,
///                       start.sh / docker / ops glue
///   engineering-lead  — operator-facing dispatch, stall escalations,
///                       cross-team coordination, ADR drafting
///
/// Ordering matters: more-specific patterns win. The fallback is hex-coder
/// so unknown step shapes don't stall.
///
/// Replaces the previous 30-branch router that targeted 9+ specialist
/// personas. The lean version routes onto roles whose pools the supervisor
/// actually keeps alive; everything else is dispatched as if it were code.
fn route_step_to_persona(step: &Value) -> String {
    let layer = step.get("layer").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
    let desc = step.get("description").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();
    let id = step.get("id").and_then(|v| v.as_str()).unwrap_or("").to_ascii_lowercase();

    // === TESTER ===
    // Acceptance + integration + behavioral specs.
    if desc.contains("acceptance test") || desc.contains("acceptance ")
        || desc.contains("smoke test") || desc.contains("smoke ")
        || desc.contains("integration test")
        || desc.contains("behavioral spec") || desc.contains("write specs")
        || id.contains("acceptance") || id.contains("smoke")
        || layer == "tests"
    {
        return "hex-tester".into();
    }

    // === REVIEWER ===
    // Explicit review work, ADR conformance, dead-code, boundary checks.
    if desc.contains("code review") || desc.contains("review for")
        || desc.contains("adr conformance") || desc.contains("adr-conformance")
        || desc.contains("dead code") || desc.contains("dead-code")
        || desc.contains("boundary check") || desc.contains("hex analyze")
        || desc.contains("hex-analyze")
        || desc.contains("adversarial")
    {
        return "hex-reviewer".into();
    }

    // === INTEGRATOR ===
    // Composition root, merge, docs, ops glue.
    if desc.contains("composition root") || desc.contains("composition_root")
        || desc.contains("merge worktree") || desc.contains("merge feature")
        || desc.contains("readme") || desc.contains("docs/") || desc.contains("documentation")
        || desc.contains("start.sh") || desc.contains("docker-compose")
        || desc.contains("docker compose") || desc.contains("dockerfile")
        || desc.contains("runbook") || desc.contains("deploy")
        || layer == "composition" || layer == "integration"
        || id.contains("compose")
    {
        return "integrator".into();
    }

    // === LEAD ===
    // ADR drafting + operator-facing decisions. Not raised by typical
    // workplan steps; surfaces when escalations need attention.
    if desc.contains("adr draft") || desc.contains("draft an adr")
        || desc.contains("architecture decision")
        || desc.contains("operator action") || desc.contains("operator decision")
    {
        return "engineering-lead".into();
    }

    // === CODER (default for everything that writes/edits source) ===
    // domain types, ports, use cases, adapters (primary + secondary),
    // STDB reducer code, frontend pages — all code.
    "hex-coder".into()
}

fn build_brief(workplan_id: &str, workplan_path: &Path, step: &Value, repo_root: &Path) -> String {
    let step_id = step.get("id").and_then(|v| v.as_str()).unwrap_or("???");
    let description = step.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let done_condition = step.get("done_condition").and_then(|v| v.as_str()).unwrap_or("");
    let files_all: Vec<&str> = step
        .get("files_to_create")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    // Split into MISSING and ALREADY-PRESENT so the brief drives the
    // persona toward the actual gap instead of the populated dir.
    let mut files: Vec<&str> = Vec::new();
    let mut present_files: Vec<&str> = Vec::new();
    for f in &files_all {
        let exists_nonempty = std::fs::metadata(repo_root.join(f))
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false);
        if exists_nonempty {
            present_files.push(*f);
        } else {
            files.push(*f);
        }
    }
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
    if !present_files.is_empty() {
        out.push_str(&format!(
            "ALREADY COMPLETE for this step — DO NOT rewrite these {} files:\n",
            present_files.len()
        ));
        for f in &present_files {
            out.push_str(&format!("- ✓ {} (already on disk, non-empty)\n", f));
        }
        out.push('\n');
    }
    if !files.is_empty() {
        out.push_str(&format!(
            "ONLY THESE {} FILE(S) ARE STILL MISSING — emit a code_patch for each:\n",
            files.len()
        ));
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

/// Escalate a step whose dispatch circuit has opened — the persona kept
/// producing an artifact that never reaches evidence-verified done. The
/// conductor will not re-dispatch it; a human (or a revised workplan) must
/// intervene.
async fn escalate_circuit(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    workplan_id: &str,
    step_id: &str,
    attempts: u64,
) {
    let content = format!(
        "Step `{}` of workplan `{}` was dispatched {} times without reaching \
         evidence-verified done. The conductor has OPENED its dispatch circuit and \
         will NOT re-dispatch it — this stops the persona re-hallucinating the same \
         broken artifact in a loop. Operator action: inspect the step's files + \
         `evidence` predicate, fix by hand or revise the workplan, then restart the \
         conductor to reset the circuit.",
        step_id, workplan_id, attempts
    );
    let _ = send_dm(
        http,
        nexus_base,
        "engineering-lead",
        &format!("CIRCUIT-OPEN: {} {}", workplan_id, step_id),
        &content,
    )
    .await;
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
    fn route_acceptance_to_tester() {
        let s = json!({"id":"step-30","description":"Acceptance test: end-to-end happy path","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-tester");
    }

    // The closed loop: files-present is necessary but not sufficient. A step's
    // `evidence` predicate decides done, so the gate goes RED on a failing
    // predicate and GREEN on a passing one — even with the same files present.
    #[tokio::test]
    async fn evidence_gate_red_on_fail_green_on_pass() {
        let root = std::env::temp_dir();
        let http = Arc::new(reqwest::Client::new());
        // Empty nexus_base => reward POST is skipped, keeping the test hermetic.
        let base = "";

        // GREEN: a passing predicate.
        let pass = json!({ "id": "ev-pass", "evidence": ["true"] });
        let (ok, reason) = step_evidence_gate(&http, base, "feat-test", "ev-pass", &pass, &root).await;
        assert!(ok, "passing predicate must be done");
        assert!(reason.is_none());

        // RED: a failing predicate — files could all exist and this is still
        // NOT done. This is the case that would have caught the stub router.
        let fail = json!({ "id": "ev-fail", "evidence": ["exit 1"] });
        let (ok, reason) = step_evidence_gate(&http, base, "feat-test", "ev-fail", &fail, &root).await;
        assert!(!ok, "failing predicate must NOT be done");
        assert!(reason.is_some(), "failure surfaces a reason for the operator");

        // Legacy: no evidence declared keeps files-exist semantics (vacuous pass).
        let none = json!({ "id": "ev-none" });
        let (ok, _) = step_evidence_gate(&http, base, "feat-test", "ev-none", &none, &root).await;
        assert!(ok, "no evidence => legacy files-exist semantics preserved");
    }

    // De-conflation: hex's conductor refuses workplans that build an example.
    #[test]
    fn conductor_skips_example_targeting_workplans() {
        let hex_wp = vec![
            json!({"id":"s1","files_to_create":["hex-nexus/src/foo.rs","hex-core/src/bar.rs"]}),
        ];
        assert!(!workplan_targets_examples(&hex_wp), "hex's own workplan must be driven");

        let example_wp = vec![
            json!({"id":"s1","files_to_create":["examples/ebay-clone/backend/src/main.rs"]}),
            json!({"id":"s2","files_to_create":["examples/ebay-clone/frontend/src/App.tsx"]}),
        ];
        assert!(workplan_targets_examples(&example_wp), "example workplan must be refused");

        // Empty / no files → not an example workplan (don't accidentally skip).
        assert!(!workplan_targets_examples(&[json!({"id":"s1"})]));
    }

    // Re-hallucination guard: the circuit opens once a step has burned its
    // dispatch budget, so the conductor stops re-dispatching (and escalates).
    #[test]
    fn dispatch_circuit_opens_at_max() {
        assert!(!is_circuit_open(0, 5));
        assert!(!is_circuit_open(4, 5), "under budget stays closed");
        assert!(is_circuit_open(5, 5), "at budget opens");
        assert!(is_circuit_open(99, 5), "over budget stays open");
        assert!(!is_circuit_open(1000, 0), "max=0 disables the breaker");
    }

    // Evidence-by-default: a code-producing step with no explicit `evidence`
    // derives a compile predicate for the owning crate; non-code derives none.
    #[test]
    fn derive_default_evidence_from_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("crateA/src/core")).unwrap();
        fs::write(root.join("crateA/Cargo.toml"), "[package]\nname=\"a\"\n").unwrap();

        // Two .rs files in the same crate collapse to one cargo-check predicate.
        let preds = derive_default_evidence(
            &["crateA/src/lib.rs".into(), "crateA/src/core/x.rs".into()],
            root,
        );
        assert_eq!(preds.len(), 1, "one crate => one predicate: {preds:?}");
        assert!(preds[0].contains("cd crateA"), "{preds:?}");
        assert!(preds[0].contains("cargo check"), "{preds:?}");

        // Docs/scripts derive nothing (keep files-exist semantics).
        assert!(derive_default_evidence(&["README.md".into(), "scripts/x.sh".into()], root).is_empty());

        // A .rs file with no ancestor Cargo.toml derives nothing.
        assert!(derive_default_evidence(&["nowhere/y.rs".into()], root).is_empty());
    }

    // The reward keying generalizes across steps of the same shape (tier+layer)
    // so the q-table learns routing quality, not per-step noise.
    #[test]
    fn evidence_state_key_encodes_tier_and_layer() {
        assert_eq!(
            evidence_state_key(&json!({"tier":2,"layer":"secondary"})),
            "evidence:tier2:secondary"
        );
        assert_eq!(evidence_state_key(&json!({"tier":6})), "evidence:tier6");
        assert_eq!(evidence_state_key(&json!({})), "evidence:step");
    }

    #[test]
    fn route_integration_tests_to_tester() {
        let s = json!({"id":"step-29","description":"Backend integration tests for the bidding pipeline","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-tester");
    }

    #[test]
    fn route_behavioral_specs_to_tester() {
        let s = json!({"id":"step-x","description":"Write behavioral specs for the auction module","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-tester");
    }

    #[test]
    fn route_composition_root_to_integrator() {
        let s = json!({"id":"step-21","description":"Composition root: main.rs + composition_root.rs","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "integrator");
    }

    #[test]
    fn route_start_sh_to_integrator() {
        let s = json!({"id":"step-31","description":"start.sh + README.md + docker-compose.yml","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "integrator");
    }

    #[test]
    fn route_readme_to_integrator() {
        let s = json!({"id":"step-x","description":"Write README.md and quickstart","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "integrator");
    }

    #[test]
    fn route_dead_code_to_reviewer() {
        let s = json!({"id":"step-x","description":"Hex analyze pass for dead code in the workspace","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-reviewer");
    }

    #[test]
    fn route_code_review_to_reviewer() {
        let s = json!({"id":"step-x","description":"Code review for the new auction aggregate","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-reviewer");
    }

    #[test]
    fn route_auth_step_NOT_to_security() {
        // Old router miss: "auth" in a domain step's description routed to
        // ciso. In the lean fleet, the lead-keywords are narrower and this
        // falls back to hex-coder (which is the right place for use case
        // code to live).
        let s = json!({"id":"step-17","description":"Use case: auth — register_user and login","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-coder");
    }

    #[test]
    fn route_register_user_NOT_to_security() {
        // Specifically the bug we caught in production: step-6 marketplace
        // reducer with "register_user" in the description routed to ciso.
        let s = json!({"id":"step-6","description":"Marketplace reducers (part 1): register_user and create_listing","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-coder");
    }

    #[test]
    fn route_domain_types_to_coder() {
        let s = json!({"id":"step-2","description":"Domain value types: newtypes for UserId","layer":"domain","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "hex-coder");
    }

    #[test]
    fn route_adr_drafting_to_lead() {
        let s = json!({"id":"step-x","description":"Draft an ADR for the new auth flow","files_to_create":["a"]});
        assert_eq!(route_step_to_persona(&s), "engineering-lead");
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
        let tmp = TempDir::new().unwrap();
        let brief = build_brief("feat-x", Path::new("docs/workplans/feat-x.json"), &step, tmp.path());
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
    fn build_brief_hides_already_present_files() {
        // Self-poisoning recursion fix: when files_to_create are mostly
        // on-disk already, the brief should NOT prompt the persona to
        // create them again — only mention the missing ones.
        let step = json!({
            "id": "step-29",
            "description": "Backend integration tests",
            "files_to_create": [
                "tests/integration_auth.rs",
                "tests/integration_bidding.rs",
                "tests/integration_listings.rs",
                "tests/integration_images.rs",
                "tests/common/mod.rs",
            ],
        });
        let tmp = TempDir::new().unwrap();
        // Pre-populate 4 of the 5 files.
        for name in &[
            "tests/integration_auth.rs",
            "tests/integration_bidding.rs",
            "tests/integration_images.rs",
            "tests/common/mod.rs",
        ] {
            let p = tmp.path().join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "// real content\n").unwrap();
        }
        let brief = build_brief(
            "feat-x",
            Path::new("docs/workplans/feat-x.json"),
            &step,
            tmp.path(),
        );

        // The brief must call out the ONE missing file as the gap.
        assert!(brief.contains("code_patch: create tests/integration_listings.rs"));
        assert!(brief.contains("ONLY THESE 1 FILE(S) ARE STILL MISSING"));

        // The brief MUST NOT prompt the persona to create the existing 4.
        assert!(!brief.contains("code_patch: create tests/integration_auth.rs"));
        assert!(!brief.contains("code_patch: create tests/integration_bidding.rs"));
        assert!(!brief.contains("code_patch: create tests/integration_images.rs"));
        assert!(!brief.contains("code_patch: create tests/common/mod.rs"));

        // It should still LIST the present ones as completed (so the
        // persona has full context without being asked to rewrite them).
        assert!(brief.contains("ALREADY COMPLETE for this step"));
        assert!(brief.contains("tests/integration_auth.rs"));
    }

    #[test]
    fn build_brief_with_no_files_present_lists_all_as_missing() {
        let step = json!({
            "id": "step-1",
            "description": "scaffold",
            "files_to_create": ["a.rs", "b.rs", "c.rs"],
        });
        let tmp = TempDir::new().unwrap();
        let brief = build_brief(
            "feat-y",
            Path::new("docs/workplans/feat-y.json"),
            &step,
            tmp.path(),
        );
        assert!(brief.contains("ONLY THESE 3 FILE(S)"));
        assert!(!brief.contains("ALREADY COMPLETE"));
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
