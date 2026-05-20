//! Sched commands (ADR-2026-04-10-2200).
//!
//! `hex sched status|test|scores|models|validate`
//!
//! status   - Show sched service status and configuration
//! test     - Run a manual test of a model
//! scores   - Show learned method scores
//! models   - List available models for sched selection
//! validate - Run self-diagnostics (CLI wiring, etc.)

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;

use tracing::debug;

use crate::fmt::{pretty_table, truncate};

use super::adr::doctor;

/// Self-improvement loop (ADR-2026-04-27-1100). P1.1 lands the discovery surface;
/// later tasks plug in variant generation, judging, and the tick handler.
pub mod improver;

/// Daemon-local state persisted across ticks (wp-brain-updates P1.2).
/// Tracks issue counts from the previous validate tick so regressions can
/// be detected — a count that increases tick-over-tick is a regression.
/// Persisted to `~/.hex/brain-state.json` so the baseline survives daemon
/// restarts (otherwise every restart would silently hide cross-restart
/// regressions by re-seeding from the current tick).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DaemonState {
    /// Last tick's issue counts keyed by check name
    /// (e.g. "cli_wiring" → 2, "mcp_parity" → 0, "workplans_stale" → 1).
    #[serde(default)]
    last_counts: HashMap<String, usize>,
    /// Whether we've observed at least one tick (first tick establishes baseline,
    /// no regression notification on the first tick).
    #[serde(default)]
    seeded: bool,
    /// Consecutive ticks the queue has been fully idle — both `pending` and
    /// `in_flight` (in_progress) at zero. Bumped by `queue_drain` each time it
    /// runs and sees the queue empty, reset the moment either counter is
    /// non-zero. Paired with `sched.idle_threshold_ticks` so downstream code
    /// can treat sustained idleness as a signal (e.g. wind down a worker).
    #[serde(default)]
    idle_tick_count: u32,
    /// Monotonic per-daemon-process tick counter. Used by the improver
    /// auto-act gate so a small action sweep fires only every N ticks
    /// (default 6 ≈ every 3 minutes at 30s interval). Persisted across
    /// restarts so a restart-loop can't repeatedly fire auto-act.
    #[serde(default)]
    tick_count: u32,
    /// Last time an analyze task was enqueued (RFC3339). Used to enforce the
    /// `brain.analyze_interval_secs` interval so we don't spam the queue.
    #[serde(default)]
    last_analyze_at: Option<String>,
    /// Summary of the last analysis result (violation counts by category).
    /// Stored for regression detection across daemon restarts.
    #[serde(default)]
    last_analysis_summary: HashMap<String, usize>,
    /// UTC date of last terminal-task sweep (YYYY-MM-DD). Prevents running the
    /// deletion every tick — only first tick of each UTC day actually deletes.
    #[serde(default)]
    sweep_date: String,
}

/// Per-tick state passed to sched daemon tick handlers (e.g. [`tick_adr_health`]).
/// Aliases [`DaemonState`] so the daemon's persisted state and the per-tick
/// handler signatures share a single type — adding a tick handler doesn't
/// require a parallel state struct.
pub type SchedState = DaemonState;

/// Default idle-tick threshold — after this many consecutive fully-idle
/// `queue_drain` calls the scheduler is "quiet". Override per-project in
/// `.hex/project.json` at `sched.idle_threshold_ticks`.
const DEFAULT_IDLE_THRESHOLD_TICKS: u32 = 4;

/// Default interval between periodic analyze tasks (1 hour).
const DEFAULT_ANALYZE_INTERVAL_SECS: u64 = 3600;

/// Default minimum interval between idle-triggered research sweeps, in hours
/// (wp-idle-research-swarm P1.2 / ADR-2026-04-15-1200). The throttle keeps the
/// idle-trigger from re-firing every queue_drain tick once the queue settles
/// — without it a quiet repo would self-enqueue research sweeps continuously.
const DEFAULT_MIN_SWEEP_INTERVAL_H: u64 = 6;

/// Read the minimum sweep-interval (in hours) from `.hex/project.json`
/// (`sched.min_sweep_interval_h`). Falls back to
/// [`DEFAULT_MIN_SWEEP_INTERVAL_H`] on any read/parse failure or when the key
/// is absent. Mirrors [`load_idle_threshold_ticks`] — the throttle should
/// degrade to its default rather than erroring on a missing config.
fn load_min_sweep_interval_h() -> u64 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return DEFAULT_MIN_SWEEP_INTERVAL_H,
    };
    let content = match std::fs::read_to_string(cwd.join(".hex/project.json")) {
        Ok(c) => c,
        Err(_) => return DEFAULT_MIN_SWEEP_INTERVAL_H,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(_) => return DEFAULT_MIN_SWEEP_INTERVAL_H,
    };
    parsed
        .get("sched")
        .and_then(|s| s.get("min_sweep_interval_h"))
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_MIN_SWEEP_INTERVAL_H)
}

/// Path to the persisted "last research-sweep" timestamp.
/// `~/.hex/sched/last_research_sweep` is a simple single-line RFC3339
/// timestamp file. Lives outside `brain-state.json` because operators may
/// want to rotate the throttle independently (e.g. `rm` to force a sweep).
fn last_research_sweep_path() -> PathBuf {
    sched_signal_dir().join("last_research_sweep")
}

/// Path to the persisted "last memory-health check" timestamp.
fn last_memory_health_check_path() -> PathBuf {
    sched_signal_dir().join("last_memory_health_check")
}

fn read_last_memory_health_check() -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = std::fs::read_to_string(last_memory_health_check_path()).ok()?;
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn write_last_memory_health_check(ts: chrono::DateTime<chrono::Utc>) {
    let path = last_memory_health_check_path();
    if let Err(e) = std::fs::create_dir_all(path.parent().unwrap()) {
        eprintln!("  {} last_memory_health_check dir: {}", "warn".yellow(), e);
    }
    if let Err(e) = std::fs::write(&path, ts.to_rfc3339()) {
        eprintln!("  {} last_memory_health_check write: {}", "warn".yellow(), e)
    }
}

fn should_enqueue_memory_health(
    last: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
    interval_h: u64,
) -> bool {
    match last {
        None => true, // Never run before
        Some(last_ts) => {
            let elapsed = now.signed_duration_since(last_ts);
            elapsed.num_hours() >= interval_h as i64
        }
    }
}

/// Sched coordination directory. Mirrors `default_signal_dir()` in
/// `hex-nexus/src/research/coordinator.rs` — both processes coordinate via
/// flat files under this directory (in-flight marker, abort signal, last
/// sweep). `hex-cli` doesn't depend on `hex-nexus`, so the wire format
/// (filenames + parent dir) is the contract; the matching constants below
/// must stay in lockstep with their nexus-side counterparts.
fn sched_signal_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".hex").join("sched")
}

/// Filename of the sweep in-flight marker. Must equal
/// `hex_nexus::research::coordinator::SWEEP_IN_FLIGHT_FILENAME`.
const SWEEP_IN_FLIGHT_FILENAME: &str = "sweep_in_flight";

/// Filename of the sweep abort signal. Must equal
/// `hex_nexus::research::coordinator::SWEEP_ABORT_FILENAME`.
const SWEEP_ABORT_FILENAME: &str = "sweep_abort";

fn sweep_in_flight_path() -> PathBuf {
    sched_signal_dir().join(SWEEP_IN_FLIGHT_FILENAME)
}

fn sweep_abort_path() -> PathBuf {
    sched_signal_dir().join(SWEEP_ABORT_FILENAME)
}

/// True iff the coordinator currently has a sweep in flight.
/// `hex-cli` reads this only — the marker is owned by the coordinator.
fn is_sweep_in_flight() -> bool {
    sweep_in_flight_path().exists()
}

/// Write the abort signal so the coordinator's per-analyst poll bails
/// cleanly at the next checkpoint (wp-idle-research-swarm P4.4).
/// Best-effort — a failed write logs and returns; worst case the next
/// drain tick re-tries. Idempotent: re-writing the same file is a no-op
/// from the coordinator's perspective.
fn request_sweep_abort() {
    let dir = sched_signal_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("  {} sweep-abort dir: {}", "warn".yellow(), e);
        return;
    }
    if let Err(e) = std::fs::write(sweep_abort_path(), chrono::Utc::now().to_rfc3339()) {
        eprintln!("  {} sweep-abort write: {}", "warn".yellow(), e);
    }
}

/// Delete the persisted "last research-sweep" timestamp so the throttle
/// gate (`sweep_throttle_elapsed`) returns true on the next idle window.
/// Used by the preemption path: when we abort an in-flight sweep we want
/// the next idle window to re-fire it, not wait six more hours.
fn clear_last_research_sweep() {
    let path = last_research_sweep_path();
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("  {} clear last_research_sweep: {}", "warn".yellow(), e);
        }
    }
}

/// Pure: should the daemon preempt the in-flight sweep?
///
/// Preemption fires iff a sweep is currently in-flight AND at least one
/// pending task has a kind other than `research-sweep`. Both gates are
/// required:
/// * Without the in-flight check we'd write an abort signal that would
///   sit on disk forever (no coordinator to consume it) and could
///   spuriously cancel the *next* sweep.
/// * Without the kind check, queueing a second research-sweep behind an
///   in-flight one would self-cancel — but we want concurrent sweeps to
///   coalesce via the throttle, not to abort each other.
fn should_preempt_sweep(pending_kinds: &[&str], sweep_in_flight: bool) -> bool {
    sweep_in_flight && pending_kinds.iter().any(|k| *k != "research-sweep")
}

/// Read the last research-sweep timestamp. Missing file or malformed
/// content returns `None` — both mean "never swept", so the throttle gate
/// allows the next sweep to fire.
fn read_last_research_sweep() -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = std::fs::read_to_string(last_research_sweep_path()).ok()?;
    let trimmed = raw.trim();
    chrono::DateTime::parse_from_rfc3339(trimmed)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Persist the last research-sweep timestamp. Best-effort — a failed write
/// is logged but does not abort the caller; we'd rather double-fire a sweep
/// than crash the drain loop.
fn write_last_research_sweep(ts: chrono::DateTime<chrono::Utc>) {
    let path = last_research_sweep_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("  {} last_research_sweep dir: {}", "warn".yellow(), e);
            return;
        }
    }
    if let Err(e) = std::fs::write(&path, ts.to_rfc3339()) {
        eprintln!("  {} last_research_sweep write: {}", "warn".yellow(), e);
    }
}

/// Minimal projection of `docs/analysis/idle-sweep-*.yaml` — only the fields
/// needed to format the operator-facing summary line. The full coordinator
/// document carries `analysts_run`, `analyst_errors`, etc.; we deserialize
/// just `sweep_at`, `findings_total`, and the `findings` array (so we can
/// count which ones produce drafts).
#[derive(serde::Deserialize)]
struct SweepDocSummary {
    sweep_at: String,
    #[serde(default)]
    findings_total: usize,
    #[serde(default)]
    findings: Vec<hex_core::Finding>,
}

/// Format a duration as a compact age — "12s", "5m", "3h", "2d". Mirrors
/// the convention `agent_audit::format_age` uses so the status panels read
/// uniformly. Negative or zero durations clamp to "0s" (clock skew between
/// sweep_at and now should never produce negative output).
fn format_sweep_age(elapsed: chrono::Duration) -> String {
    let secs = elapsed.num_seconds().max(0);
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// Pure: count findings whose `suggested_action.kind` produces an on-disk
/// draft. ADR / DraftWorkplan / AmendWorkplan all write a file under
/// `docs/{adrs,workplans}/drafts/`; Memory and Informational do not. Keep
/// this enum match exhaustive — adding a new draft-producing kind without
/// updating it would silently undercount the operator-facing draft total.
fn count_drafts(findings: &[hex_core::Finding]) -> usize {
    findings
        .iter()
        .filter(|f| {
            matches!(
                f.suggested_action.kind,
                hex_core::ActionKind::DraftWorkplan
                    | hex_core::ActionKind::AmendWorkplan
                    | hex_core::ActionKind::DraftAdr
            )
        })
        .count()
}

/// Pure: format the `last_sweep:` line body — `"<age> (<n> findings, <m> drafts)"` —
/// from a parsed sweep document. Returns `None` when `sweep_at` is unparseable
/// (treat a malformed YAML as "no last sweep" rather than crash the status
/// panel).
fn format_last_sweep_line(
    doc: &SweepDocSummary,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    let sweep_at = chrono::DateTime::parse_from_rfc3339(&doc.sweep_at)
        .ok()?
        .with_timezone(&chrono::Utc);
    let age = format_sweep_age(now.signed_duration_since(sweep_at));
    // Operators care about what the sweep *found*; `findings_total` is the
    // pre-cap count, `findings.len()` is what actually got serialized. Pre-cap
    // is the more honest "n findings" — a low cap shouldn't make a noisy repo
    // look quiet. Fall back to `findings.len()` when `findings_total` is
    // missing or smaller (older YAMLs without the field).
    let n_findings = doc.findings_total.max(doc.findings.len());
    let n_drafts = count_drafts(&doc.findings);
    Some(format!(
        "{} ago ({} findings, {} drafts)",
        age, n_findings, n_drafts
    ))
}

/// Locate the newest `idle-sweep-*.yaml` under `<repo_root>/docs/analysis/`.
/// Filename stem is `idle-sweep-YYYYMMDD-HHMM`, so lexicographic ordering
/// matches chronological ordering — no need to stat each file. Returns
/// `None` when the directory or any matching file is missing.
fn find_latest_sweep_yaml(repo_root: &std::path::Path) -> Option<PathBuf> {
    let analysis_dir = repo_root.join("docs").join("analysis");
    let entries = std::fs::read_dir(&analysis_dir).ok()?;
    let mut latest: Option<(String, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("idle-sweep-") || !name_str.ends_with(".yaml") {
            continue;
        }
        let owned = name_str.into_owned();
        match &latest {
            Some((cur, _)) if cur >= &owned => {}
            _ => latest = Some((owned, entry.path())),
        }
    }
    latest.map(|(_, p)| p)
}

/// Read & summarize the most recent idle-sweep YAML under `repo_root` for
/// the operator status surfaces (`hex sched daemon status`, `hex` no-arg
/// panel — wp-idle-research-swarm P5.1). Returns `None` when no sweep has
/// run yet or the YAML can't be parsed; callers omit the line entirely in
/// that case rather than printing a placeholder.
pub(crate) fn last_sweep_summary_line(repo_root: &std::path::Path) -> Option<String> {
    let path = find_latest_sweep_yaml(repo_root)?;
    let body = std::fs::read_to_string(&path).ok()?;
    let doc: SweepDocSummary = serde_yaml::from_str(&body).ok()?;
    format_last_sweep_line(&doc, chrono::Utc::now())
}

/// Pure: has at least `interval_h` hours elapsed since `last_sweep`?
/// `None` (never swept) is treated as eligible — the first sweep should
/// fire as soon as the idle threshold is reached.
fn sweep_throttle_elapsed(
    last_sweep: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
    interval_h: u64,
) -> bool {
    match last_sweep {
        None => true,
        Some(last) => {
            let elapsed = now.signed_duration_since(last);
            elapsed.num_seconds() >= (interval_h as i64) * 3600
        }
    }
}

/// Pure: combined gate for the idle-research trigger
/// (wp-idle-research-swarm P1.2). Fires only when the queue has been idle
/// for at least `threshold` ticks AND the last sweep is older than
/// `interval_h` hours. Both gates are required — idleness alone would spam
/// the queue, throttle alone would never fire on a busy repo.
fn should_self_enqueue_research_sweep(
    idle_ticks: u32,
    threshold: u32,
    last_sweep: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
    interval_h: u64,
) -> bool {
    idle_ticks >= threshold && sweep_throttle_elapsed(last_sweep, now, interval_h)
}

/// Read the idle-tick threshold from `.hex/project.json` (key
/// `sched.idle_threshold_ticks`). Falls back to [`DEFAULT_IDLE_THRESHOLD_TICKS`]
/// on any read/parse failure or when the key is absent — the feature degrades
/// to its default rather than erroring on a missing config.
fn load_idle_threshold_ticks() -> u32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return DEFAULT_IDLE_THRESHOLD_TICKS,
    };
    let content = match std::fs::read_to_string(cwd.join(".hex/project.json")) {
        Ok(c) => c,
        Err(_) => return DEFAULT_IDLE_THRESHOLD_TICKS,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(_) => return DEFAULT_IDLE_THRESHOLD_TICKS,
    };
    parsed
        .get("sched")
        .and_then(|s| s.get("idle_threshold_ticks"))
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_IDLE_THRESHOLD_TICKS)
}

fn brain_state_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".hex").join("brain-state.json")
}

/// Load persisted daemon state. A missing / unreadable / malformed file
/// returns default state — we never want a corrupt state file to crash the
/// daemon. Returns fresh default on any error.
fn load_daemon_state() -> DaemonState {
    let path = brain_state_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => DaemonState::default(),
    }
}

/// Persist daemon state. Best-effort — a failed write is logged but does
/// not abort the tick; we'd rather drop the baseline than stop the loop.
fn save_daemon_state(state: &DaemonState) {
    let path = brain_state_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("  {} brain-state dir: {}", "warn".yellow(), e);
            return;
        }
    }
    match serde_json::to_string_pretty(state) {
        Ok(body) => {
            if let Err(e) = std::fs::write(&path, body) {
                eprintln!("  {} brain-state write: {}", "warn".yellow(), e);
            }
        }
        Err(e) => eprintln!("  {} brain-state encode: {}", "warn".yellow(), e),
    }
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

#[derive(Subcommand)]
pub enum BrainAction {
    /// Show sched service status and configuration
    Status,
    /// Run a test with a specific model
    Test {
        /// Model name (e.g. nemotron-mini, qwen3:8b)
        #[arg(default_value = "nemotron-mini")]
        model: String,
    },
    /// Show learned method scores from RL engine
    Scores,
    /// List models available for sched selection
    Models,
    /// Run self-diagnostics (CLI wiring check, etc.)
    Validate,
    /// Run the sched supervisor loop — validates + auto-fixes every interval (ADR-2026-04-13-2300)
    Daemon {
        /// Tick interval in seconds (default 10)
        #[arg(long, default_value = "10")]
        interval: u64,
        /// Max consecutive failures before pausing (default 3)
        #[arg(long, default_value = "3")]
        max_failures: u32,
        /// Run in background (spawn child process + PID file)
        #[arg(long)]
        background: bool,
    },
    /// Stop the background sched daemon
    DaemonStop,
    /// Restart the background daemon (stop + start with current interval).
    /// Use this after rebuilding hex-cli to pick up new code.
    DaemonRestart,
    /// Show sched daemon status (running/stopped)
    DaemonStatus,
    /// Enqueue a task for the sched daemon (ADR-2026-04-13-2330)
    Enqueue {
        /// Task kind (hex-command, workplan, shell)
        kind: String,
        /// Task payload (command args, workplan path, or shell command)
        payload: String,
        /// Scheduling priority (0 = normal, higher = more urgent). The daemon
        /// drains pending tasks in priority-desc, created_at-asc order so a
        /// `--priority 9` task jumps ahead of normal-priority backlog without
        /// needing the queue to drain first. Useful when local Ollama is
        /// saturated and a frontier-bound urgent task needs to bypass the
        /// speculative loops.
        #[arg(long, default_value = "0")]
        priority: u8,
    },
    /// Manage the sched task queue
    Queue {
        #[command(subcommand)]
        action: QueueAction,
    },
    /// Watch brain_tick events as they arrive (wp-brain-updates P3.1).
    /// Polls GET /api/events every 2s, filters for brain_tick, prints new events.
    /// Exits on Ctrl-C.
    Watch {
        /// ISO 8601 timestamp (e.g. "2026-04-14T10:00:00Z") — only show events
        /// newer than this. Omit to watch from the current moment forward.
        #[arg(long)]
        since: Option<String>,
    },
    /// Prime sched for this project: start daemon if needed, discover active
    /// workplans in docs/workplans/, and seed the queue in one shot.
    Prime {
        /// Tick interval when starting the daemon (default 10s)
        #[arg(long, default_value = "10")]
        interval: u64,
    },
    /// Self-improvement loop (ADR-2026-04-27-1100) — discovery, judging, act.
    /// Operator-facing preview surface for what the autonomous loop would
    /// propose; later phases plug in variant generation and act().
    Improver {
        #[command(subcommand)]
        action: improver::ImproverAction,
    },
}

#[derive(Subcommand)]
pub enum QueueAction {
    /// List sched tasks (defaults to pending; use --include to widen)
    List {
        /// Comma-separated statuses to include: pending, completed, failed, all
        /// (default: pending).
        #[arg(long, default_value = "pending")]
        include: String,
        /// Only show tasks newer than this duration (e.g. 1h, 30m, 2d, 7d).
        #[arg(long)]
        since: Option<String>,
    },
    /// Clear completed/failed tasks
    Clear,
    /// Force drain and execute pending tasks now
    Drain,
    /// Show recent sched tasks across all statuses (wp-sched-queue-history P1.3).
    ///
    /// Primary use: verify the ADR-2026-04-14-1400 §1 P1 evidence-guard correctly
    /// flipped silent-drain workplans to `failed`. Filter with `--status failed`
    /// to see only guard-flipped tasks; the result column shows the
    /// `no git evidence` marker produced by `check_evidence`.
    History {
        /// Filter by exact status ("pending", "in_progress", "completed", "failed").
        /// Omit to include all statuses.
        #[arg(long)]
        status: Option<String>,
        /// Maximum rows to show (clamped server-side to [1, 200]).
        #[arg(long, default_value = "20")]
        limit: u32,
    },
}

pub async fn run(action: BrainAction) -> anyhow::Result<()> {
    match action {
        BrainAction::Status => status().await,
        BrainAction::Test { model } => test(&model).await,
        BrainAction::Scores => scores().await,
        BrainAction::Models => models().await,
        BrainAction::Validate => validate(false).await,
        BrainAction::Daemon { interval, max_failures, background } => {
            if background {
                daemon_background(interval, max_failures)
            } else {
                daemon(interval, max_failures).await
            }
        }
        BrainAction::DaemonStop => daemon_stop(),
        BrainAction::DaemonRestart => daemon_restart(),
        BrainAction::DaemonStatus => daemon_status(),
        BrainAction::Enqueue { kind, payload, priority } => {
            let id = enqueue_brain_task_with_priority(&kind, &payload, priority).await?;
            let priority_tag = if priority > 0 { format!(" priority={priority}") } else { String::new() };
            println!("⬡ enqueued sched task {id} ({kind}: {payload}){priority_tag}");
            Ok(())
        }
        BrainAction::Queue { action } => match action {
            QueueAction::List { include, since } => queue_list(&include, since.as_deref()).await,
            QueueAction::Clear => queue_clear().await,
            QueueAction::Drain => queue_drain().await,
            QueueAction::History { status, limit } => queue_history(status, limit).await,
        },
        BrainAction::Prime { interval } => prime(interval).await,
        BrainAction::Watch { since } => watch(since).await,
        BrainAction::Improver { action } => improver::run(action).await,
    }
}

async fn status() -> anyhow::Result<()> {
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    
    // Scope to current project so the queue counts reflect only this repo's
    // enqueued work. Without a .hex/project.json in cwd we fall back to the
    // unscoped endpoint (useful for operator views on hex-intf itself).
    // project_id is a UUID → safe as a URL query value without encoding.
    let url = match brain_project_id() {
        Some(pid) => format!("{}/api/sched/status?project={}", base_url, pid),
        None => format!("{}/api/sched/status", base_url),
    };
    let resp = client.get(&url).send().await?;
    
    if resp.status() == 404 {
        println!("{}", "Sched service not configured. Run hex-nexus with sched service enabled.".yellow());
        return Ok(());
    }
    
    if !resp.status().is_success() {
        eprintln!("Error: {}", resp.status());
        return Ok(());
    }
    
    let body: serde_json::Value = resp.json().await?;
    println!("{}", "Sched Service Status".green().bold());
    println!("  Service: {}", body.get("service_enabled").unwrap_or(&json!(false)));
    println!("  Test Model: {}", body.get("test_model").unwrap_or(&json!("nemotron-mini")));
    println!("  Interval: {} seconds", body.get("interval_secs").unwrap_or(&json!(10)));
    println!("  Last Test: {}", body.get("last_test").unwrap_or(&json!("never")));
    let pending = body.get("queue_pending").and_then(|v| v.as_u64()).unwrap_or(0);
    let running = body.get("queue_running").and_then(|v| v.as_u64()).unwrap_or(0);
    let queue_label = match (pending, running) {
        (0, 0) => "0 (idle)".dimmed().to_string(),
        (p, 0) => format!("{} pending {}", p, "⤵".cyan()),
        (0, r) => format!("{} running {}", r, "▶".green()),
        (p, r) => format!(
            "{} running {} · {} pending {}",
            r,
            "▶".green(),
            p,
            "⤵".cyan()
        ),
    };
    println!("  Queue:     {}", queue_label);

    if let Some(current) = body.get("current_task") {
        let id = current.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let kind = current.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let payload = current.get("payload").and_then(|v| v.as_str()).unwrap_or("");
        if !id.is_empty() {
            println!(
                "  Current:   {} {} {} {}",
                "▶".green(),
                &id[..id.len().min(8)],
                format!("({})", kind).dimmed(),
                truncate(payload, 60)
            );
        }
    }

    Ok(())
}

async fn test(model: &str) -> anyhow::Result<()> {
    println!("Testing model: {}", model.green());
    
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    
    let url = format!("{}/api/sched/test", base_url);
    let body = json!({ "model": model });
    let resp = client.post(&url).json(&body).send().await?;
    
    if !resp.status().is_success() {
        eprintln!("Test failed: {}", resp.status());
        let err: serde_json::Value = resp.json().await.unwrap_or_default();
        eprintln!("{}", err);
        return Ok(());
    }
    
    let result: serde_json::Value = resp.json().await?;
    println!("{}", "Test Result".green().bold());
    println!("  Outcome: {}", result.get("outcome").unwrap_or(&json!("unknown")));
    println!("  Reward: {}", result.get("reward").unwrap_or(&json!(0.0)));
    println!("  Response: {}", truncate(&result.get("response").unwrap_or(&json!("")).to_string(), 200));
    
    Ok(())
}

async fn scores() -> anyhow::Result<()> {
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    
    let url = format!("{}/api/sched/scores", base_url);
    let resp = client.get(&url).send().await?;
    
    if resp.status() == 404 {
        println!("{}", "No scores yet. Sched service is learning.".yellow());
        return Ok(());
    }
    
    if !resp.status().is_success() {
        eprintln!("Error: {}", resp.status());
        return Ok(());
    }
    
    let scores: Vec<serde_json::Value> = resp.json().await?;
    
    if scores.is_empty() {
        println!("{}", "No scores recorded yet.".yellow());
        return Ok(());
    }
    
    println!("{}", "Method Scores".green().bold());
    let rows: Vec<Vec<String>> = scores
        .iter()
        .map(|score| {
            vec![
                score.get("method").unwrap_or(&json!("")).to_string(),
                format!("{:.3}", score.get("q_value").unwrap_or(&json!(0.0))),
                score.get("visit_count").unwrap_or(&json!(0)).to_string(),
            ]
        })
        .collect();
    println!("{}", pretty_table(&["Method", "Score", "Visits"], &rows));
    
    Ok(())
}

/// Inspect the hex-cli source tree at runtime and return module names that have a
/// `.rs` file in `commands/` but are missing from either `mod.rs` or `main.rs`.
fn check_cli_wiring() -> anyhow::Result<Vec<String>> {
    use std::collections::HashSet;

    // Locate hex-cli/src/commands/ relative to the cargo manifest dir at build time,
    // but we read files at *runtime* — so derive from the binary's own source tree.
    // The binary may be running from any cwd, so we locate the source via CARGO_MANIFEST_DIR
    // baked at compile time.
    let cli_src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let commands_dir = format!("{}/commands", cli_src);
    let mod_rs_path = format!("{}/commands/mod.rs", cli_src);
    let main_rs_path = format!("{}/main.rs", cli_src);

    // 1. Glob all .rs files in commands/ (excluding mod.rs)
    let mut file_modules = HashSet::new();
    for entry in std::fs::read_dir(&commands_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".rs") && name != "mod.rs" {
            file_modules.insert(name.trim_end_matches(".rs").to_string());
        }
    }

    // 2. Parse mod.rs for `pub mod <name>` entries
    let mod_rs = std::fs::read_to_string(&mod_rs_path)?;
    let mut mod_entries = HashSet::new();
    for line in mod_rs.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("pub mod ") {
            if let Some(name) = rest.strip_suffix(';') {
                mod_entries.insert(name.trim().to_string());
            }
        }
    }

    // 3. Parse main.rs for modules referenced in `use commands::{...}` block
    //    and in `commands::X::run()` match arms.
    let main_rs = std::fs::read_to_string(&main_rs_path)?;
    let mut main_entries = HashSet::new();
    let mut in_use_block = false;
    for line in main_rs.lines() {
        let trimmed = line.trim();
        // Detect `use commands::{` block
        if trimmed.starts_with("use commands::{") {
            in_use_block = true;
            continue;
        }
        if in_use_block {
            if trimmed.contains('}') {
                in_use_block = false;
                continue;
            }
            // Lines like `adr::AdrAction,` or `analyze,`
            let seg = trimmed.split("::").next().unwrap_or("")
                .trim_end_matches([',', ';', '{', '}'])
                .trim();
            if !seg.is_empty() {
                main_entries.insert(seg.to_string());
            }
            continue;
        }
        // Also catch `commands::X::run(action)` in match arms
        if let Some(rest) = trimmed.strip_prefix("commands::") {
            let seg = rest.split("::").next().unwrap_or("")
                .trim_end_matches([',', ';', '(', '{'])
                .trim();
            if !seg.is_empty() {
                main_entries.insert(seg.to_string());
            }
        }
    }

    // 4. Find modules with a .rs file but missing from mod.rs OR main.rs
    let mut unwired: Vec<String> = file_modules
        .iter()
        .filter(|m| !mod_entries.contains(m.as_str()) || !main_entries.contains(m.as_str()))
        .cloned()
        .collect();
    unwired.sort();
    Ok(unwired)
}

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
    use super::plan::reconcile_evidence::{
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

/// Record the daemon validate outcome as an RL reward to SpacetimeDB.
///
/// +0.5 for a clean validate pass, +0.1 for passing with regressions
/// (still better than a crash), -0.5 for a validate failure.
/// This closes the daemon self-improvement loop: each tick is a training step.
async fn record_daemon_reward(validate_ok: bool, has_regressions: bool) {
    let reward = if validate_ok && !has_regressions {
        0.5 // clean pass — reward the daemon for doing nothing wrong
    } else if validate_ok && has_regressions {
        0.1 // regressions found — slight reward for still running
    } else {
        -0.5 // validate crashed or errored — penalize
    };

    let body = serde_json::json!({
        "state_key": "daemon:validate",
        "action": "daemon:tick",
        "reward": reward,
        "next_state_key": "daemon:validate",
        "rate_limited": false,
        "openrouter_cost_usd": 0.0,
        "task_type": "daemon",
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });

    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let url = format!("http://127.0.0.1:{}/api/rl/reward", port);

    if let Err(e) = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        eprintln!("  {} daemon RL reward POST failed: {}", "✗".red(), e);
    }
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

/// Auto-fix: remove a stale worktree. Returns true on success.
fn autofix_worktree(wt: &StaleWorktree) -> bool {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    std::process::Command::new("git")
        .args(["worktree", "remove", &wt.path])
        .current_dir(&workspace_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn validate(dry_run: bool) -> anyhow::Result<()> {
    println!("{}", "⬡ hex sched validate".bold());

    // CLI wiring check
    let cli_src = concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands");
    let total = std::fs::read_dir(cli_src)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".rs") && name != "mod.rs"
        })
        .count();

    match check_cli_wiring() {
        Ok(unwired) if unwired.is_empty() => {
            println!(
                "  CLI wiring:  {} {}/{} modules registered",
                "✓".green(),
                total,
                total
            );
        }
        Ok(unwired) => {
            println!(
                "  CLI wiring:  {} {} unwired modules: {:?}",
                "✗".red(),
                unwired.len(),
                unwired
            );
        }
        Err(e) => {
            println!("  CLI wiring:  {} error: {}", "✗".red(), e);
        }
    }

    // Binary freshness check (auto-fix: rebuild is already spawned by check_binary_freshness)
    match check_binary_freshness() {
        FreshnessStatus::Fresh => {
            println!("  Binary:      {} release binary is up-to-date", "✓".green());
        }
        FreshnessStatus::Stale {
            binary_age_secs,
            commit_age_secs,
        } => {
            let delta = commit_age_secs.saturating_sub(binary_age_secs);
            println!(
                "  Binary:      {} stale by ~{}s — background rebuild spawned {}",
                "✗".red(),
                delta,
                "[auto-fixed]".cyan()
            );
        }
        FreshnessStatus::Missing => {
            println!(
                "  Binary:      {} target/release/hex not found (run cargo build --release)",
                "✗".red()
            );
        }
        FreshnessStatus::Unknown(reason) => {
            println!("  Binary:      {} unknown: {}", "?".yellow(), reason);
        }
    }

    // Workplan status check (auto-fix: reconcile stale tasks to "done")
    match check_workplan_status() {
        Ok(summaries) if summaries.is_empty() => {
            println!("  Workplans:   {} no active workplans", "✓".green());
        }
        Ok(summaries) => {
            let total_stale: usize = summaries.iter().map(|s| s.stale_tasks.len()).sum();
            let mut total_fixed = 0usize;

            // Auto-fix: reconcile stale tasks whose git evidence proves completion.
            // In dry_run mode (daemon tick), only report candidates — never mutate
            // workplan JSON (ADR-2026-04-14-2201, wp-reconcile-evidence-verification R2.2).
            if total_stale > 0 && !dry_run {
                for wp in &summaries {
                    match autofix_workplan(wp) {
                        Ok(n) => total_fixed += n,
                        Err(e) => {
                            eprintln!("    auto-fix error for {}: {}", wp.id, e);
                        }
                    }
                }
            }

            if total_stale == 0 {
                println!(
                    "  Workplans:   {} {} active, all tasks consistent",
                    "✓".green(),
                    summaries.len()
                );
            } else if dry_run {
                println!(
                    "  Workplans:   {} {} active, {} stale tasks {}",
                    "⚠".yellow(),
                    summaries.len(),
                    total_stale,
                    "[dry-run, would reconcile]".cyan()
                );
            } else if total_fixed == total_stale {
                println!(
                    "  Workplans:   {} {} active, reconciled {} stale tasks {}",
                    "✓".green(),
                    summaries.len(),
                    total_fixed,
                    "[auto-fixed]".cyan()
                );
            } else {
                println!(
                    "  Workplans:   {} {} active, {}/{} stale tasks reconciled{}",
                    "✗".red(),
                    summaries.len(),
                    total_fixed,
                    total_stale,
                    if total_fixed > 0 { " [partial auto-fix]" } else { "" }
                );
            }
            for wp in &summaries {
                let effective_done = if dry_run {
                    wp.done_tasks
                } else {
                    wp.done_tasks + wp.stale_tasks.len()
                };
                let progress = if wp.total_tasks > 0 {
                    format!("{}/{}", effective_done, wp.total_tasks)
                } else {
                    "0/0".to_string()
                };
                let stale_note = if wp.stale_tasks.is_empty() {
                    String::new()
                } else if dry_run {
                    format!(
                        " — would reconcile: {}",
                        wp.stale_tasks.join(", ")
                    )
                } else {
                    format!(
                        " — reconciled: {} {}",
                        wp.stale_tasks.join(", "),
                        "[auto-fixed]".cyan()
                    )
                };
                let label = if wp.feature.is_empty() {
                    wp.id.clone()
                } else {
                    format!("{} ({})", wp.id, truncate(&wp.feature, 30))
                };
                println!(
                    "    {} [{}] {} tasks{}",
                    label,
                    progress,
                    wp.status,
                    stale_note
                );
            }
        }
        Err(e) => {
            println!("  Workplans:   {} error: {}", "✗".red(), e);
        }
    }

    // MCP ↔ CLI parity check
    match check_mcp_cli_parity() {
        Ok(orphans) if orphans.is_empty() => {
            println!(
                "  MCP parity:  {} all MCP tools have CLI equivalents",
                "✓".green()
            );
        }
        Ok(orphans) => {
            println!(
                "  MCP parity:  {} {} tools without CLI commands:",
                "✗".red(),
                orphans.len()
            );
            for orphan in &orphans {
                println!("    - {}", orphan);
            }
        }
        Err(e) => {
            println!("  MCP parity:  {} error: {}", "✗".red(), e);
        }
    }

    // Inference health check — check all registered endpoints via hex-nexus
    match check_inference_health().await {
        Ok(results) if results.is_empty() => {
            println!("  Inference:   {} no endpoints registered", "✓".green());
        }
        Ok(results) => {
            let healthy = results.iter().filter(|r| r.status == "healthy").count();
            let total = results.len();
            if healthy == total {
                println!("  Inference:   {} {}/{} endpoints healthy", "✓".green(), healthy, total);
            } else {
                println!("  Inference:   {} {}/{} healthy", "⚠".yellow(), healthy, total);
                for r in &results {
                    let icon = if r.status == "healthy" { "✓" } else { "✗" };
                    println!("    {} {} ({})", icon, r.id, r.status);
                }
            }
        }
        Err(e) => {
            println!("  Inference:   {} error: {}", "✗".red(), e);
        }
    }

    // Stale worktree check (auto-fix: remove worktrees with no commits for 24h+)
    match check_stale_worktrees() {
        Ok(stale) if stale.is_empty() => {
            println!("  Worktrees:   {} no stale worktrees", "✓".green());
        }
        Ok(stale) => {
            let mut removed = 0usize;
            for wt in &stale {
                if autofix_worktree(wt) {
                    removed += 1;
                }
            }
            if removed == stale.len() {
                println!(
                    "  Worktrees:   {} removed {} stale worktrees {}",
                    "✓".green(),
                    removed,
                    "[auto-fixed]".cyan()
                );
            } else {
                println!(
                    "  Worktrees:   {} {}/{} stale worktrees removed{}",
                    "✗".red(),
                    removed,
                    stale.len(),
                    if removed > 0 { " [partial auto-fix]" } else { "" }
                );
            }
            for wt in &stale {
                let hours = wt.age_secs / 3600;
                println!("    {} ({}h stale on {})", wt.path, hours, wt.branch);
            }
        }
        Err(e) => {
            println!("  Worktrees:   {} error: {}", "✗".red(), e);
        }
    }

    // Stale swarm check (ADR-2026-04-14-2300): active swarms whose tasks are all done
    // but status still "active" — auto-complete them via PATCH /api/swarms/:id.
    match check_stale_swarms().await {
        Ok(stale) if stale.is_empty() => {
            println!("  Swarms:      {} no stale swarms", "✓".green());
        }
        Ok(stale) => {
            let total = stale.len();
            let mut completed = 0usize;
            for s in &stale {
                if autofix_stale_swarm(&s.id).await {
                    completed += 1;
                }
            }
            if completed == total {
                println!(
                    "  Swarms:      {} auto-completed {} stale swarms {}",
                    "✓".green(),
                    completed,
                    "[auto-fixed]".cyan()
                );
            } else {
                println!(
                    "  Swarms:      {} {}/{} stale swarms completed{}",
                    "✗".red(),
                    completed,
                    total,
                    if completed > 0 { " [partial auto-fix]" } else { "" }
                );
            }
            for s in &stale {
                println!(
                    "    {} {} ({}/{} tasks done)",
                    truncate(&s.id, 8),
                    truncate(&s.name, 40),
                    s.completed_tasks,
                    s.total_tasks
                );
            }
        }
        Err(e) => {
            println!("  Swarms:      {} error: {}", "✗".red(), e);
        }
    }

    Ok(())
}

/// Prime sched for this project in one shot: ensure the daemon is running,
/// discover active workplans, and seed the queue. Idempotent — safe to re-run.
async fn prime(interval: u64) -> anyhow::Result<()> {
    println!("{}", "⬡ hex sched prime".bold());

    // 1. Daemon — start if not running
    match read_pid_file() {
        Some(pid) if process_alive(pid) => {
            println!("  Daemon:    {} already running pid={}", "✓".green(), pid);
        }
        _ => {
            remove_pid_file();
            daemon_background(interval, 3)?;
            println!("  Daemon:    {} started (interval={}s)", "✓".green(), interval);
        }
    }

    // 2. Discover workplans in docs/workplans/*.json — include any with
    //    status not in {"done", "completed", "abandoned"}.
    let mut discovered: Vec<PathBuf> = Vec::new();
    let wp_dir = PathBuf::from("docs/workplans");
    if wp_dir.is_dir() {
        for entry in std::fs::read_dir(&wp_dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            let Ok(json): Result<serde_json::Value, _> = serde_json::from_str(&content) else { continue };
            let status = json.get("status").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            if matches!(status.as_str(), "done" | "completed" | "abandoned") {
                continue;
            }
            discovered.push(path);
        }
    }

    if discovered.is_empty() {
        println!("  Workplans: {} no active workplans to enqueue", "✓".green());
    } else {
        println!("  Workplans: {} {} active, enqueuing...", "✓".green(), discovered.len());
    }

    // 3. Avoid duplicates — skip if already pending or in-progress FOR THIS
    //    PROJECT. Tasks for other projects (or legacy unscoped tasks with
    //    empty project_id) don't count as duplicates of ours.
    let this_project = brain_project_id().unwrap_or_default();
    let existing = list_brain_tasks(None).await.unwrap_or_default();
    let existing_paths: std::collections::HashSet<String> = existing
        .iter()
        .filter_map(|t| {
            let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if !matches!(status, "pending" | "in_progress") {
                return None;
            }
            let task_project = t
                .get("project_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if task_project != this_project {
                return None;
            }
            t.get("payload").and_then(|v| v.as_str()).map(String::from)
        })
        .collect();

    let mut enqueued = 0usize;
    let mut skipped = 0usize;
    for path in &discovered {
        let payload = path.to_string_lossy().to_string();
        if existing_paths.contains(&payload) {
            skipped += 1;
            continue;
        }
        match enqueue_brain_task("workplan", &payload).await {
            Ok(id) => {
                enqueued += 1;
                println!("    {} {} ({})", "+".green(), truncate(&payload, 50), &id[..8]);
            }
            Err(e) => {
                eprintln!("    {} {}: {}", "✗".red(), payload, e);
            }
        }
    }

    println!(
        "  Queue:     {} enqueued {}, skipped {} (already queued)",
        "✓".green(),
        enqueued,
        skipped
    );
    Ok(())
}

/// A swarm whose tasks are all completed but whose status is still `active`.
#[derive(Debug)]
struct StaleSwarm {
    id: String,
    name: String,
    total_tasks: usize,
    completed_tasks: usize,
}

/// Identify stale swarms by querying `/api/swarms/active` and filtering to
/// those where every task has status `completed`. Empty-task swarms are
/// excluded — they may be freshly initialized and still expecting tasks.
async fn check_stale_swarms() -> anyhow::Result<Vec<StaleSwarm>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let resp = client
        .get("http://127.0.0.1:5555/api/swarms/active")
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    let swarms = body.as_array().cloned().unwrap_or_default();

    let mut stale = Vec::new();
    for s in &swarms {
        if s["status"].as_str() != Some("active") {
            continue;
        }
        let id = s["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let name = s["name"].as_str().unwrap_or("").to_string();
        let tasks = s["tasks"].as_array();
        let total = tasks.map(|t| t.len()).unwrap_or(0);
        // Exclude empty-task swarms — might be freshly initialized.
        if total == 0 {
            continue;
        }
        let completed = tasks
            .map(|t| t.iter().filter(|tk| tk["status"].as_str() == Some("completed")).count())
            .unwrap_or(0);
        if completed == total {
            stale.push(StaleSwarm {
                id,
                name,
                total_tasks: total,
                completed_tasks: completed,
            });
        }
    }
    Ok(stale)
}

/// Mark a stale swarm complete via `PATCH /api/swarms/:id`. Returns `true`
/// on success. Respects `HEX_BRAIN_DRY_RUN=1` (ADR-2026-04-14-2300 safety mitigation).
async fn autofix_stale_swarm(id: &str) -> bool {
    if std::env::var("HEX_BRAIN_DRY_RUN").as_deref() == Ok("1") {
        return false;
    }
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .patch(&format!("http://127.0.0.1:5555/api/swarms/{}", id))
        .json(&json!({}))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Result of a single inference endpoint health check.
#[derive(Debug)]
struct InferenceHealthResult {
    id: String,
    status: String,
    #[allow(dead_code)]
    url: String,
}

/// Check health of all registered inference endpoints via hex-nexus.
async fn check_inference_health() -> anyhow::Result<Vec<InferenceHealthResult>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;

    let resp = client
        .post("http://127.0.0.1:5555/api/inference/health")
        .send()
        .await?;

    let body: serde_json::Value = resp.json().await?;
    let results = body
        .get("results")
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow::anyhow!("missing 'results' in health response"))?;

    let mut output = Vec::new();
    for r in results {
        output.push(InferenceHealthResult {
            id: r.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
            status: r.get("status").and_then(|v| v.as_str()).unwrap_or("unknown").to_string(),
            url: r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        });
    }
    Ok(output)
}

/// Convert PascalCase to kebab-case (e.g. "NeuralLab" → "neural-lab").
fn pascal_to_kebab(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            result.push('-');
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
}

/// Compare MCP tool definitions in `hex-cli/assets/mcp/mcp-tools.json` against
/// the `Commands` enum in `main.rs`. Returns tool names whose expected CLI
/// subcommand has no matching enum variant.
pub fn check_mcp_cli_parity() -> anyhow::Result<Vec<String>> {
    use std::collections::HashSet;

    let cli_src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let main_rs_path = format!("{}/main.rs", cli_src);
    let mcp_tools_path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/mcp/mcp-tools.json");

    // 1. Parse mcp-tools.json → extract (tool_name, top-level subcommand)
    let mcp_json = std::fs::read_to_string(mcp_tools_path)?;
    let mcp: serde_json::Value = serde_json::from_str(&mcp_json)?;

    let tools = mcp
        .get("tools")
        .and_then(|t| t.as_array())
        .ok_or_else(|| anyhow::anyhow!("mcp-tools.json missing 'tools' array"))?;

    let mut mcp_tools: Vec<(String, String)> = Vec::new();
    for tool in tools {
        let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let cli = tool.get("cli").and_then(|c| c.as_str()).unwrap_or("");
        let parts: Vec<&str> = cli.split_whitespace().collect();
        if parts.len() >= 2 {
            mcp_tools.push((name.to_string(), parts[1].to_string()));
        }
    }

    // 2. Parse Commands enum from main.rs to discover all CLI subcommands.
    let main_rs = std::fs::read_to_string(&main_rs_path)?;
    let mut cli_subcommands = HashSet::new();

    let mut in_commands_enum = false;
    for line in main_rs.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("enum Commands") {
            in_commands_enum = true;
            continue;
        }
        if !in_commands_enum {
            continue;
        }
        if trimmed == "}" {
            break;
        }

        // Capture explicit #[command(name = "...")] or #[command(alias = "...")]
        if let Some(rest) = trimmed.strip_prefix("#[command(") {
            for attr in ["name = \"", "alias = \""] {
                if let Some(start) = rest.find(attr) {
                    let after = &rest[start + attr.len()..];
                    if let Some(end) = after.find('"') {
                        cli_subcommands.insert(after[..end].to_string());
                    }
                }
            }
            continue;
        }

        // Skip comments, other attributes, empty lines
        if trimmed.starts_with("//") || trimmed.starts_with("#[") || trimmed.is_empty() {
            continue;
        }

        // Extract variant name and convert PascalCase → kebab-case
        let variant = trimmed
            .split(|c: char| c == '{' || c == '(' || c == ',' || c == ' ')
            .next()
            .unwrap_or("")
            .trim();
        if !variant.is_empty() && variant.chars().next().map_or(false, |c| c.is_uppercase()) {
            cli_subcommands.insert(pascal_to_kebab(variant));
        }
    }

    // 3. Find MCP tools whose subcommand is absent from the Commands enum
    let mut orphans: Vec<String> = Vec::new();
    let mut seen_subcmds = HashSet::new();
    for (tool_name, subcmd) in &mcp_tools {
        if !cli_subcommands.contains(subcmd.as_str()) && seen_subcmds.insert(subcmd.clone()) {
            orphans.push(format!("{} (expects `hex {}`)", tool_name, subcmd));
        }
    }
    orphans.sort();
    Ok(orphans)
}

async fn models() -> anyhow::Result<()> {
    let models = vec![
        ("nemotron-mini", "Fast local inference", "0.3"),
        ("qwen3:4b", "Small local model", "0.25"),
        ("qwen3:8b", "Medium local model", "0.35"),
        ("qwen3.5:9b", "Large local model", "0.40"),
        ("qwen2.5-coder:32b", "Coding dedicated", "0.50"),
        ("sonnet", "Cloud fallback", "0.50"),
    ];
    
    println!("{}", "Available Models".green().bold());
    println!("{:<20}  {:<25}  Base Score", "Model", "Description");
    println!("{}", "-".repeat(60));
    
    for (model, desc, score) in models {
        println!("{:<20}  {:<25}  {}", model, desc, score);
    }
    
    Ok(())
}fn pid_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".hex").join("sched-daemon.pid")
}

/// Legacy path from before the brain→sched rename. Read-only fallback so an
/// in-flight daemon started by the old binary still gets picked up by
/// `daemon-status` / `daemon-stop` in the new binary; new writes always go to
/// the new path. Safe to delete the legacy file once no operator runs the
/// pre-rename binary.
fn legacy_pid_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".hex").join("brain-daemon.pid")
}

fn write_pid_file(pid: u32) -> anyhow::Result<()> {
    let path = pid_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, pid.to_string())?;
    Ok(())
}

fn read_pid_file() -> Option<i32> {
    let path = pid_file_path();
    let contents = std::fs::read_to_string(&path)
        .or_else(|_| std::fs::read_to_string(legacy_pid_file_path()))
        .ok()?;
    contents.trim().parse::<i32>().ok()
}

fn remove_pid_file() {
    let _ = std::fs::remove_file(pid_file_path());
    let _ = std::fs::remove_file(legacy_pid_file_path());
}

fn process_alive(pid: i32) -> bool {
    // Signal 0 probes existence without delivering a signal.
    // Returns 0 on success (process exists), -1 on error.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

// ─── Daemon staleness detection (ADR-2026-04-24-1820) ────────────────────────────

/// Path to the daemon staleness record.
fn staleness_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".hex").join("daemon-staleness.json")
}

/// A record of the daemon's binary identity at startup. Written by a running
/// daemon; read by the CLI to detect when the binary has been rebuilt.
#[derive(Debug, Default, Serialize, Deserialize)]
struct DaemonStaleness {
    /// Path to the running binary.
    binary: PathBuf,
    /// mtime of the binary when the daemon started.
    startup_mtime: String,
    /// PID of the running daemon.
    startup_pid: u32,
    /// Human-readable binary version or git commit (if available).
    version: String,
    /// Tick interval in seconds (so restart can re-use the same interval).
    #[serde(default)]
    interval_secs: u64,
    /// Max failures threshold.
    #[serde(default)]
    max_failures: u32,
}

impl DaemonStaleness {
    fn current() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let mtime = std::fs::metadata(&exe).ok()?.modified().ok()?;
        let mtime_str = chrono::DateTime::<chrono::Utc>::from(mtime)
            .to_rfc3339();
        let version = git_head_sha().unwrap_or_else(|| "unknown".to_string());
        Some(Self {
            binary: exe,
            startup_mtime: mtime_str,
            startup_pid: std::process::id(),
            version,
            interval_secs: 0,
            max_failures: 0,
        })
    }
}

/// Load the staleness record. Returns None if the file is absent, unreadable,
/// or the PID does not match the current process (stale record from dead daemon).
fn load_staleness_record() -> Option<DaemonStaleness> {
    let path = staleness_file_path();
    let contents = std::fs::read_to_string(&path).ok()?;
    let record: DaemonStaleness = serde_json::from_str(&contents).ok()?;
    // If the record's PID is not alive, the record is stale — ignore it.
    if !process_alive(record.startup_pid as i32) {
        return None;
    }
    Some(record)
}

/// Save the staleness record. Writes atomically (temp + rename) to prevent
/// corruption if the daemon crashes mid-write.
fn save_staleness_record(record: &DaemonStaleness) {
    let path = staleness_file_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("  {} staleness-dir: {}", "warn".yellow(), e);
            return;
        }
    }
    let tmp = path.with_extension("tmp");
    match serde_json::to_string_pretty(record) {
        Ok(body) => {
            if let Err(e) = std::fs::write(&tmp, &body) {
                eprintln!("  {} staleness-write: {}", "warn".yellow(), e);
                return;
            }
            if let Err(e) = std::fs::rename(&tmp, &path) {
                eprintln!("  {} staleness-rename: {}", "warn".yellow(), e);
                return;
            }
        }
        Err(e) => eprintln!("  {} staleness-encode: {}", "warn".yellow(), e),
    }
}

/// Returns true if the current binary's mtime differs from the recorded startup
/// mtime — i.e. the binary was rebuilt while this daemon was running.
/// Respects DAEMON_STALENESS_CHECK=off for CI environments.
fn is_current_binary_stale() -> Option<bool> {
    if std::env::var("DAEMON_STALENESS_CHECK").unwrap_or_default() == "off" {
        return Some(false);
    }
    let current = DaemonStaleness::current()?;
    let record = load_staleness_record()?;
    if record.startup_pid != current.startup_pid {
        return Some(false);
    }
    Some(record.startup_mtime != current.startup_mtime)
}

/// Check staleness and warn if the current binary was rebuilt while the daemon
/// was running. Logs to stderr. Returns true if stale (daemon should skip tick).
fn check_and_warn_staleness() -> bool {
    match is_current_binary_stale() {
        Some(true) => {
            eprintln!(
                "{} daemon binary was rebuilt while running (pid={}). Restart with: hex sched daemon restart",
                "⚠ STALE".yellow().bold(),
                std::process::id()
            );
            true
        }
        Some(false) => false,
        None => false,
    }
}

/// Collect issue counts for each validate check without printing.
/// Pure data view of the same checks `validate()` runs — used by the daemon
/// to diff tick-over-tick and detect regressions (wp-brain-updates P2.1).
///
/// Keys:
/// - `cli_wiring`       — unwired command modules
/// - `binary_stale`     — 1 if release binary is stale, else 0
/// - `workplans_stale`  — total stale workplan tasks across all workplans
/// - `mcp_parity`       — MCP tools without a matching CLI subcommand
/// - `worktrees_stale`  — stale git worktrees
fn collect_issue_counts() -> HashMap<String, usize> {
    let mut counts = HashMap::new();

    counts.insert(
        "cli_wiring".to_string(),
        check_cli_wiring().map(|u| u.len()).unwrap_or(0),
    );

    counts.insert(
        "binary_stale".to_string(),
        match check_binary_freshness() {
            FreshnessStatus::Stale { .. } | FreshnessStatus::Missing => 1,
            _ => 0,
        },
    );

    counts.insert(
        "workplans_stale".to_string(),
        check_workplan_status()
            .map(|ws| ws.iter().map(|w| w.stale_tasks.len()).sum())
            .unwrap_or(0),
    );

    counts.insert(
        "mcp_parity".to_string(),
        check_mcp_cli_parity().map(|o| o.len()).unwrap_or(0),
    );

    counts.insert(
        "worktrees_stale".to_string(),
        check_stale_worktrees().map(|s| s.len()).unwrap_or(0),
    );

    counts
}

/// Per-kind verbosity controls for operator notifications (wp-brain-updates P3.2).
/// Loaded from `.hex/daemon.toml` under `[notify]`. Every flag defaults to `true`
/// so out-of-the-box behavior matches the pre-P3.2 "send everything" contract —
/// operators opt INTO quieter daemons by flipping flags off. `min_priority`
/// applies after the per-kind toggle: it's an overall noise floor (1 = send
/// everything, 2 = only send urgent items like failures / regressions).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BrainNotifyConfig {
    #[serde(default = "BrainNotifyConfig::default_true")]
    on_task_complete: bool,
    #[serde(default = "BrainNotifyConfig::default_true")]
    on_task_failure: bool,
    #[serde(default = "BrainNotifyConfig::default_true")]
    on_validate_regression: bool,
    #[serde(default = "BrainNotifyConfig::default_true")]
    on_workplan_complete: bool,
    #[serde(default = "BrainNotifyConfig::default_true")]
    on_grade_drop: bool,
    #[serde(default = "BrainNotifyConfig::default_min_priority")]
    min_priority: u8,
}

impl Default for BrainNotifyConfig {
    fn default() -> Self {
        Self {
            on_task_complete: true,
            on_task_failure: true,
            on_validate_regression: true,
            on_workplan_complete: true,
            on_grade_drop: true,
            min_priority: 1,
        }
    }
}

impl BrainNotifyConfig {
    fn default_true() -> bool {
        true
    }

    fn default_min_priority() -> u8 {
        1
    }

    /// Whether a notification of (`kind`, `priority`) should be delivered.
    /// Unknown kinds are always allowed — new notification types must not be
    /// silently swallowed just because they're missing from the schema.
    fn should_notify(&self, kind: &str, priority: u8) -> bool {
        if priority < self.min_priority {
            return false;
        }
        match kind {
            k if k.starts_with("brain.task.") && k.ends_with(".completed") => self.on_task_complete,
            "brain.task.completed" => self.on_task_complete,
            k if k.starts_with("brain.task.") && k.ends_with(".failed") => self.on_task_failure,
            "brain.task.failed" => self.on_task_failure,
            "brain.validate.regression" => self.on_validate_regression,
            "brain.workplan.complete" => self.on_workplan_complete,
            "brain.grade.drop" => self.on_grade_drop,
            _ => true,
        }
    }
}

/// TOML shape for `.hex/daemon.toml`. Only the `[notify]` section is consumed
/// today — other sections are ignored so adding future daemon knobs won't
/// break config parsing for existing deployments.
#[derive(Debug, Default, Deserialize)]
struct DaemonTomlFile {
    #[serde(default)]
    notify: Option<BrainNotifyConfig>,
}

/// Load `.hex/daemon.toml` from cwd. Missing / unreadable / malformed file →
/// default config (everything enabled). Must never panic or fail the caller —
/// config errors should not silence notifications.
fn load_notify_config() -> BrainNotifyConfig {
    let Ok(cwd) = std::env::current_dir() else {
        return BrainNotifyConfig::default();
    };
    let path = cwd.join(".hex").join("daemon.toml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return BrainNotifyConfig::default();
    };
    match toml::from_str::<DaemonTomlFile>(&content) {
        Ok(parsed) => parsed.notify.unwrap_or_default(),
        Err(_) => BrainNotifyConfig::default(),
    }
}

/// Central fire-and-forget operator notification helper (wp-brain-updates P1.1).
/// POSTs to `/api/hexflo/inbox/notify` with the project_id auto-resolved from
/// `.hex/project.json` in cwd. All daemon-side notifications flow through here
/// so routing, scoping, and error handling stay in one place. Errors are
/// swallowed — must never fail the caller.
///
/// P3.2: Applies `.hex/daemon.toml` verbosity filters before sending. A
/// suppressed notification is a no-op — no HTTP call, no log spam.
async fn notify_operator(kind: &str, body: serde_json::Value, priority: u8) {
    let Some(project_id) = brain_project_id() else { return };
    if !load_notify_config().should_notify(kind, priority) {
        return;
    }
    let envelope = json!({
        "project_id": project_id,
        "priority": priority,
        "kind": kind,
        "payload": body.to_string(),
    });
    let nexus = crate::nexus_client::NexusClient::from_env();
    let _ = nexus.post("/api/hexflo/inbox/notify", &envelope).await;
}

/// Fire-and-forget operator notification when validate counts regress
/// tick-over-tick (wp-brain-updates P2.1). priority=2 — operator intervention
/// may be needed.
async fn notify_validate_regression(
    regressions: &[(String, usize, usize)],
    current: &HashMap<String, usize>,
) {
    let regression_lines: Vec<String> = regressions
        .iter()
        .map(|(k, prev, curr)| format!("{}: {} → {}", k, prev, curr))
        .collect();
    let body = json!({
        "regressions": regressions
            .iter()
            .map(|(k, prev, curr)| json!({"check": k, "previous": prev, "current": curr}))
            .collect::<Vec<_>>(),
        "summary": regression_lines.join(", "),
        "counts": current,
    });
    notify_operator("brain.validate.regression", body, 2).await;
}

/// POST to `/api/events` recording a structured sched-daemon event.
/// Mirrors the inline `brain_tick` POST in [`daemon`] but exposes a `payload`
/// field so per-handler counts can travel alongside the kind. Network errors
/// and HTTP non-success responses are logged to stderr (visible in the daemon
/// log) per ADR-2026-04-24-1815 P2 — the original silent-drop bug it diagnoses
/// returned a 400 that fire-and-forget code never noticed.
async fn record_sched_event(event_type: &str, payload: serde_json::Value) {
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let event_url = format!("http://127.0.0.1:{}/api/events", port);
    let session_id = std::env::var("CLAUDE_SESSION_ID")
        .unwrap_or_else(|_| format!("sched-daemon-{}", std::process::id()));
    let body = serde_json::json!({
        "session_id": session_id,
        "event_type": event_type,
        "payload": payload,
    });
    match reqwest::Client::new()
        .post(&event_url)
        .json(&body)
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        Ok(resp) if !resp.status().is_success() => {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            eprintln!(
                "  {} {} event POST rejected ({}): {}",
                "✗".red(),
                event_type,
                status,
                body_text.trim()
            );
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!(
                "  {} {} event POST failed: {}",
                "✗".red(),
                event_type,
                e
            );
        }
    }
}

/// Sched event emitted by `tick_adr_health_actions`. Holds the same payload
/// shape that the live tick handler POSTs to `/api/events`; surfaced as data
/// so tests can assert on it without standing up a fake HTTP server.
#[derive(Debug, Clone)]
pub struct TickEvent {
    pub event_type: &'static str,
    pub payload: serde_json::Value,
}

/// Inbox notification queued by `tick_adr_health_actions`. Mirrors the
/// arguments [`notify_operator`] would have been called with — `kind` is the
/// dotted notification key (e.g. `adr.doctor.applied`), `body` is the JSON
/// payload, `priority` is the inbox priority (1=interrupt, 2=normal,
/// 3=informational).
#[derive(Debug, Clone)]
pub struct TickNotification {
    pub kind: &'static str,
    pub body: serde_json::Value,
    pub priority: u8,
}

/// Outcome of one `tick_adr_health` orchestration pass. Returned by
/// [`tick_adr_health_actions`] so callers (the live wrapper, P3.2 tests) can
/// inspect what *would* be dispatched without coupling to HTTP I/O.
#[derive(Debug, Clone)]
pub struct TickAdrHealthResult {
    pub event: TickEvent,
    pub notifications: Vec<TickNotification>,
}

/// Hermetic core of [`tick_adr_health`]: takes pre-collected findings + an
/// optional shadow-promote config, runs the Tier-A/B/C dispatch in order,
/// and returns the (event, notifications) pair the live wrapper would
/// otherwise have POSTed to nexus.
///
/// File-system side effects of Tier-A shadow-promotion and Tier-B drafter
/// commits *do* happen inside this function — that's the auto-fix itself —
/// but no HTTP is performed. Callers who want HTTP dispatch wrap the result
/// (see [`tick_adr_health`]). Tests assert against the returned result and
/// the resulting filesystem state directly.
pub async fn tick_adr_health_actions(
    findings: &[doctor::Finding],
    cfg: Option<&doctor::ShadowPromoteConfig>,
) -> TickAdrHealthResult {
    let total = findings.len();
    let tier_a = findings.iter().filter(|f| f.tier == doctor::AutoFixTier::A).count();
    let tier_b = findings.iter().filter(|f| f.tier == doctor::AutoFixTier::B).count();
    let tier_c = findings.iter().filter(|f| f.tier == doctor::AutoFixTier::C).count();
    let errors = findings.iter().filter(|f| f.severity == doctor::Severity::Error).count();
    let warnings = findings.iter().filter(|f| f.severity == doctor::Severity::Warning).count();

    let event = TickEvent {
        event_type: "adr_doctor_tick",
        payload: json!({
            "total": total,
            "tier_a": tier_a,
            "tier_b": tier_b,
            "tier_c": tier_c,
            "errors": errors,
            "warnings": warnings,
        }),
    };

    let mut notifications = Vec::new();
    for finding in findings {
        match finding.tier {
            doctor::AutoFixTier::A => {
                let Some(cfg) = cfg else {
                    notifications.push(TickNotification {
                        kind: "adr.doctor.aborted",
                        body: json!({
                            "adr_id": finding.adr_id,
                            "kind": format!("{:?}", finding.kind),
                            "tier": "A",
                            "reason": "shadow_promote config unavailable",
                            "detail": finding.detail,
                        }),
                        priority: 2,
                    });
                    continue;
                };
                let outcome = doctor::shadow_promote_with_policy(
                    finding,
                    cfg,
                    doctor::MergePolicy::Merge,
                )
                .unwrap_or_else(|e| doctor::Outcome::Aborted {
                    reason: format!("shadow_promote raised: {}", e),
                });
                match outcome {
                    doctor::Outcome::Applied { branch, commit } => {
                        notifications.push(TickNotification {
                            kind: "adr.doctor.applied",
                            body: json!({
                                "adr_id": finding.adr_id,
                                "kind": format!("{:?}", finding.kind),
                                "tier": "A",
                                "branch": branch,
                                "commit": commit,
                                "detail": finding.detail,
                            }),
                            priority: 3,
                        });
                    }
                    doctor::Outcome::Aborted { reason } => {
                        notifications.push(TickNotification {
                            kind: "adr.doctor.aborted",
                            body: json!({
                                "adr_id": finding.adr_id,
                                "kind": format!("{:?}", finding.kind),
                                "tier": "A",
                                "reason": reason,
                                "detail": finding.detail,
                            }),
                            priority: 2,
                        });
                    }
                }
            }
            doctor::AutoFixTier::B => {
                let Some(cfg) = cfg else {
                    notifications.push(TickNotification {
                        kind: "adr.doctor.aborted",
                        body: json!({
                            "adr_id": finding.adr_id,
                            "kind": format!("{:?}", finding.kind),
                            "tier": "B",
                            "reason": "tier_b_draft config unavailable",
                            "detail": finding.detail,
                        }),
                        priority: 2,
                    });
                    continue;
                };
                let outcome = doctor::tier_b_draft_with_config(finding, cfg).unwrap_or_else(
                    |e| doctor::Outcome::Aborted {
                        reason: format!("tier_b_draft raised: {}", e),
                    },
                );
                match outcome {
                    doctor::Outcome::Applied { branch, commit } => {
                        notifications.push(TickNotification {
                            kind: "adr.doctor.draft",
                            body: json!({
                                "adr_id": finding.adr_id,
                                "kind": format!("{:?}", finding.kind),
                                "tier": "B",
                                "branch": branch,
                                "commit": commit,
                                "detail": finding.detail,
                            }),
                            priority: 2,
                        });
                    }
                    doctor::Outcome::Aborted { reason } => {
                        notifications.push(TickNotification {
                            kind: "adr.doctor.aborted",
                            body: json!({
                                "adr_id": finding.adr_id,
                                "kind": format!("{:?}", finding.kind),
                                "tier": "B",
                                "reason": reason,
                                "detail": finding.detail,
                            }),
                            priority: 2,
                        });
                    }
                }
            }
            doctor::AutoFixTier::C => {
                notifications.push(TickNotification {
                    kind: "adr.doctor.notify",
                    body: json!({
                        "adr_id": finding.adr_id,
                        "kind": format!("{:?}", finding.kind),
                        "tier": "C",
                        "severity": format!("{:?}", finding.severity),
                        "detail": finding.detail,
                    }),
                    priority: 1,
                });
            }
        }
    }

    TickAdrHealthResult { event, notifications }
}

/// Run `hex adr doctor` once and route findings per ADR-2026-04-27-0800 §1a.
///
///   - Tier-A → [`doctor::shadow_promote`] (with [`doctor::MergePolicy::Merge`]).
///     On `Outcome::Applied`, fire a P3 inbox entry summarizing the fix
///     (informational; no operator interrupt). On `Outcome::Aborted`, downgrade
///     to a P2 notification so the operator can decide whether to retry.
///   - Tier-B → [`doctor::tier_b_draft_with_config`]. The drafter writes a
///     placeholder notes file on a `sched/auto-fix/ADR-doctor/tier-b/...`
///     branch (worktree left in place); we file a P2 inbox entry with the
///     branch + commit so the operator can review the diff and decide.
///   - Tier-C → P1 inbox notification, no mutation. Tier-C kinds are judgment
///     calls (`DuplicateId`, `MissingRequiredField`, `DanglingDependency`) that
///     the doctor can detect but never resolve mechanically.
///
/// Records a `sched_event` of kind `adr_doctor_tick` with finding counts
/// keyed by tier and severity so the daemon's event log captures every scan
/// — including clean ones — for trend analysis. The `_state` parameter is
/// reserved for future tick-state coordination (e.g. cross-tick rate-limiting
/// of the doctor scan); the doctor itself is stateless across ticks.
///
/// Thin wrapper around [`tick_adr_health_actions`]: that's where the
/// orchestration lives, here we add scan + HTTP dispatch. Tests drive the
/// hermetic path directly.
pub async fn tick_adr_health(_state: &SchedState) {
    let findings = match doctor::run().await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("  {} adr doctor scan failed: {}", "✗".red(), e);
            record_sched_event(
                "adr_doctor_tick",
                json!({ "error": e.to_string() }),
            )
            .await;
            return;
        }
    };

    // Live config for shadow-promote / tier-b drafter. If the daemon isn't
    // running inside a git repo (rare — examples/test envs) we log + bail
    // rather than crashing the tick. The Tier-C P1 notifications still go
    // through, so operators don't lose visibility of judgment-call findings.
    let cfg = match doctor::ShadowPromoteConfig::live() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!(
                "  {} adr doctor: shadow-promote config unavailable: {} (Tier-C notify only)",
                "⚠".yellow(),
                e
            );
            None
        }
    };

    let total = findings.len();
    let tier_a = findings.iter().filter(|f| f.tier == doctor::AutoFixTier::A).count();
    let tier_b = findings.iter().filter(|f| f.tier == doctor::AutoFixTier::B).count();
    let tier_c = findings.iter().filter(|f| f.tier == doctor::AutoFixTier::C).count();

    let result = tick_adr_health_actions(&findings, cfg.as_ref()).await;

    record_sched_event(result.event.event_type, result.event.payload).await;

    if total == 0 {
        return;
    }

    println!(
        "  {} adr doctor: {} finding(s) (A:{} B:{} C:{})",
        "⬡".cyan(),
        total,
        tier_a,
        tier_b,
        tier_c
    );

    for n in result.notifications {
        notify_operator(n.kind, n.body, n.priority).await;
    }
}

/// Foreground supervisor loop. Validates every `interval` seconds; after
/// `max_failures` consecutive failures, pauses for 5x interval before retrying.
/// Exits cleanly on ctrl-C.
/// Read /etc/hostname for the heartbeat worker_id suffix. Falls back to
/// "unknown" on any failure — the worker_id stays unique because PID is
/// also part of it.
fn hostname_local() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

async fn daemon(interval: u64, max_failures: u32) -> anyhow::Result<()> {
    // Write the PID so DaemonStop can find a foreground instance too.
    let pid = std::process::id();
    let _ = write_pid_file(pid);

    // ADR-2026-04-24-1820: record binary identity at startup for staleness detection.
    // Stores interval/max_failures so `daemon_restart` can re-use the same settings.
    let mut current = DaemonStaleness::current();
    if let Some(ref mut rec) = current {
        rec.interval_secs = interval;
        rec.max_failures = max_failures;
        // Check if binary was rebuilt since last daemon run.
        if let Some(prev) = load_staleness_record() {
            if prev.startup_mtime != rec.startup_mtime {
                println!(
                    "  {} binary was rebuilt since last daemon run ({}, {})",
                    "fresh binary".cyan(),
                    prev.startup_mtime.split('T').next().unwrap_or(&prev.startup_mtime),
                    rec.startup_mtime.split('T').next().unwrap_or(&rec.startup_mtime),
                );
            }
        }
        save_staleness_record(rec);
    }

    println!(
        "{} interval={}s max_failures={} pid={}",
        "⬡ sched daemon starting".green().bold(),
        interval,
        max_failures,
        pid
    );

    // Heartbeat client (ADR-2605190900 P1.4) — sched daemon publishes
    // its liveness to worker_process every 15s. supervisor_tick reaps
    // any row with last_heartbeat > 60s, so a hung daemon is visible
    // in /api/liveness within one supervisor cycle (today: 10s) instead
    // of going silent for 11 days like the May 8 zombie did. Detached
    // task — on STDB outage we miss beats but the daemon keeps draining.
    {
        let role = "sched-daemon";
        let worker_id = format!("{}-{}-{}", role, hostname_local(), pid);
        tokio::spawn(async move {
            let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
                .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
            let database = std::env::var("HEX_STDB_DATABASE")
                .unwrap_or_else(|_| "hex".to_string());
            let http = match reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
            {
                Ok(c) => c,
                Err(_) => return,
            };
            // First-time register via worker_process_register. Best-effort.
            let _ = http
                .post(format!("{stdb_host}/v1/database/{database}/call/worker_process_register"))
                .json(&serde_json::json!([
                    worker_id,
                    "sched-daemon-default",
                    role,
                    hostname_local(),
                    pid as i64,
                ]))
                .send()
                .await;
            let mut ticker = tokio::time::interval(Duration::from_secs(15));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker.tick().await; // consume immediate first tick — register handled it
            loop {
                ticker.tick().await;
                let _ = http
                    .post(format!("{stdb_host}/v1/database/{database}/call/worker_process_status"))
                    .json(&serde_json::json!([
                        worker_id,
                        "healthy",
                        "",
                    ]))
                    .send()
                    .await;
            }
        });
    }

    let mut consecutive_failures: u32 = 0;
    let mut paused_cycles: u32 = 0;
    let mut state = load_daemon_state();

    loop {
        // ADR-2026-04-24-1820: skip tick if binary was rebuilt while daemon was running.
        if check_and_warn_staleness() {
            // Log skipped tick and wait for operator to restart.
            let port = std::env::var("HEX_NEXUS_PORT")
                .unwrap_or_else(|_| "5555".to_string())
                .parse::<u16>()
                .unwrap_or(5555);
            eprintln!("  {} skipped tick (binary stale) — restart daemon to resume", "⏱".yellow());
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(interval)) => continue,
                _ = tokio::signal::ctrl_c() => {
                    println!("\n{}", "⬡ sched daemon shutting down".yellow());
                    remove_pid_file();
                    return Ok(());
                }
            }
        }

        let timestamp = chrono::Utc::now().to_rfc3339();
        let start = Instant::now();
        let pending_count = list_brain_tasks(Some("pending")).await.map(|t| t.len()).unwrap_or(0);
        let in_progress_count = list_brain_tasks(Some("in_progress")).await.map(|t| t.len()).unwrap_or(0);
        let failed_count = list_brain_tasks(Some("failed")).await.map(|t| t.len()).unwrap_or(0);
        let validate_result = validate(true).await;
        let elapsed = start.elapsed();
        let queue_summary = if pending_count + in_progress_count == 0 && failed_count == 0 {
            "queue=0".to_string()
        } else {
            let mut parts = Vec::new();
            if pending_count > 0 { parts.push(format!("{}p", pending_count)); }
            if in_progress_count > 0 { parts.push(format!("{}w", in_progress_count)); }
            if failed_count > 0 { parts.push(format!("{}f", failed_count)); }
            format!("queue={}", parts.join("+"))
        };
        println!("{} {} ✓ {}ms  {}", "⬡".cyan(), timestamp, elapsed.as_millis(), queue_summary.bold());

        // Diff issue counts tick-over-tick (wp-brain-updates P2.1).
        // First tick seeds the baseline; no notification until we have a prior.
        let current_counts = collect_issue_counts();
        // Track has_regressions for RL reward before the seeded guard.
        let mut has_regressions = false;
        if state.seeded {
            let mut regressions: Vec<(String, usize, usize)> = Vec::new();
            let mut improvements: Vec<(String, usize, usize)> = Vec::new();
            for (key, &curr) in &current_counts {
                let prev = state.last_counts.get(key).copied().unwrap_or(0);
                if curr > prev {
                    regressions.push((key.clone(), prev, curr));
                } else if curr < prev {
                    improvements.push((key.clone(), prev, curr));
                }
            }
            has_regressions = !regressions.is_empty();
            if !regressions.is_empty() {
                regressions.sort_by(|a, b| a.0.cmp(&b.0));
                let summary: Vec<String> = regressions
                    .iter()
                    .map(|(k, p, c)| format!("{} {}→{}", k, p, c))
                    .collect();
                eprintln!(
                    "  {} validate regressed: {}",
                    "⚠".red().bold(),
                    summary.join(", ")
                );
                notify_validate_regression(&regressions, &current_counts).await;
            }
            if !improvements.is_empty() {
                improvements.sort_by(|a, b| a.0.cmp(&b.0));
                let summary: Vec<String> = improvements
                    .iter()
                    .map(|(k, p, c)| format!("{} {}→{}", k, p, c))
                    .collect();
                println!(
                    "  {} validate improved: {}",
                    "✓".green(),
                    summary.join(", ")
                );
            }
        }
        state.last_counts = current_counts;
        state.seeded = true;
        save_daemon_state(&state);

        // ── Validate result ────────────────────────────────────────────────
        let validate_ok = validate_result.is_ok();
        match validate_result {
            Ok(()) => {
                if consecutive_failures > 0 {
                    println!("  {} after {} failure(s)", "recovered".green(), consecutive_failures);
                }
                consecutive_failures = 0;
                paused_cycles = 0;
            }
            Err(err) => {
                consecutive_failures += 1;
                eprintln!("  {} ({}/{}) validate: {}", "fail".red(), consecutive_failures, max_failures, err);
            }
        }

        // Drain brain queue — hand up to 1 pending task per tick to a
        // `brain-lease` swarm (ADR-2026-04-14-1400 P1.2). The daemon no longer
        // executes work inline; it stamps the lease and moves on. Swarm
        // workers progress the task; the sweeper reclaims if the lease
        // expires. Runs regardless of validate() outcome.
        //
        // ADR-2026-04-14-1400 §1 partial-impl gap (dog-food finding 2026-04-14):
        // no swarm workers register against `brain-lease`, and no reclaim
        // sweeper exists, so a pure swarm-lease path silently parks every
        // task in `leased` forever — bypassing the §1 P1 evidence guard
        // that lives in execute_brain_task. Until §2 ships fully, fall
        // back to inline execution whenever dispatch reports no live
        // worker. The guard runs on the fallback path.
        // Drain up to 5 tasks per tick (was 1, caused 12min delay with 24 tasks)
        let drain_result = drain_brain_tasks(5).await;
        match drain_result {
            Ok(tasks) if !tasks.is_empty() => {
                for task in tasks {
                    let id = task
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let kind = task
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let payload = task
                        .get("payload")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    println!("  ⬡ leasing sched task {id} ({kind})");
                    let outcome = match dispatch_brain_task(&task).await {
                        Ok(handle) => classify_dispatch(&handle).await,
                        Err(err) => DispatchOutcome::Error(err.to_string()),
                    };
                    if should_fallback_inline(&outcome) {
                        // Inline fallback: guard-active execute_brain_task.
                        // Mirrors queue_drain()'s inline pattern 1:1.
                        match &outcome {
                            DispatchOutcome::Error(err) => {
                                eprintln!(
                                    "    {} dispatch failed: {} — inline exec (guard active)",
                                    "✗".red(),
                                    err
                                );
                            }
                            _ => {
                                println!(
                                    "    {} leased swarm empty — inline exec (guard active)",
                                    "→".cyan(),
                                );
                            }
                        }
                        let _ =
                            update_brain_task(&id, BrainTaskStatus::InProgress, "").await;
                        let (mut ok, mut result) = execute_brain_task(&kind, &payload).await;
                        // ADR-2026-04-14-2155 P2.3: reject vacuous executor output
                        if ok {
                            if let Err(reason) = validate_dispatch_evidence(Some(&result)) {
                                ok = false;
                                result.push_str(&format!("\n--- dispatch-evidence guard ---\n{reason}"));
                            }
                        }
                        let status = if ok {
                            BrainTaskStatus::Completed
                        } else {
                            BrainTaskStatus::Failed
                        };
                        if let Err(err) = update_brain_task(&id, status, &result).await {
                            eprintln!(
                                "    {} update_brain_task failed: {}",
                                "✗".red(),
                                err
                            );
                        }
                        println!(
                            "    {} {}",
                            if ok { "✓".green() } else { "✗".red() },
                            status.as_str()
                        );
                        notify_brain_task_result(
                            &id,
                            &kind,
                            &payload,
                            status.as_str(),
                            &result,
                        )
                        .await;
                    } else if let DispatchOutcome::LeasedToWorker {
                        swarm_id,
                        swarm_task_id,
                        leased_until,
                        ..
                    } = &outcome
                    {
                        println!(
                            "    {} leased to swarm {} (task {}, until {})",
                            "→".cyan(),
                            swarm_id,
                            swarm_task_id,
                            leased_until,
                        );
                    }
                }
            }
            _ => {}
        }

        // ── Periodic dead-code analysis (ADR-2026-04-24-1800) ─────────────────
        if should_enqueue_analyze(&state).await {
            match enqueue_analyze_task().await {
                Ok(id) => {
                    println!("  ⬡ enqueued periodic analyze task {}", id);
                    state.last_analyze_at = Some(timestamp.clone());
                }
                Err(e) => {
                    eprintln!("  {} enqueue analyze: {}", "✗".red(), e);
                }
            }
        }

        // ── Auto-retry failed tasks (wp-auto-retry-loop) ─────────────────
        // Re-enqueue failed tasks < 24h old with retry_count < 3
        if let Ok(failed_tasks) = list_brain_tasks(Some("failed")).await {
            for task in failed_tasks {
                let task_id = task.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let created_at = task.get("created_at").and_then(|v| v.as_str());
                let payload_retry_count = task
                    .get("payload")
                    .and_then(|v| v.as_str())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                    .and_then(|p| p.get("retry_count").and_then(|v| v.as_u64()));
                let retry_count = task
                    .get("retry_count")
                    .and_then(|v| v.as_u64())
                    .or(payload_retry_count)
                    .unwrap_or(0);

                if let Some(created_str) = created_at {
                    if let Ok(created) = chrono::DateTime::parse_from_rfc3339(created_str) {
                        let age = chrono::Utc::now().signed_duration_since(created.with_timezone(&chrono::Utc));
                        if age.num_hours() < 24 && retry_count < 3 {
                            let kind = task.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                            let payload = task.get("payload").and_then(|v| v.as_str()).unwrap_or("");
                            eprintln!("  ⬡ auto-retry {} (attempt {}/3, age {}h)", task_id, retry_count + 1, age.num_hours());
                            // Re-enqueue with incremented retry_count
                            let updated_payload = if let Ok(mut p) = serde_json::from_str::<serde_json::Value>(payload) {
                                p["retry_count"] = serde_json::json!(retry_count + 1);
                                p.to_string()
                            } else {
                                payload.to_string()
                            };
                            let _ = enqueue_brain_task(kind, &updated_payload).await;
                            // Mark original as completed to prevent re-retry
                            let _ = update_brain_task(task_id, BrainTaskStatus::Completed, "auto-retried").await;
                        } else if retry_count >= 3 {
                            // Retry budget exhausted — quarantine to DeadLetter
                            // so this task stops appearing in every subsequent
                            // failed-pool scan. The dead_letter detector will
                            // surface it as a hypothesis so the operator (or a
                            // future act mapping) can decide whether the task
                            // is structurally broken or worth a manual rerun.
                            eprintln!(
                                "  ⊘ dead-letter {} (retry budget exhausted, age {}h)",
                                task_id,
                                age.num_hours()
                            );
                            let _ = update_brain_task(
                                task_id,
                                BrainTaskStatus::DeadLetter,
                                "retry budget exhausted",
                            )
                            .await;
                        }
                    }
                }
            }
        }

        // ── Improver learn + auto-act pass (ADR-2026-04-27-1100 P5/P6) ───────
        // Two-step:
        //   1. If a prior `act --apply` snapshot is on disk, learn from it
        //      (credit resolved hypotheses, rotate snapshot).
        //   2. If no snapshot is on disk AND tick is divisible by
        //      AUTO_ACT_EVERY_N_TICKS, run a small auto-act sweep
        //      (top 1 only, to keep blast radius minimal). Operator can
        //      still drive larger sweeps via `hex sched improver act`.
        const AUTO_ACT_EVERY_N_TICKS: u32 = 6;
        if let Ok(home) = std::env::var("HOME") {
            let snap_path = std::path::PathBuf::from(&home).join(".hex/improver/snapshot.json");
            let repo = std::env::current_dir();
            let hyps = repo
                .as_ref()
                .ok()
                .and_then(|r| crate::commands::sched::improver::discover::discover(r).ok());

            // Visibility: emit an `improver_tick` event so `hex sched watch`
            // shows what the loop is finding, not just brain_tick heartbeats.
            // Without this, operators see only adr_doctor/brain_tick and
            // assume the improver is dead even when it's running.
            if let Some(ref hs) = hyps {
                let by_source: std::collections::HashMap<String, usize> = hs
                    .iter()
                    .fold(std::collections::HashMap::new(), |mut m, h| {
                        *m.entry(format!("{:?}", h.source)).or_insert(0) += 1;
                        m
                    });
                let port = std::env::var("HEX_NEXUS_PORT")
                    .unwrap_or_else(|_| "5555".to_string())
                    .parse::<u16>()
                    .unwrap_or(5555);
                let event_url = format!("http://127.0.0.1:{}/api/events", port);
                let session_id = std::env::var("CLAUDE_SESSION_ID")
                    .unwrap_or_else(|_| format!("sched-daemon-{}", std::process::id()));
                let body = serde_json::json!({
                    "session_id": session_id,
                    "event_type": "improver_tick",
                    "duration_ms": 0_i64,
                    "input_json": serde_json::to_string(&serde_json::json!({
                        "total": hs.len(),
                        "by_source": by_source,
                    })).unwrap_or_default(),
                });
                let _ = reqwest::Client::new()
                    .post(&event_url)
                    .json(&body)
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await;
            }

            // Step 1: learn (consumes snapshot if present)
            if snap_path.exists() {
                if let Some(ref hs) = hyps {
                    match crate::commands::sched::improver::learn::observe_and_reward(hs) {
                        Ok(0) => {}
                        Ok(n) => println!("  ⬡ improver learn: credited {n} action(s)"),
                        Err(e) => eprintln!("  {} improver learn: {}", "✗".red(), e),
                    }
                }
            }

            // Step 2: auto-act (only when no snapshot pending — don't
            // pile new actions on top of unobserved ones).
            //
            // Previously gated to every 6th tick (every 3min at 30s
            // intervals) — too slow for an interactive feedback loop on a
            // failing build. The score≥80 + top-1 throttle still bounds
            // blast radius to one high-confidence action per tick.
            // AUTO_ACT_EVERY_N_TICKS retained as a safety knob: if a tick
            // is ever flagged as "skip" (e.g. expensive sweep), the modulo
            // still fires every Nth tick.
            let snap_path_now_exists = snap_path.exists();
            let tick_modulo = state.tick_count % AUTO_ACT_EVERY_N_TICKS;
            let _ = tick_modulo;
            if !snap_path_now_exists {
                if let Some(ref hs) = hyps {
                    let ranked = crate::commands::sched::improver::judge::rank(hs);
                    if let Some(top) = ranked.first() {
                        if top.score >= 80 {
                            match crate::commands::sched::improver::act::act(&ranked, 1, true).await {
                                Ok(actions) => {
                                    if let Some(a) = actions.first() {
                                        println!(
                                            "  ⬡ improver auto-act: {} (priority {}, score {})",
                                            a.derived_from, a.priority, top.score
                                        );
                                        // Visible event so watchers see the action land.
                                        let port = std::env::var("HEX_NEXUS_PORT")
                                            .unwrap_or_else(|_| "5555".to_string())
                                            .parse::<u16>()
                                            .unwrap_or(5555);
                                        let event_url = format!("http://127.0.0.1:{}/api/events", port);
                                        let session_id = std::env::var("CLAUDE_SESSION_ID")
                                            .unwrap_or_else(|_| format!("sched-daemon-{}", std::process::id()));
                                        let body = serde_json::json!({
                                            "session_id": session_id,
                                            "event_type": "improver_act",
                                            "input_json": serde_json::to_string(&serde_json::json!({
                                                "derived_from": a.derived_from,
                                                "priority": a.priority,
                                                "score": top.score,
                                                "kind": format!("{:?}", a.kind),
                                            })).unwrap_or_default(),
                                        });
                                        let _ = reqwest::Client::new()
                                            .post(&event_url)
                                            .json(&body)
                                            .timeout(Duration::from_secs(2))
                                            .send()
                                            .await;
                                    }
                                }
                                Err(e) => eprintln!("  {} improver auto-act: {}", "✗".red(), e),
                            }
                        }
                    }
                }
            }

            // Step 3: autonomous high-severity Recommend notifications (BS-5).
            // Recommends don't enqueue work, so they skip the score≥80
            // auto-act gate. But severity=error patterns (e.g. an ADR cited
            // by ≥5 personas across recent thoughts) are signals the
            // operator should see in real time, not on the next manual
            // `hex sched improver act --apply`. notify_high_severity_recommends
            // dedups on a 24h window so persistent patterns surface once
            // per day rather than every tick.
            if let Some(ref hs) = hyps {
                let ranked = crate::commands::sched::improver::judge::rank(hs);
                crate::commands::sched::improver::act::notify_high_severity_recommends(&ranked).await;
            }
        }
        state.tick_count = state.tick_count.saturating_add(1);
        save_daemon_state(&state);

        // ── Improver history recorder ───────────────────────────────────
        // One snapshot per tick so the convergence series accumulates
        // passively even when no operator is invoking the CLI. Side-
        // effect: appends one line to ~/.hex/improver/history.jsonl.
        // No stdout output (we have the daemon tick summary above).
        if let Err(e) = crate::commands::sched::improver::record_history_snapshot().await {
            eprintln!("  ! improver history snapshot: {}", e);
        }

        // ── Analyze regression detection (ADR-2026-04-24-1800) ────────────────
        // After each tick, check if a completed analyze task has more violations
        // than last time. Notify operator on regression.
        if state.seeded {
            if let Some(summary) = check_analyze_regression(&state).await {
                let body = serde_json::json!({
                    "regressions": summary.regressions,
                    "current": summary.current,
                    "previous": summary.previous,
                });
                notify_operator("brain.analyze.regression", body, 2).await;
                for (key, prev, curr) in &summary.regressions {
                    eprintln!(
                        "  {} analyze regressed: {} {} → {}",
                        "⚠".red().bold(),
                        key,
                        prev,
                        curr
                    );
                }
                state.last_analysis_summary = summary.current.clone();
            }
        }

        // ── ADR registry health (ADR-2026-04-27-0800 §1a, wp-ADR-doctor-self-fix P3.2)
        // After workplan reconcile, before swarm cleanup. Cheap when the
        // registry is clean (a single `docs/adrs/` scan). Routes Tier-A
        // findings through shadow-promotion and emits P1 inbox entries for
        // Tier-C judgment calls — the rest of the loop is unaffected.
        tick_adr_health(&state).await;

        // Sweep stuck in_progress tasks (ADR-2026-04-14-2155 P2.2).
        match sweep_stuck_tasks().await {
            Ok(swept) if !swept.is_empty() => {
                for tid in &swept {
                    eprintln!(
                        "  {} swept stuck task {} → failed",
                        "⏱".red(),
                        tid,
                    );
                }
            }
            Err(err) => {
                eprintln!("  {} sweep error: {}", "✗".red(), err);
            }
            _ => {}
        }

        // Emit brain_tick event to nexus — ADR-2026-04-24-1815: explicit session_id
        // and schema-validated body. Errors are logged, not silently dropped.
        let port = std::env::var("HEX_NEXUS_PORT")
            .unwrap_or_else(|_| "5555".to_string())
            .parse::<u16>()
            .unwrap_or(5555);
        let event_url = format!("http://127.0.0.1:{}/api/events", port);
        let session_id = std::env::var("CLAUDE_SESSION_ID")
            .unwrap_or_else(|_| format!("sched-daemon-{}", std::process::id()));
        let body = serde_json::json!({
            "session_id": session_id,
            "event_type": "brain_tick",
            "duration_ms": elapsed.as_millis() as i64,
        });
        match reqwest::Client::new()
            .post(&event_url)
            .json(&body)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            Ok(resp) if !resp.status().is_success() => {
                let status = resp.status();
                let body_text = resp.text().await.unwrap_or_default();
                eprintln!(
                    "  {} brain_tick event POST rejected ({}): {}",
                    "✗".red(),
                    status,
                    body_text.trim()
                );
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("  {} brain_tick event POST failed: {}", "✗".red(), e);
            }
        }

        // ADR-2026-04-24-1820: Record validate outcome as RL reward.
        // +0.5 if validate passed with no regressions, -0.3 if regressions found.
        // This closes the RL loop: daemon behavior improves based on project health.
        record_daemon_reward(validate_ok, has_regressions).await;

        // Auto-sweep old terminal (failed/completed) tasks — keep history for 7 days.
        // Deletes records older than 7 days to prevent unbounded SpacetimeDB growth.
        // Runs every tick but only actually deletes on the first tick of each day.
        sweep_old_terminal_tasks(&mut state).await;

        // Sleep — longer if we're over the failure threshold.
        let sleep_secs = if consecutive_failures >= max_failures {
            paused_cycles = paused_cycles.saturating_add(1);
            eprintln!(
                "{} {}s (paused cycle {})",
                "  backing off for".yellow(),
                interval * 5,
                paused_cycles
            );
            interval * 5
        } else {
            interval
        };

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(sleep_secs)) => {}
            _ = tokio::signal::ctrl_c() => {
                println!("\n{}", "⬡ sched daemon received ctrl-C, shutting down".yellow());
                remove_pid_file();
                return Ok(());
            }
        }
    }
}

/// Background mode: re-exec `hex sched daemon` (without `--background`) as a
/// detached child process, write its PID, and exit the parent.
fn daemon_background(interval: u64, max_failures: u32) -> anyhow::Result<()> {
    // Already running?
    if let Some(pid) = read_pid_file() {
        if process_alive(pid) {
            println!(
                "{} pid={} (pid file: {})",
                "sched daemon already running".yellow(),
                pid,
                pid_file_path().display()
            );
            return Ok(());
        } else {
            // Stale pid file — clean it up before starting.
            remove_pid_file();
        }
    }

    let exe = std::env::current_exe()?;
    let child = std::process::Command::new(exe)
        .arg("sched")
        .arg("daemon")
        .arg("--interval")
        .arg(interval.to_string())
        .arg("--max-failures")
        .arg(max_failures.to_string())
        // Detach: swallow stdio so the child survives the parent.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    let pid = child.id();
    write_pid_file(pid)?;

    println!(
        "{} pid={} interval={}s",
        "⬡ sched daemon started in background".green().bold(),
        pid,
        interval
    );
    println!("  pid file: {}", pid_file_path().display());
    println!("  stop with: hex sched daemon-stop");
    Ok(())
}

/// Stop the background daemon: send SIGTERM, wait up to 5s, remove PID file.
fn daemon_stop() -> anyhow::Result<()> {
    let pid = match read_pid_file() {
        Some(pid) => pid,
        None => {
            println!(
                "{} (no pid file at {})",
                "sched daemon not running".yellow(),
                pid_file_path().display()
            );
            return Ok(());
        }
    };

    if !process_alive(pid) {
        println!(
            "{} pid={} not alive — removing stale pid file",
            "sched daemon".yellow(),
            pid
        );
        remove_pid_file();
        return Ok(());
    }

    println!("sending SIGTERM to pid {}...", pid);
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        anyhow::bail!("kill(pid={}, SIGTERM) failed: {}", pid, err);
    }

    // Wait up to 5s for the process to exit.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !process_alive(pid) {
            remove_pid_file();
            println!("{} pid={}", "⬡ sched daemon stopped".green().bold(), pid);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    eprintln!(
        "{} pid={} did not exit within 5s (pid file left in place)",
        "warning:".yellow(),
        pid
    );
    Ok(())
}

/// Restart the background daemon: stop then start fresh.
/// Detects current interval from the running daemon or uses default (30s).
fn daemon_restart() -> anyhow::Result<()> {
    // Read current settings from staleness record before stopping.
    let (interval, max_failures) = load_staleness_record()
        .map(|r| (r.interval_secs.max(10), r.max_failures.max(1)))
        .unwrap_or((30, 3));

    // Stop the running daemon.
    let was_running = read_pid_file().map(process_alive).unwrap_or(false);
    if was_running {
        daemon_stop()?;
    } else {
        println!("{} no daemon running — starting fresh", "⬡".cyan());
    }

    println!(
        "{} restarting with interval={}s max_failures={}",
        "⬡".green().bold(),
        interval,
        max_failures
    );
    daemon_background(interval, max_failures)
}

/// Show whether the sched daemon is running.
/// Also warns if the daemon binary appears stale (rebuilt since daemon started).
fn daemon_status() -> anyhow::Result<()> {
    match read_pid_file() {
        Some(pid) if process_alive(pid) => {
            println!(
                "{} pid={}",
                "⬡ sched daemon running".green().bold(),
                pid
            );
            println!("  pid file: {}", pid_file_path().display());
            // ADR-2026-04-24-1820: warn if daemon binary is stale.
            if let Some(true) = is_current_binary_stale() {
                println!(
                    "  {} daemon may be stale — binary was rebuilt. Restart with: hex sched daemon restart",
                    "⚠".yellow().bold()
                );
            }
        }
        Some(pid) => {
            println!(
                "{} pid={} (stale pid file)",
                "sched daemon not running".yellow(),
                pid
            );
            println!("  pid file: {}", pid_file_path().display());
        }
        None => {
            println!("{}", "sched daemon not running".yellow());
        }
    }
    // wp-idle-research-swarm P5.1: surface the most recent idle-sweep so
    // operators can see at a glance whether the autonomous research loop has
    // run lately and how productive it was. Silent when no sweep YAML exists.
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(line) = last_sweep_summary_line(&cwd) {
            println!("  last_sweep: {}", line);
        }
    }
    Ok(())
}
// ─── wp-brain-updates P3.1 / P2.1: Watch brain_tick events ─────────────────

/// Normalize a `--since` value to an ISO 8601 UTC timestamp.
///
/// Accepts:
///   - ISO 8601 / RFC 3339 (`2026-04-14T10:00:00Z`) — returned normalized
///   - humantime durations (`1h`, `30m`, `2h15m`, `7d`) — subtracted from now
///
/// Returns `Err` with a user-facing hint if neither parse succeeds.
fn parse_since(input: &str) -> anyhow::Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--since value is empty");
    }
    // Try RFC 3339 first (it's what the server emits, so the common case).
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.with_timezone(&chrono::Utc).to_rfc3339());
    }
    // Fall back to humantime duration.
    if let Ok(dur) = humantime::parse_duration(trimmed) {
        let now = chrono::Utc::now();
        let chrono_dur = chrono::Duration::from_std(dur)
            .map_err(|e| anyhow::anyhow!("duration out of range: {e}"))?;
        let cutoff = now
            .checked_sub_signed(chrono_dur)
            .ok_or_else(|| anyhow::anyhow!("--since {trimmed} underflows"))?;
        return Ok(cutoff.to_rfc3339());
    }
    anyhow::bail!(
        "--since must be ISO 8601 (e.g. 2026-04-14T10:00:00Z) or a duration (e.g. 1h, 30m, 2h15m); got {trimmed:?}"
    );
}

/// Stream new `brain_tick` events to stdout as they appear.
async fn watch(since: Option<String>) -> anyhow::Result<()> {
    // ADR-2026-04-24-1820: warn if daemon binary is stale before starting.
    if let Some(true) = is_current_binary_stale() {
        eprintln!(
            "⚠ {} daemon may be stale — results may not reflect current code. Restart with: hex sched daemon restart",
            "⚠".yellow().bold()
        );
    }

    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let url = format!("http://127.0.0.1:{}/api/events?limit=200", port);
    let client = reqwest::Client::new();

    // Resolve --since up front so bad input fails before we start polling.
    let normalized_since: Option<String> = match since.as_deref() {
        Some(s) => Some(parse_since(s)?),
        None => None,
    };

    println!(
        "{} (ctrl-C to exit)",
        "⬡ watching brain_tick events".green().bold()
    );
    if let (Some(raw), Some(norm)) = (since.as_deref(), normalized_since.as_deref()) {
        if raw == norm {
            println!("  since: {}", raw);
        } else {
            println!("  since: {} ({})", raw, norm);
        }
    }
    println!();

    // `last_seen` is the newest `created_at` we've printed so far. When
    // `since` is None, the first poll establishes a baseline without printing
    // backlog — the user asked to watch, not to replay history.
    let mut last_seen: Option<String> = normalized_since;
    let mut first_poll = last_seen.is_none();

    loop {
        match poll_brain_events(&client, &url, last_seen.as_deref()).await {
            Ok(events) => {
                // `events` is newest-first. `max` works regardless of order.
                let newest = events
                    .iter()
                    .filter_map(|ev| ev.get("created_at").and_then(|v| v.as_str()))
                    .max()
                    .map(|s| s.to_string());

                if first_poll {
                    // Seed baseline: record newest without printing.
                    if let Some(ts) = newest {
                        last_seen = Some(ts);
                    }
                    first_poll = false;
                } else {
                    // Print newest-first (server order, no reversal).
                    for ev in &events {
                        print_brain_event(ev);
                    }
                    if let Some(ts) = newest {
                        let advance = last_seen
                            .as_deref()
                            .map(|cur| ts.as_str() > cur)
                            .unwrap_or(true);
                        if advance {
                            last_seen = Some(ts);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("{} {}", "watch error:".yellow(), e);
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            _ = tokio::signal::ctrl_c() => {
                println!("\n{}", "⬡ watch stopped".yellow());
                return Ok(());
            }
        }
    }
}

/// Fetch recent events, filter to `brain_tick`, and keep only those strictly
/// newer than `since`. Returned newest-first (matches server order).
async fn poll_brain_events(
    client: &reqwest::Client,
    url: &str,
    since: Option<&str>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    if !resp.status().is_success() {
        anyhow::bail!("GET {} returned {}", url, resp.status());
    }
    let body: serde_json::Value = resp.json().await?;
    let events = body
        .get("events")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let filtered: Vec<serde_json::Value> = events
        .into_iter()
        .filter(|ev| ev.get("event_type").and_then(|v| v.as_str()) == Some("brain_tick"))
        .filter(|ev| match since {
            Some(cutoff) => ev
                .get("created_at")
                .and_then(|v| v.as_str())
                .map(|ts| ts > cutoff)
                .unwrap_or(false),
            None => true,
        })
        .collect();

    Ok(filtered)
}

fn print_brain_event(ev: &serde_json::Value) {
    let created_at = ev.get("created_at").and_then(|v| v.as_str()).unwrap_or("?");
    let duration_ms = ev.get("duration_ms").and_then(|v| v.as_i64()).unwrap_or(0);
    let session_id = ev
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    println!(
        "  {}  brain_tick  session={} duration={}ms",
        created_at.dimmed(),
        truncate(session_id, 12),
        duration_ms,
    );
}

// ─── ADR-2026-04-13-2330: Brain task queue (HexFlo memory–backed) ───────────────

const NEXUS_BASE: &str = "http://127.0.0.1:5555";

// ─── Typed schema (ADR-2026-04-14-1400 P0.1) ────────────────────────────────────
//
// The on-wire JSON shape is shared across daemon/CLI/dashboard. A typed enum
// replaces the string-stamped `"status"` so variants are enforced at compile
// time and migration to lease-aware semantics in P1+ is type-safe.
//
// Every added field uses `#[serde(default)]` so records written by older
// daemons (pre-lease schema) deserialize without error — the queue does not
// require a stop-the-world migration.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BrainTaskStatus {
    Pending,
    Leased,
    InProgress,
    Completed,
    Failed,
    /// Quarantined: task failed, exhausted retry budget, AND the
    /// underlying problem appears non-transient. Excluded from auto-retry
    /// scans so a structurally-broken task can't camp the failed pool
    /// forever. Surfaced via the `dead_letter` improver detector so
    /// operators can see the queue's accumulated unfixables.
    DeadLetter,
}

impl BrainTaskStatus {
    /// Wire-format string. Kept stable across schema revisions — storage
    /// keys, REST payloads, and filter predicates all lean on these exact
    /// tokens.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            BrainTaskStatus::Pending => "pending",
            BrainTaskStatus::Leased => "leased",
            BrainTaskStatus::InProgress => "in_progress",
            BrainTaskStatus::Completed => "completed",
            BrainTaskStatus::Failed => "failed",
            BrainTaskStatus::DeadLetter => "dead_letter",
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            BrainTaskStatus::Completed
                | BrainTaskStatus::Failed
                | BrainTaskStatus::DeadLetter
        )
    }

    /// Parse a wire string into an enum variant. Tolerant of unknown values
    /// (returns `None`) so a new variant written by a newer daemon doesn't
    /// crash an older reader.
    pub(crate) fn from_wire(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(BrainTaskStatus::Pending),
            "leased" => Some(BrainTaskStatus::Leased),
            "in_progress" => Some(BrainTaskStatus::InProgress),
            "completed" => Some(BrainTaskStatus::Completed),
            "failed" => Some(BrainTaskStatus::Failed),
            "dead_letter" => Some(BrainTaskStatus::DeadLetter),
            _ => None,
        }
    }
}

/// Evidence surfaced by the lease sweeper / reconciler to justify a
/// completion verdict (ADR-2026-04-14-1400). Populated in P2+; defaults keep
/// older records valid.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct BrainTaskEvidence {
    #[serde(default)]
    pub(crate) commits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reconcile_verdict: Option<String>,
}

/// Typed value written at `hexflo-memory[brain-task:<id>]`. The JSON on
/// the wire stays compatible with the legacy shape — unknown fields are
/// preserved implicitly by the writer (which still builds via
/// `serde_json::Value` to avoid field-dropping round-trips), and missing
/// fields default to neutral values on read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BrainTaskRecord {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) payload: String,
    pub(crate) status: BrainTaskStatus,
    #[serde(default)]
    pub(crate) project_id: String,
    #[serde(default)]
    pub(crate) created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<String>,

    // ─── Timeout (P2.1: stored at enqueue, used by sweep) ──────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) timeout_s: Option<u64>,

    // ─── Lease fields (P0.1 schema extension) ─────────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) leased_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) leased_until: Option<String>,
    #[serde(default)]
    pub(crate) lease_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) swarm_task_id: Option<String>,
    #[serde(default)]
    pub(crate) evidence: BrainTaskEvidence,

    // ─── Scheduling priority ────────────────────────────────────────────
    // 0 = normal (default), higher = more urgent. The daemon drains pending
    // tasks in priority-desc, created_at-asc order. Lets urgent frontier-
    // bound work bypass speculative local-Ollama loops without flushing the
    // whole queue.
    #[serde(default)]
    pub(crate) priority: u8,
}

impl BrainTaskRecord {
    /// Parse a JSON value into a record, tolerating missing lease fields and
    /// unknown status strings (status falls back to `Pending` if
    /// unrecognised so the queue stays drainable). Returns `None` only when
    /// the mandatory id/kind/payload triple is missing — those are structural.
    pub(crate) fn from_value(v: &serde_json::Value) -> Option<Self> {
        // Go through serde first so `#[serde(default)]` does the heavy
        // lifting for missing fields. Fall back to a hand-rolled parse if
        // the status string isn't a known variant — we don't want an
        // unrecognised status to nuke the whole record.
        if let Ok(rec) = serde_json::from_value::<BrainTaskRecord>(v.clone()) {
            return Some(rec);
        }
        let id = v.get("id").and_then(|x| x.as_str())?.to_string();
        let kind = v.get("kind").and_then(|x| x.as_str())?.to_string();
        let payload = v.get("payload").and_then(|x| x.as_str())?.to_string();
        let status = v
            .get("status")
            .and_then(|x| x.as_str())
            .and_then(BrainTaskStatus::from_wire)
            .unwrap_or(BrainTaskStatus::Pending);
        let project_id = v.get("project_id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let created_at = v.get("created_at").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let completed_at = v.get("completed_at").and_then(|x| x.as_str()).map(String::from);
        let result = v.get("result").and_then(|x| x.as_str()).map(String::from);
        let timeout_s = v.get("timeout_s").and_then(|x| x.as_u64());
        let leased_to = v.get("leased_to").and_then(|x| x.as_str()).map(String::from);
        let leased_until = v.get("leased_until").and_then(|x| x.as_str()).map(String::from);
        let lease_attempts = v
            .get("lease_attempts")
            .and_then(|x| x.as_u64())
            .map(|n| n as u32)
            .unwrap_or(0);
        let swarm_task_id = v.get("swarm_task_id").and_then(|x| x.as_str()).map(String::from);
        let evidence = v
            .get("evidence")
            .cloned()
            .and_then(|e| serde_json::from_value::<BrainTaskEvidence>(e).ok())
            .unwrap_or_default();
        let priority = v
            .get("priority")
            .and_then(|x| x.as_u64())
            .map(|n| n.min(u8::MAX as u64) as u8)
            .unwrap_or(0);
        Some(BrainTaskRecord {
            id,
            kind,
            payload,
            status,
            project_id,
            created_at,
            completed_at,
            result,
            timeout_s,
            leased_to,
            leased_until,
            lease_attempts,
            swarm_task_id,
            evidence,
            priority,
        })
    }
}

// ─── Lease durations per kind (ADR-2026-04-14-1400 P1.1) ────────────────────────
//
// Bounded lease windows are what make the sweeper safe: a task that holds a
// lease past its window is assumed stuck and reclaimable. Each kind gets a
// duration tuned to its expected runtime — a workplan may legitimately run
// for 30 minutes, while a shell command that takes longer than 2 minutes is
// almost certainly wedged.
//
// The table is the single source of truth shared with hex-nexus's TaskKind
// enum (hex-nexus/src/routes/brain.rs). Any new variant over there must
// land an entry here, otherwise `lease_for` falls through to the shell
// timeout and legitimate long-runners get reaped mid-flight.

// Lease windows are sized for ~5 tok/s local-model output (qwen2.5-coder:32b
// benchmarked May 2026 at 5 tok/s on this Bazzite host). Frontier rates are
// 10–15× faster, so these windows leave tasks 90% idle on Claude — the cost
// is reclamation latency on stuck local-model tasks, not throughput.
pub(crate) const LEASE_DURATIONS: [(&str, Duration); 5] = [
    ("workplan", Duration::from_secs(60 * 60)),
    ("hex-command", Duration::from_secs(15 * 60)),
    ("analyze", Duration::from_secs(10 * 60)),
    ("shell", Duration::from_secs(10 * 60)),
    ("remote-shell", Duration::from_secs(5 * 60)),
];

/// Default lease window for `kind`. Unknown kinds fall back to the
/// shell-style 10-minute timeout so a typo or a newly-added kind can't camp
/// the queue forever — the sweeper will still reclaim it, just on the
/// shorter-than-ideal schedule until the table is updated.
pub(crate) fn lease_for(kind: &str) -> Duration {
    LEASE_DURATIONS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, d)| *d)
        .unwrap_or(Duration::from_secs(10 * 60))
}

pub async fn enqueue_brain_task_pub(kind: &str, payload: &str) -> anyhow::Result<String> {
    enqueue_brain_task(kind, payload).await
}

/// Resolve project ID from `.hex/project.json` in cwd. Returns `None` if missing/unreadable.
fn brain_project_id() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let content = std::fs::read_to_string(cwd.join(".hex/project.json")).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed["id"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
}

/// Fire-and-forget operator notification on sched task completion/failure
/// (wp-brain-updates P1.1). priority=1 on Completed, priority=2 on Failed.
/// Called inline from the daemon loop after each task state transition so
/// operators see outcomes as they happen, not on next pulse.
async fn notify_brain_task_result(
    task_id: &str,
    kind: &str,
    payload: &str,
    status: &str,
    result: &str,
) {
    let priority: u8 = if status == "completed" { 1 } else { 2 };
    let snippet: String = result.chars().take(200).collect();
    let body = json!({
        "task_id": task_id,
        "kind": kind,
        "payload": payload,
        "status": status,
        "result_snippet": snippet,
    });
    notify_operator(&format!("brain.task.{}", status), body, priority).await;
}

async fn enqueue_brain_task(kind: &str, payload: &str) -> anyhow::Result<String> {
    enqueue_brain_task_with_priority(kind, payload, 0).await
}

async fn enqueue_brain_task_with_priority(kind: &str, payload: &str, priority: u8) -> anyhow::Result<String> {
    use crate::nexus_client::NexusClient;

    // Reject "audit theater" stubs: shell tasks whose payload is just an echo
    // of a FIXME/TODO/NOTE marker. These drain in milliseconds with exit 0,
    // inflating queue throughput while accomplishing nothing. FIXMEs belong in
    // ADRs or source comments; actionable work belongs in workplan tasks.
    if kind == "shell" {
        let stripped = payload.trim_start();
        let is_echo_stub = stripped.starts_with("echo ")
            && (stripped.to_ascii_uppercase().contains("FIXME")
                || stripped.to_ascii_uppercase().contains("TODO")
                || stripped.to_ascii_uppercase().contains("NOTE:"));
        if is_echo_stub {
            anyhow::bail!(
                "refusing to enqueue shell stub: `echo FIXME/TODO/NOTE ...` is audit theater, \
                 not work. If it needs design → write an ADR. If it needs execution → write a \
                 workplan and enqueue it with `hex sched enqueue workplan <path>`. If it's a \
                 breadcrumb → put it in a TODO comment at the code site."
            );
        }
    }

    // Capture project scope at enqueue time — without this the brain queue is
    // global and tasks enqueued in one repo pollute another repo's statusline.
    let project_id = brain_project_id().unwrap_or_default();

    // Dedup: if an active (pending/in_progress) task with the same
    // (kind, payload, project_id) triplet already exists, return its id
    // rather than creating a duplicate. Multiple enqueue sites
    // (hex sched prime, hex sched enqueue, other agents) would otherwise
    // stack up redundant work.
    if let Ok(existing) = list_brain_tasks(None).await {
        for task in &existing {
            let t_status = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if !matches!(t_status, "pending" | "in_progress") {
                continue;
            }
            let t_kind = task.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let t_payload = task.get("payload").and_then(|v| v.as_str()).unwrap_or("");
            let t_project = task.get("project_id").and_then(|v| v.as_str()).unwrap_or("");
            if t_kind == kind && t_payload == payload && t_project == project_id {
                if let Some(id) = task.get("id").and_then(|v| v.as_str()) {
                    return Ok(id.to_string());
                }
            }
        }
    }

    // For workplan tasks, read timeout_s from the workplan JSON so the
    // daemon can honour per-workplan lease windows. Falls back to the
    // default lease_for() duration when the field is absent or the file
    // can't be read (e.g. the payload is a workplan ID, not a path).
    let timeout: u64 = if kind == "workplan" {
        let workplan_content = std::fs::read_to_string(payload)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok());

        // Validate critical path safety before enqueueing
        if let Some(ref wp) = workplan_content {
            if let Some(phases) = wp.get("phases").and_then(|v| v.as_array()) {
                for phase in phases {
                    if let Some(tasks) = phase.get("tasks").and_then(|v| v.as_array()) {
                        for task in tasks {
                            if let Some(files) = task.get("files").and_then(|v| v.as_array()) {
                                let file_paths: Vec<String> = files
                                    .iter()
                                    .filter_map(|f| f.as_str().map(String::from))
                                    .collect();

                                let blocked: Vec<String> = file_paths
                                    .iter()
                                    .filter(|p| hex_core::validation::is_critical_path(p))
                                    .cloned()
                                    .collect();
                                if !blocked.is_empty() {
                                    let task_id = task.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                                    eprintln!("⚠ Workplan rejected: task {} targets protected files:", task_id);
                                    for file in &blocked {
                                        eprintln!("  - {}", file);
                                    }
                                    anyhow::bail!(
                                        "Cannot enqueue workplan targeting critical infrastructure. \
                                         Protected files: {}",
                                        blocked.join(", ")
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        workplan_content
            .and_then(|wp| wp.get("timeout_s")?.as_u64())
            .unwrap_or_else(|| lease_for(kind).as_secs())
    } else {
        lease_for(kind).as_secs()
    };

    let id = uuid::Uuid::new_v4().to_string();
    let key = format!("brain-task:{}", id);
    let task = json!({
        "id": id,
        "kind": kind,
        "payload": payload,
        "status": "pending",
        "project_id": project_id,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "completed_at": serde_json::Value::Null,
        "result": serde_json::Value::Null,
        "timeout_s": timeout,
        "priority": priority,
    });
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;
    nexus
        .post(
            "/api/hexflo/memory",
            &json!({"key": key, "value": task.to_string()}),
        )
        .await?;
    debug!(
        task_id = %id,
        kind = %kind,
        project_id = %project_id,
        payload_len = payload.len(),
        "drain-path: enqueue"
    );
    Ok(id)
}

pub(crate) async fn list_brain_tasks(
    status_filter: Option<&str>,
) -> anyhow::Result<Vec<serde_json::Value>> {
    use crate::nexus_client::NexusClient;
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;
    let body: serde_json::Value = nexus
        .get("/api/hexflo/memory/search?q=brain-task:")
        .await?;
    let results = body
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut tasks = Vec::new();
    for item in results {
        // Response shape: [{"key": "brain-task:...", "value": "<json string>"}, ...]
        if let Some(value_str) = item.get("value").and_then(|v| v.as_str()) {
            if let Ok(task) = serde_json::from_str::<serde_json::Value>(value_str) {
                let status = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(filter) = status_filter {
                    if status != filter {
                        continue;
                    }
                }
                tasks.push(task);
            }
        }
    }
    Ok(tasks)
}

/// Drain up to `limit` pending sched tasks. Reserved for the future sched daemon tick loop
/// (P3 of ADR-2026-04-13-2330). Also invoked by `hex brain queue drain` logic indirectly.
///
/// Tasks are drained in (priority desc, created_at asc) order — higher-priority
/// work jumps the queue without needing a flush. Equal-priority tasks fall back
/// to FIFO so normal traffic stays fair. Missing/garbage `priority` defaults to 0.
#[allow(dead_code)]
pub(crate) async fn drain_brain_tasks(limit: usize) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut pending = list_brain_tasks(Some("pending")).await?;
    pending.sort_by(|a, b| {
        let pa = a.get("priority").and_then(|v| v.as_u64()).unwrap_or(0);
        let pb = b.get("priority").and_then(|v| v.as_u64()).unwrap_or(0);
        // Priority desc first; then created_at asc for FIFO at equal priority.
        pb.cmp(&pa).then_with(|| {
            let ca = a.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
            let cb = b.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
            ca.cmp(cb)
        })
    });
    let claimed: Vec<_> = pending.into_iter().take(limit).collect();
    for task in &claimed {
        let id = task.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let kind = task.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let priority = task.get("priority").and_then(|v| v.as_u64()).unwrap_or(0);
        debug!(task_id = %id, kind = %kind, priority = priority, "drain-path: claim");
    }
    Ok(claimed)
}

pub(crate) async fn update_brain_task(
    id: &str,
    status: BrainTaskStatus,
    result: &str,
) -> anyhow::Result<()> {
    use crate::nexus_client::NexusClient;
    let key = format!("brain-task:{}", id);
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;
    // GET current task value
    let resp: serde_json::Value = nexus.get(&format!("/api/hexflo/memory/{}", key)).await?;
    // Response shape: {"key": ..., "value": "<json string>"} or just the value.
    // We round-trip through serde_json::Value (not BrainTaskRecord) so lease
    // fields written by future daemons survive the update — we only mutate
    // the keys we own here, leaving everything else verbatim.
    let value_str = resp
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let mut inner: serde_json::Value =
        serde_json::from_str(value_str).unwrap_or(serde_json::json!({}));
    if let Some(obj) = inner.as_object_mut() {
        obj.insert("status".into(), json!(status.as_str()));
        obj.insert("result".into(), json!(result));
        if status.is_terminal() {
            obj.insert(
                "completed_at".into(),
                json!(chrono::Utc::now().to_rfc3339()),
            );
        }
    }
    if status.is_terminal() {
        let result_preview: String = result.chars().take(120).collect();
        debug!(
            task_id = %id,
            status = %status.as_str(),
            result_preview = %result_preview,
            "drain-path: terminate"
        );
    }
    nexus
        .post(
            "/api/hexflo/memory",
            &json!({"key": key, "value": inner.to_string()}),
        )
        .await?;
    Ok(())
}

// ─── Timeout sweep (ADR-2026-04-14-2155 P2.2) ──────────────────────────────────
//
// Weak fairness guarantee: every in_progress task eventually reaches a
// terminal state. The daemon calls sweep_stuck_tasks() each tick, which
// scans for in_progress tasks whose age exceeds timeout_s + 30s grace
// and flips them to Failed. The 30s grace prevents races where a task
// completes legitimately at the timeout boundary.

const SWEEP_GRACE_SECS: u64 = 30;

pub(crate) async fn sweep_stuck_tasks() -> anyhow::Result<Vec<String>> {
    let in_progress = list_brain_tasks(Some("in_progress")).await?;
    let now = chrono::Utc::now();
    let mut swept: Vec<String> = Vec::new();

    for task_val in &in_progress {
        let Some(rec) = BrainTaskRecord::from_value(task_val) else {
            continue;
        };
        let created = match chrono::DateTime::parse_from_rfc3339(&rec.created_at) {
            Ok(dt) => dt.with_timezone(&chrono::Utc),
            Err(_) => continue,
        };
        let timeout = rec
            .timeout_s
            .unwrap_or_else(|| lease_for(&rec.kind).as_secs());
        let deadline_secs = timeout + SWEEP_GRACE_SECS;
        let age = now.signed_duration_since(created);
        if age.num_seconds() < deadline_secs as i64 {
            continue;
        }
        let reason = format!(
            "timeout sweep: in_progress for {}s exceeds {}s (timeout_s={} + grace={})",
            age.num_seconds(),
            deadline_secs,
            timeout,
            SWEEP_GRACE_SECS,
        );
        debug!(task_id = %rec.id, age_s = %age.num_seconds(), deadline_s = %deadline_secs, "sweep: failing stuck task");
        if let Err(err) = update_brain_task(&rec.id, BrainTaskStatus::Failed, &reason).await {
            eprintln!("  {} sweep update failed for {}: {}", "✗".red(), rec.id, err);
            continue;
        }
        swept.push(rec.id.clone());
    }
    Ok(swept)
}

/// Default age threshold before a terminal (failed/completed) task is swept (7 days).
const TERMINAL_SWEEP_DAYS: i64 = 7;

/// Sweep old terminal tasks — delete completed/failed records older than 7 days.
/// Prevents unbounded SpacetimeDB growth. Uses a date-based throttle so deletion
/// only actually runs on the first tick of each UTC day (not every 30s).
async fn sweep_old_terminal_tasks(state: &mut DaemonState) {
    // Throttle: only run deletion once per UTC day (stored in brain-state).
    let today = chrono::Utc::now().date_naive().to_string();
    if state.sweep_date == today {
        return; // already ran today
    }

    let cutoff = chrono::Utc::now() - chrono::Duration::days(TERMINAL_SWEEP_DAYS);
    let cutoff_str = cutoff.to_rfc3339();

    // Collect terminal tasks older than cutoff
    let old_tasks: Vec<String> = match list_brain_tasks(None).await {
        Ok(tasks) => tasks
            .into_iter()
            .filter(|t| {
                let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if !matches!(status, "failed" | "completed") {
                    return false;
                }
                let completed_at = t.get("completed_at")
                    .or_else(|| t.get("created_at"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                completed_at < cutoff_str.as_str()
            })
            .filter_map(|t| t.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect(),
        Err(_) => return,
    };

    if old_tasks.is_empty() {
        state.sweep_date = today;
        return;
    }

    // Delete each old task from SpacetimeDB via DELETE /api/hexflo/memory/{key}
    let nexus = crate::nexus_client::NexusClient::from_env();
    if nexus.ensure_running().await.is_err() {
        return;
    }

    let mut deleted = 0;
    for task_id in &old_tasks {
        let key = format!("brain-task:{}", task_id);
        if nexus.delete(&format!("/api/hexflo/memory/{}", key)).await.is_ok() {
            deleted += 1;
        }
    }

    if deleted > 0 {
        println!("  {} swept {} old terminal tasks (>{}-day)", "🗑".cyan(), deleted, TERMINAL_SWEEP_DAYS);
    }
    state.sweep_date = today;
}
//
// The direct-subprocess flow (`execute_brain_task` called inline from the
// daemon tick) is being replaced by a lease handoff: the daemon registers a
// swarm task referencing the sched task, stamps `Leased` + `leased_until`,
// and walks away. Swarm workers pick the task up, progress it through
// `InProgress`, and a later confirm-complete path finalises
// `Completed`/`Failed`. If the lease expires without progress, the sweeper
// (P2) flips the task back to `Pending` and bumps `lease_attempts`.
//
// Keeping `execute_brain_task` alive below is deliberate: P1.3 will have
// `dispatch_brain_task` short-circuit to it for `shell` and `hex-command`
// kinds so trivial tasks don't pay the swarm round-trip.

/// Handle returned by [`dispatch_brain_task`]. Captures the ids the sweeper
/// (P2) and confirm-complete path need to correlate a sched task with the
/// swarm that holds its lease. `brain_task_id` and `leased_to` are surfaced
/// verbatim for callers that log or reconcile handles — the sweeper reads
/// them after a lease expires.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct LeaseHandle {
    pub(crate) brain_task_id: String,
    pub(crate) swarm_id: String,
    pub(crate) swarm_task_id: String,
    pub(crate) leased_to: String,
    pub(crate) leased_until: String,
}

/// Result of `dispatch_brain_task` as seen by the daemon drain loop
/// (ADR-2026-04-14-1400 §1 inline-fallback). Separated from `LeaseHandle`
/// because the daemon's decision about whether to inline-execute depends
/// on whether a live worker actually holds the lease, not just whether
/// the swarm/task records were created.
#[derive(Debug, Clone)]
pub(crate) enum DispatchOutcome {
    /// Dispatch registered a swarm task AND a live worker is polling the
    /// swarm. Daemon should NOT fall back; the worker will progress it.
    #[allow(dead_code)]
    LeasedToWorker {
        swarm_id: String,
        swarm_task_id: String,
        #[allow(dead_code)]
        agent_id: String,
        leased_until: String,
    },
    /// Dispatch registered a swarm task but no worker is registered against
    /// it. This is the default outcome today because §2 worker registration
    /// has not shipped. Daemon MUST fall back to inline `execute_brain_task`
    /// so the evidence guard runs and the task doesn't sit `leased` forever.
    LeasedEmpty {
        #[allow(dead_code)]
        swarm_id: String,
        #[allow(dead_code)]
        swarm_task_id: String,
    },
    /// Dispatch itself failed (nexus down, swarm create errored, etc.).
    /// Daemon falls back to inline so the task still makes progress.
    Error(String),
}

/// Pure predicate: should the daemon fall back to inline execute_brain_task?
/// Tested in isolation so the fallback decision is verifiable without
/// standing up a full daemon loop.
pub(crate) fn should_fallback_inline(outcome: &DispatchOutcome) -> bool {
    matches!(
        outcome,
        DispatchOutcome::Error(_) | DispatchOutcome::LeasedEmpty { .. }
    )
}

/// Classify a successful `dispatch_brain_task` result into a
/// [`DispatchOutcome`]. Currently treats every lease as empty because no
/// swarm workers register against `brain-lease` — that's the §2 gap the
/// inline fallback papers over. Once §2 lands, this helper will probe the
/// swarm's live-agent list and return `LeasedToWorker` when applicable.
pub(crate) async fn classify_dispatch(handle: &LeaseHandle) -> DispatchOutcome {
    // TODO(§2): query /api/swarms/{swarm_id}/agents and return
    // LeasedToWorker when a registered agent exists. For now, every lease
    // is empty in practice — see ADR-2026-04-14-1400 §1 "Known gaps".
    DispatchOutcome::LeasedEmpty {
        swarm_id: handle.swarm_id.clone(),
        swarm_task_id: handle.swarm_task_id.clone(),
    }
}

/// Hand a pending sched task off to a `sched-lease` swarm. Returns a
/// [`LeaseHandle`] the daemon can surface to the sweeper.
///
/// Steps:
/// 1. Find or create a `sched-lease` swarm scoped to the task's project.
/// 2. Register a swarm task whose title embeds `sched-task:<id>` so workers
///    polling the swarm can resolve back to the sched task.
/// 3. Stamp `Leased` + `leased_to` (swarm id) + `leased_until` (wall clock
///    deadline from [`lease_for`]) + `swarm_task_id` on the sched task
///    record, and bump `lease_attempts`.
pub(crate) async fn dispatch_brain_task(
    task: &serde_json::Value,
) -> anyhow::Result<LeaseHandle> {
    use crate::nexus_client::NexusClient;

    let brain_task_id = task
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("sched task missing id"))?
        .to_string();
    let kind = task
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let payload = task
        .get("payload")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let project_id = task
        .get("project_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let swarm_id = match find_brain_lease_swarm(&nexus, &project_id).await? {
        Some(id) => id,
        None => create_brain_lease_swarm(&nexus, &project_id).await?,
    };

    // Truncate payload in the title so a runaway multi-KB shell payload
    // doesn't blow up swarm UIs. The full payload stays on the sched task.
    let payload_snippet: String = payload.chars().take(80).collect();
    let title = format!("brain-task:{} [{}] {}", brain_task_id, kind, payload_snippet);
    let resp = nexus
        .post(
            &format!("/api/swarms/{}/tasks", swarm_id),
            &json!({ "title": title, "dependsOn": "" }),
        )
        .await?;
    let swarm_task_id = resp
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("create swarm task response missing id"))?
        .to_string();

    let window_secs = task
        .get("timeout_s")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| lease_for(&kind).as_secs());
    let window = Duration::from_secs(window_secs);
    let leased_until = (chrono::Utc::now()
        + chrono::Duration::from_std(window).unwrap_or_else(|_| chrono::Duration::minutes(2)))
    .to_rfc3339();
    let leased_to = swarm_id.clone();

    stamp_brain_task_lease(&brain_task_id, &leased_to, &leased_until, &swarm_task_id).await?;

    debug!(
        task_id = %brain_task_id,
        kind = %kind,
        swarm_id = %swarm_id,
        swarm_task_id = %swarm_task_id,
        leased_until = %leased_until,
        "drain-path: dispatch"
    );

    Ok(LeaseHandle {
        brain_task_id,
        swarm_id,
        swarm_task_id,
        leased_to,
        leased_until,
    })
}

/// Look up an existing active `brain-lease` swarm for `project_id`. Returns
/// `None` if no matching swarm is active — the caller creates one.
async fn find_brain_lease_swarm(
    nexus: &crate::nexus_client::NexusClient,
    project_id: &str,
) -> anyhow::Result<Option<String>> {
    let resp: serde_json::Value = nexus.get("/api/swarms/active").await?;
    // Response may be either a raw array or `{"swarms": [...]}`. Handle both
    // so we don't break if the envelope changes.
    let swarms = if let Some(arr) = resp.as_array() {
        arr.clone()
    } else if let Some(arr) = resp.get("swarms").and_then(|v| v.as_array()) {
        arr.clone()
    } else {
        Vec::new()
    };
    for s in swarms {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let sp = s
            .get("projectId")
            .or_else(|| s.get("project_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if name == "brain-lease" && sp == project_id {
            if let Some(id) = s.get("id").and_then(|v| v.as_str()) {
                return Ok(Some(id.to_string()));
            }
        }
    }
    Ok(None)
}

async fn create_brain_lease_swarm(
    nexus: &crate::nexus_client::NexusClient,
    project_id: &str,
) -> anyhow::Result<String> {
    let resp = nexus
        .post(
            "/api/swarms",
            &json!({
                "projectId": project_id,
                "name": "brain-lease",
                "topology": "hierarchical",
            }),
        )
        .await?;
    resp.get("id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("create swarm response missing id"))
}

/// Mirror of [`update_brain_task`] that writes lease metadata instead of
/// `result`. Keeps the round-trip-through-Value pattern so unknown fields
/// added by other writers (sweeper evidence, etc.) survive the merge.
async fn stamp_brain_task_lease(
    brain_task_id: &str,
    leased_to: &str,
    leased_until: &str,
    swarm_task_id: &str,
) -> anyhow::Result<()> {
    use crate::nexus_client::NexusClient;
    let key = format!("brain-task:{}", brain_task_id);
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;
    let resp: serde_json::Value = nexus.get(&format!("/api/hexflo/memory/{}", key)).await?;
    let value_str = resp.get("value").and_then(|v| v.as_str()).unwrap_or("{}");
    let mut inner: serde_json::Value =
        serde_json::from_str(value_str).unwrap_or(serde_json::json!({}));
    if let Some(obj) = inner.as_object_mut() {
        obj.insert("status".into(), json!(BrainTaskStatus::Leased.as_str()));
        obj.insert("leased_to".into(), json!(leased_to));
        obj.insert("leased_until".into(), json!(leased_until));
        obj.insert("swarm_task_id".into(), json!(swarm_task_id));
        let attempts = obj
            .get("lease_attempts")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        obj.insert("lease_attempts".into(), json!(attempts + 1));
    }
    nexus
        .post(
            "/api/hexflo/memory",
            &json!({"key": key, "value": inner.to_string()}),
        )
        .await?;
    Ok(())
}

/// Return the current HEAD SHA of the workspace git repo, or `None` on any
/// failure (not a repo, git missing, subprocess error). Used by the
/// workplan-evidence guard in [`execute_brain_task`]: if HEAD is unchanged
/// before and after `hex plan execute` runs, we know the subprocess did no
/// real work regardless of its exit code (ADR-2026-04-14-1400 §1 P1).
/// Read the analyze interval from `.hex/project.json` (key
/// `brain.analyze_interval_secs`). Falls back to `DEFAULT_ANALYZE_INTERVAL_SECS`
/// when absent or unreadable.
fn load_analyze_interval_secs() -> u64 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return DEFAULT_ANALYZE_INTERVAL_SECS,
    };
    let content = match std::fs::read_to_string(cwd.join(".hex/project.json")) {
        Ok(c) => c,
        Err(_) => return DEFAULT_ANALYZE_INTERVAL_SECS,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(_) => return DEFAULT_ANALYZE_INTERVAL_SECS,
    };
    parsed
        .get("brain")
        .and_then(|b| b.get("analyze_interval_secs"))
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_ANALYZE_INTERVAL_SECS)
}

/// Read whether analyze is enabled from `.hex/project.json`
/// (key `brain.analyze_enabled`). Default true.
fn is_analyze_enabled() -> bool {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(_) => return true,
    };
    let content = match std::fs::read_to_string(cwd.join(".hex/project.json")) {
        Ok(c) => c,
        Err(_) => return true,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(_) => return true,
    };
    parsed
        .get("brain")
        .and_then(|b| b.get("analyze_enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Check if the periodic analyze task should be enqueued this tick.
/// Returns true if: analysis is enabled, no analyze task is already
/// pending/in_progress for this project, and the interval has elapsed.
async fn should_enqueue_analyze(state: &DaemonState) -> bool {
    if !is_analyze_enabled() {
        return false;
    }

    // Skip if an analyze task is already pending/in_progress
    if let Ok(tasks) = list_brain_tasks(None).await {
        for t in &tasks {
            let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status != "pending" && status != "in_progress" {
                continue;
            }
            let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "analyze" {
                return false;
            }
        }
    }

    // Check interval
    let interval_secs = load_analyze_interval_secs();
    let now = chrono::Utc::now();

    if let Some(last) = &state.last_analyze_at {
        if let Ok(last_time) = chrono::DateTime::parse_from_rfc3339(last) {
            let elapsed = now.signed_duration_since(last_time.with_timezone(&chrono::Utc));
            if elapsed.num_seconds() < interval_secs as i64 {
                return false;
            }
        }
    }

    true
}

/// Enqueue an "analyze" task for the current project. Returns the task ID.
async fn enqueue_analyze_task() -> anyhow::Result<String> {
    let project_id = brain_project_id().unwrap_or_default();
    let payload = serde_json::json!({
        "project_id": project_id,
        "command": "hex analyze . --json",
    });
    enqueue_brain_task("analyze", &payload.to_string()).await
}

/// Summary of an analyze regression detection run.
struct AnalyzeRegressionSummary {
    regressions: Vec<(String, usize, usize)>,
    current: HashMap<String, usize>,
    previous: HashMap<String, usize>,
}

/// Parse violation counts from a `hex analyze --json` output string.
/// Returns a HashMap keyed by category (e.g. "boundary_violations",
/// "dead_exports", "circular_deps") with the count as value.
fn parse_analyze_summary(output: &str) -> HashMap<String, usize> {
    let mut summary = HashMap::new();
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) {
        let get_count = |key| {
            parsed
                .get(key)
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(0)
        };
        summary.insert(
            "boundary_violations".to_string(),
            get_count("boundary_violations"),
        );
        summary.insert("dead_exports".to_string(), get_count("dead_exports"));
        summary.insert("circular_deps".to_string(), get_count("circular_deps"));
        summary.insert("unused_adapters".to_string(), get_count("unused_adapters"));
        if let Some(violations) = parsed.get("violations").and_then(|v| v.as_array()) {
            summary.insert("total_violations".to_string(), violations.len());
        }
    }
    summary
}

/// Check if the most recently completed analyze task has more violations
/// than the last recorded summary. Returns Some(summary) if regressions found,
/// None otherwise. Updates state.last_analysis_summary on call.
async fn check_analyze_regression(state: &DaemonState) -> Option<AnalyzeRegressionSummary> {
    // Find the most recently completed analyze task
    if let Ok(tasks) = list_brain_tasks(Some("completed")).await {
        let mut analyze_tasks: Vec<_> = tasks
            .into_iter()
            .filter(|t| {
                t.get("kind").and_then(|v| v.as_str()) == Some("analyze")
            })
            .collect();
        analyze_tasks.sort_by(|a, b| {
            let a_time = a
                .get("completed_at")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let b_time = b
                .get("completed_at")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            b_time.cmp(a_time) // descending — most recent first
        });

        if let Some(last_task) = analyze_tasks.first() {
            let result = last_task
                .get("result")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let current = parse_analyze_summary(result);
            let previous = state.last_analysis_summary.clone();

            let mut regressions = Vec::new();
            for (key, curr_count) in &current {
                if let Some(&prev_count) = previous.get(key) {
                    if curr_count > &prev_count {
                        regressions.push((key.clone(), prev_count, *curr_count));
                    }
                }
            }

            if !regressions.is_empty() {
return Some(AnalyzeRegressionSummary {
                    regressions,
                    current,
                    previous,
                });
            }
        }
    }
    None
}

fn git_head_sha() -> Option<String> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&workspace_root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Pure evidence-check used by the workplan branch of [`execute_brain_task`]
/// and by the unit test `test_workplan_no_evidence`. Returns
/// `(success, snippet_suffix)` given the subprocess exit status and the
/// pre/post HEAD shas. A workplan that exits 0 but leaves HEAD unchanged is
/// ADR-2026-04-14-2155 P2.3: output-level evidence guard mirroring
/// `hex_nexus::orchestration::workplan_executor::validate_dispatch_evidence`.
/// Rejects empty / whitespace-only executor output so vacuous acks like
/// `"Execution dispatched: Object {"` cannot promote a task to `completed`.
pub(crate) fn validate_dispatch_evidence(output: Option<&str>) -> Result<(), String> {
    match output {
        Some(s) if !s.trim().is_empty() => Ok(()),
        Some(_) => Err(
            "dispatch-evidence guard: executor produced whitespace-only output — \
             refusing to accept completion (ADR-2026-04-11-1800)"
                .to_string(),
        ),
        None => Err(
            "dispatch-evidence guard: no executor output received — \
             refusing to accept completion (ADR-2026-04-11-1800)"
                .to_string(),
        ),
    }
}

/// treated as a failed run — the whole point of the guard (ADR-2026-04-14-1400 §1
/// P1). If HEAD is unreadable on either side, the guard errs on the side of
/// marking the run a failure; silent drains are the bug we're killing.
fn check_evidence(
    exit_ok: bool,
    pre: Option<&str>,
    post: Option<&str>,
) -> (bool, String) {
    let has_evidence = matches!((pre, post), (Some(a), Some(b)) if a != b);
    let pre_kv = pre.unwrap_or("UNREADABLE");
    let post_kv = post.unwrap_or("UNREADABLE");
    let verdict = match (pre, post) {
        (Some(a), Some(b)) if has_evidence => {
            format!("HEAD {a} → {b}")
        }
        (Some(a), Some(b)) => {
            format!("no git evidence: HEAD unchanged ({a} → {b})")
        }
        _ => "no git evidence: HEAD unreadable".to_string(),
    };
    let snippet = format!(
        "\n--- guard ---\npre_head={pre_kv}\npost_head={post_kv}\n{verdict}"
    );
    (exit_ok && has_evidence, snippet)
}

/// Execute memory-health check: query memory for stale tasks, categorize, archive.
async fn execute_memory_health_check() -> anyhow::Result<String> {
    use crate::nexus_client::NexusClient;

    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Query memory for all brain-tasks
    let body: serde_json::Value = nexus
        .get("/api/hexflo/memory/search?q=brain-task:")
        .await?;

    let results = body
        .get("results")
        .and_then(|r| r.as_array())
        .context("No results in memory query")?;

    let mut stale_count = 0;
    let mut archived_count = 0;
    let mut failed_count = 0;
    let mut total_count = 0;

    let now = chrono::Utc::now();

    for entry in results {
        let value_str = entry.get("value").and_then(|v| v.as_str()).unwrap_or("");
        let task: serde_json::Value = match serde_json::from_str(value_str) {
            Ok(t) => t,
            Err(_) => continue,
        };

        total_count += 1;

        let status = task.get("status").and_then(|s| s.as_str()).unwrap_or("");
        let completed_at = task.get("completed_at").and_then(|c| c.as_str());

        // Categorize failed tasks
        if status == "failed" {
            failed_count += 1;

            // Check if old enough to archive (>7 days)
            if let Some(completed_str) = completed_at {
                if let Ok(completed_dt) = chrono::DateTime::parse_from_rfc3339(completed_str) {
                    let age = now.signed_duration_since(completed_dt.with_timezone(&chrono::Utc));
                    if age.num_days() > 7 {
                        // Archive old failed tasks
                        // For now just count, full archive logic would call nexus API
                        archived_count += 1;
                        stale_count += 1;
                    }
                }
            }
        }

        // Detect stale in_progress tasks (leased_until > 2h ago)
        if status == "in_progress" {
            if let Some(leased_str) = task.get("leased_until").and_then(|l| l.as_str()) {
                if let Ok(leased_dt) = chrono::DateTime::parse_from_rfc3339(leased_str) {
                    let elapsed = now.signed_duration_since(leased_dt.with_timezone(&chrono::Utc));
                    if elapsed.num_hours() > 2 {
                        stale_count += 1;
                    }
                }
            }
        }
    }

    let summary = format!(
        "Memory Health Check\n\
         Total entries: {}\n\
         Stale tasks: {}\n\
         Failed tasks: {}\n\
         Archived (old failures): {}\n\
         \n\
         Status: {} stale tasks detected\n",
        total_count,
        stale_count,
        failed_count,
        archived_count,
        if stale_count > 0 { "NEEDS ATTENTION" } else { "HEALTHY" }
    );

    Ok(summary)
}

/// P6: Validate workplan task evidence after execution.
/// Reads workplan JSON, finds completed tasks, runs their evidence commands.
/// Returns validation summary for logging.
async fn validate_workplan_evidence(workplan_path: &str) -> anyhow::Result<String> {
    let content = tokio::fs::read_to_string(workplan_path).await
        .context("Failed to read workplan JSON")?;

    let workplan: serde_json::Value = serde_json::from_str(&content)
        .context("Failed to parse workplan JSON")?;

    let mut validation_log = Vec::new();
    let mut failed_count = 0;
    let mut passed_count = 0;
    let mut stub_count = 0;

    // Find all tasks marked "done" and validate them
    if let Some(phases) = workplan.get("phases").and_then(|p| p.as_array()) {
        for phase in phases {
            if let Some(tasks) = phase.get("tasks").and_then(|t| t.as_array()) {
                for task in tasks {
                    let status = task.get("status").and_then(|s| s.as_str()).unwrap_or("");
                    if status != "done" {
                        continue;
                    }

                    let task_id = task.get("id").and_then(|i| i.as_str()).unwrap_or("unknown");
                    let files = task.get("files").and_then(|f| f.as_array());
                    let evidence = task.get("evidence").and_then(|e| e.as_array());

                    // Check for TODO stubs in files
                    if let Some(files_arr) = files {
                        for file in files_arr {
                            if let Some(file_path) = file.as_str() {
                                if let Ok(content) = std::fs::read_to_string(file_path) {
                                    if content.contains("TODO: implement") || content.contains("// TODO") {
                                        validation_log.push(format!("Task {} STUB: {} contains TODO", task_id, file_path));
                                        stub_count += 1;
                                    }
                                }
                            }
                        }
                    }

                    // Run evidence commands
                    if let Some(evidence_arr) = evidence {
                        for ev in evidence_arr {
                            if let Some(cmd) = ev.as_str() {
                                let output = std::process::Command::new("sh")
                                    .arg("-c")
                                    .arg(cmd)
                                    .output();

                                match output {
                                    Ok(out) if out.status.success() => {
                                        passed_count += 1;
                                    }
                                    Ok(out) => {
                                        let stderr = String::from_utf8_lossy(&out.stderr);
                                        validation_log.push(format!("Task {} FAIL: {} ({})", task_id, cmd, stderr.trim()));
                                        failed_count += 1;
                                    }
                                    Err(e) => {
                                        validation_log.push(format!("Task {} ERROR: {} ({})", task_id, cmd, e));
                                        failed_count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut summary = String::new();
    if stub_count > 0 || failed_count > 0 {
        summary.push_str("VALIDATION FAILED\n");
        summary.push_str(&format!("Stubs: {}, Failed: {}, Passed: {}\n", stub_count, failed_count, passed_count));
        for log in &validation_log {
            summary.push_str(&format!("{}\n", log));
        }
    } else {
        summary.push_str(&format!("VALIDATION PASSED ({})", passed_count));
    }

    Ok(summary)
}

pub(crate) async fn execute_brain_task(kind: &str, payload: &str) -> (bool, String) {
    debug!(kind = %kind, payload_len = payload.len(), "drain-path: execute-start");
    // ADR-2026-04-14-1400 §1 P1: capture pre-HEAD only for workplan tasks; the
    // other kinds stay exit-code-only in this slice.
    let pre_head = if kind == "workplan" {
        git_head_sha()
    } else {
        None
    };
    // ADR-2605190900 P5 — liveness ping. Synthetic task the doctor
    // liveness probe enqueues to walk the full dispatch chain. Handled
    // INLINE rather than shelling out: emit a `pong` improver_event row
    // with scope=<uuid> so the probe's SQL poll sees it. No agent claim,
    // no inference call, no side effects beyond the event row.
    if kind == "hex-command" && payload.starts_with("ping ") {
        let uuid = payload[5..].trim();
        let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
        let database = std::env::var("HEX_STDB_DATABASE").unwrap_or_else(|_| "hex".to_string());
        let url = format!("{stdb_host}/v1/database/{database}/call/improver_event_record");
        let timestamp = chrono::Utc::now().to_rfc3339();
        let http = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => return (false, format!("ping handler http build: {e}")),
        };
        match http
            .post(&url)
            .json(&serde_json::json!(["pong", "Liveness", uuid, "{}", 0u64, timestamp]))
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => {
                return (true, format!("pong emitted for {uuid}"));
            }
            Ok(res) => {
                let status = res.status();
                let body = res.text().await.unwrap_or_else(|_| "<no body>".to_string());
                return (false, format!("improver_event_record HTTP {status}: {body}"));
            }
            Err(e) => return (false, format!("improver_event_record: {e}")),
        }
    }
    let output = match kind {
        "hex-command" => std::process::Command::new("hex")
            .args(payload.split_whitespace())
            .output(),
        "analyze" => std::process::Command::new("hex")
            .args(["analyze", ".", "--json"])
            .output(),
        "workplan" => {
            // Path C (ADR-2026-04-29-1354): when no active Claude session, spawn
            // autonomous hex-agent instead of dispatching to nexus
            if std::env::var("CLAUDE_SESSION_ID").is_err() {
                eprintln!("⬡ spawned autonomous agent for workplan {}", payload);
                std::process::Command::new("hex-agent")
                    .args(["workplan", payload, "--background"])
                    .output()
            } else {
                // Path B: dispatch to active Claude session via nexus
                std::process::Command::new("hex")
                    .args(["plan", "execute", payload])
                    .output()
            }
        }
        "shell" => {
            // Whitelist for shell-kind tasks. `hex` is allowed so the daemon
            // can dispatch hex's own subcommands (plan draft, doctor, …) —
            // those flow back through the same trusted CLI surface that
            // produced the task. Anything broader belongs in a workplan.
            let mut parts = payload.split_whitespace();
            let cmd = match parts.next() {
                Some(c) => c,
                None => return (false, "empty shell command".to_string()),
            };
            const ALLOWED: &[&str] = &["cargo", "git", "ls", "echo", "ssh", "hex"];
            if !ALLOWED.contains(&cmd) {
                return (
                    false,
                    format!(
                        "shell command '{}' not in whitelist (allowed: {:?})",
                        cmd, ALLOWED
                    ),
                );
            }
            std::process::Command::new(cmd).args(parts).output()
        }
        "memory-health" => {
            // Execute memory health check: query memory, categorize, archive stale tasks
            match execute_memory_health_check().await {
                Ok(summary) => Ok(std::process::Output {
                    status: std::process::ExitStatus::default(),
                    stdout: summary.into_bytes(),
                    stderr: Vec::new(),
                }),
                Err(e) => Ok(std::process::Output {
                    status: std::process::ExitStatus::default(),
                    stdout: Vec::new(),
                    stderr: format!("memory-health error: {}", e).into_bytes(),
                }),
            }
        }
        "research-sweep" => {
            // Execute idle research sweep: run analysts, generate findings
            // TODO: implement research-sweep executor
            return (false, "research-sweep executor not yet implemented".to_string())
        }
        other => {
            return (
                false,
                format!(
                    "unknown task kind '{}' (expected: hex-command, workplan, shell, memory-health, research-sweep)",
                    other
                ),
            )
        }
    };

    match output {
        Ok(out) => {
            let mut combined = String::new();
            combined.push_str(&String::from_utf8_lossy(&out.stdout));
            if !out.stderr.is_empty() {
                combined.push_str("\n--- stderr ---\n");
                combined.push_str(&String::from_utf8_lossy(&out.stderr));
            }
            // Keep the TAIL of output (not head) — errors land at the end of
            // stdout/stderr; truncating from the front hides them. 8000 chars
            // is enough for full error context without bloating the STDB row.
            let count = combined.chars().count();
            let skip = count.saturating_sub(8000);
            let mut snippet: String = combined.chars().skip(skip).collect();
            // ADR-2026-04-14-1400 §1 P1: for workplan tasks, require that HEAD
            // actually moved. `hex plan execute` exits 0 in multiple no-op
            // paths (tasks already done, inference unavailable, empty
            // dispatch) — exit code alone produces silent drains.
            if kind == "workplan" {
                let post_head = git_head_sha();
                let (guarded_success, guard_snippet) = check_evidence(
                    out.status.success(),
                    pre_head.as_deref(),
                    post_head.as_deref(),
                );
                snippet.push_str(&guard_snippet);

                // P6: Validation judge - verify workplan task evidence after execution
                if guarded_success {
                    match validate_workplan_evidence(payload).await {
                        Ok(validation_result) => {
                            snippet.push_str(&format!("\n--- validation ---\n{}", validation_result));
                            // If validation failed, override success even if git evidence exists
                            let final_success = !validation_result.contains("VALIDATION FAILED");
                            (final_success, snippet)
                        }
                        Err(e) => {
                            snippet.push_str(&format!("\n--- validation ---\nValidation error: {}", e));
                            (guarded_success, snippet)
                        }
                    }
                } else {
                    (guarded_success, snippet)
                }
            } else {
                (out.status.success(), snippet)
            }
        }
        Err(e) => (false, format!("spawn error: {}", e)),
    }
}

/// Parse a human-friendly duration string into [`Duration`].
/// Supports: `30s`, `5m`, `2h`, `7d`. Bare numbers are treated as seconds.
fn parse_duration_str(s: &str) -> anyhow::Result<Duration> {
    let s = s.trim();
    if s.is_empty() {
        anyhow::bail!("empty duration string");
    }
    let (num_part, unit) = if s.ends_with('d') {
        (&s[..s.len() - 1], 'd')
    } else if s.ends_with('h') {
        (&s[..s.len() - 1], 'h')
    } else if s.ends_with('m') {
        (&s[..s.len() - 1], 'm')
    } else if s.ends_with('s') {
        (&s[..s.len() - 1], 's')
    } else {
        (s, 's')
    };
    let n: u64 = num_part
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration: {}", s))?;
    let secs = match unit {
        'd' => n * 86400,
        'h' => n * 3600,
        'm' => n * 60,
        _ => n,
    };
    Ok(Duration::from_secs(secs))
}

async fn queue_list(include: &str, since: Option<&str>) -> anyhow::Result<()> {
    let statuses: Vec<&str> = include.split(',').map(|s| s.trim()).collect();
    let show_all = statuses.iter().any(|s| *s == "all");

    let cutoff = match since {
        Some(dur_str) => {
            let dur = parse_duration_str(dur_str)?;
            Some(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs() - dur.as_secs())
        }
        None => None,
    };

    let status_filter: Option<&str> = if show_all || statuses.len() > 1 {
        None
    } else {
        Some(statuses[0])
    };

    let all_tasks = list_brain_tasks(status_filter).await?;

    let tasks: Vec<_> = all_tasks
        .into_iter()
        .filter(|t| {
            if !show_all && statuses.len() > 1 {
                let st = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if !statuses.contains(&st) {
                    return false;
                }
            }
            if let Some(cutoff_epoch) = cutoff {
                if let Some(created) = t.get("created_at").and_then(|v| v.as_str()) {
                    if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(created) {
                        return ts.timestamp() as u64 >= cutoff_epoch;
                    }
                    if let Ok(epoch) = created.parse::<u64>() {
                        return epoch >= cutoff_epoch;
                    }
                }
            }
            true
        })
        .collect();

    if tasks.is_empty() {
        let scope = if show_all {
            "all".to_string()
        } else {
            include.to_string()
        };
        let since_label = since.map(|s| format!(" (since {})", s)).unwrap_or_default();
        println!(
            "{}",
            format!("No sched tasks matching status={}{}", scope, since_label).yellow()
        );
        return Ok(());
    }

    let heading = if show_all {
        "Sched Tasks (all)".to_string()
    } else {
        format!("Sched Tasks ({})", include)
    };
    println!("{}", heading.green().bold());

    let show_status_col = show_all || statuses.len() > 1;
    let rows: Vec<Vec<String>> = tasks
        .iter()
        .map(|t| {
            let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let raw_payload = t.get("payload").and_then(|v| v.as_str()).unwrap_or("");
            let mut row = vec![
                t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                kind.to_string(),
            ];
            if show_status_col {
                row.push(
                    t.get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                );
            }
            row.push(render_task_target(kind, raw_payload));
            row.push(truncate(raw_payload, 40));
            row.push(
                t.get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            );
            row
        })
        .collect();

    let headers: Vec<&str> = if show_status_col {
        vec!["ID", "Kind", "Status", "Target", "Payload", "Created"]
    } else {
        vec!["ID", "Kind", "Target", "Payload", "Created"]
    };
    println!("{}", pretty_table(&headers, &rows));
    Ok(())
}

/// Render the recent brain-task history table (wp-sched-queue-history P1.3).
///
/// Hits `GET /api/sched/queue/history` and formats each row with a 60-char
/// tail of the result string so the `no git evidence` marker (ADR-2026-04-14-1400
/// §1 P1 evidence-guard) is visible without horizontal scrolling. Using the
/// tail rather than the head is deliberate — the guard appends the marker,
/// so a head-truncation would hide it.
async fn queue_history(status: Option<String>, limit: u32) -> anyhow::Result<()> {
    use crate::nexus_client::NexusClient;
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Build query string; skip `status=` entirely when unset so nexus treats
    // it as "all statuses" rather than filtering on an empty-string match.
    let mut path = format!("/api/sched/queue/history?limit={}", limit);
    if let Some(s) = status.as_deref() {
        path.push_str(&format!("&status={}", s));
    }
    let body: serde_json::Value = nexus.get(&path).await?;
    let rows: Vec<serde_json::Value> = body.as_array().cloned().unwrap_or_default();

    if rows.is_empty() {
        let scope = status
            .as_deref()
            .map(|s| format!(" (status={})", s))
            .unwrap_or_default();
        println!("{}", format!("No sched tasks in history{}.", scope).yellow());
        return Ok(());
    }

    let heading = match status.as_deref() {
        Some(s) => format!("Sched Task History — status={}", s),
        None => "Sched Task History".to_string(),
    };
    println!("{}", heading.green().bold());

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let kind = r.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let status = r.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let payload = r
                .get("payload_truncated")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let result = r
                .get("result_truncated")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // ID column is first 8 chars — enough to disambiguate in practice,
            // keeps the table narrow. Operators who need the full UUID can
            // re-query via /api/hexflo/memory/brain-task:<id>.
            let short_id = if id.len() >= 8 { &id[..8] } else { id };
            vec![
                short_id.to_string(),
                kind.to_string(),
                status.to_string(),
                truncate(payload, 40),
                result_tail(result, 60),
            ]
        })
        .collect();
    println!(
        "{}",
        pretty_table(
            &["ID", "Kind", "Status", "Payload", "Result-Tail"],
            &table_rows,
        )
    );
    Ok(())
}

/// Return the last `n` chars of `s`. Used by `queue_history` so the trailing
/// evidence-guard marker (`no git evidence...`) stays visible in the table.
/// For short strings (<= n), returns the whole string verbatim.
fn result_tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_string();
    }
    s.chars().skip(count - n).collect()
}

/// Extract the user-visible target for a sched task row. For
/// `remote-shell`, that's the destination host parsed out of the JSON
/// payload. For any other kind, we render `-` so the Target column stays
/// aligned without leaking implementation details of the payload shape.
fn render_task_target(kind: &str, payload: &str) -> String {
    if kind == "remote-shell" {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
            if let Some(host) = v.get("host").and_then(|h| h.as_str()) {
                return host.to_string();
            }
        }
        // Malformed remote-shell payloads shouldn't abort the listing — show
        // a sentinel so the row is still readable and the problem is visible.
        return "?".to_string();
    }
    "-".to_string()
}

async fn queue_clear() -> anyhow::Result<()> {
    let all = list_brain_tasks(None).await?;
    let client = reqwest::Client::new();
    let mut cleared = 0usize;
    for task in all {
        let status = task.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if status == "completed" || status == "failed" {
            if let Some(id) = task.get("id").and_then(|v| v.as_str()) {
                let key = format!("brain-task:{}", id);
                let _ = client
                    .delete(format!("{}/api/hexflo/memory/{}", NEXUS_BASE, key))
                    .send()
                    .await;
                cleared += 1;
            }
        }
    }
    println!("⬡ cleared {cleared} completed/failed sched tasks");
    Ok(())
}

async fn queue_drain() -> anyhow::Result<()> {
    let pending = list_brain_tasks(Some("pending")).await?;
    // Snapshot in_flight alongside pending so the idle-tick gate counts real
    // inactivity (no queued work AND no tasks actively running). A failure to
    // read in_progress is treated as "unknown" → non-idle, so we don't
    // spuriously declare the queue quiet when we can't see it.
    let in_flight = list_brain_tasks(Some("in_progress")).await.unwrap_or_default();
    let is_idle = pending.is_empty() && in_flight.is_empty();

    // ── Sweep preemption (wp-idle-research-swarm P4.4) ──────────────────
    // If a research sweep is currently in-flight (the coordinator's
    // marker file is present) AND a non-research task has landed in
    // pending, signal-abort the sweep so the higher-priority work runs
    // first. We also clear the throttle so the next idle window re-fires
    // the sweep — otherwise the 6h gate would block a re-enqueue and the
    // half-finished research never gets resumed.
    let pending_kinds: Vec<&str> = pending
        .iter()
        .map(|t| t.get("kind").and_then(|v| v.as_str()).unwrap_or(""))
        .collect();
    if should_preempt_sweep(&pending_kinds, is_sweep_in_flight()) {
        let non_research_count = pending_kinds.iter().filter(|k| **k != "research-sweep").count();
        request_sweep_abort();
        clear_last_research_sweep();
        println!(
            "  {} preempting in-flight research-sweep for {} non-research task(s); will re-enqueue at next idle window",
            "⚠".yellow(),
            non_research_count,
        );
    }

    let mut state = load_daemon_state();
    let threshold = load_idle_threshold_ticks();
    if is_idle {
        state.idle_tick_count = state.idle_tick_count.saturating_add(1);
    } else {
        state.idle_tick_count = 0;
    }
    let idle_ticks = state.idle_tick_count;
    save_daemon_state(&state);

    // ── Idle-research trigger (wp-idle-research-swarm P1.2 / ADR-2026-04-15-1200) ──
    // When the queue has been idle for `threshold` ticks AND no sweep has
    // run for `min_sweep_interval_h` hours, self-enqueue a research-sweep.
    // The actual analyst dispatch lands in P4.1 (hex-nexus); for now we
    // just put the work item on the queue with the trigger metadata so a
    // downstream worker (or `hex sched queue history`) can pick it up.
    if is_idle && idle_ticks >= threshold {
        let interval_h = load_min_sweep_interval_h();
        let now = chrono::Utc::now();
        let last = read_last_research_sweep();
        if should_self_enqueue_research_sweep(idle_ticks, threshold, last, now, interval_h) {
            let payload = json!({
                "trigger": "idle",
                "idle_ticks": idle_ticks,
                "threshold": threshold,
                "min_sweep_interval_h": interval_h,
                "enqueued_at": now.to_rfc3339(),
            })
            .to_string();
            match enqueue_brain_task("research-sweep", &payload).await {
                Ok(id) => {
                    println!(
                        "  ⬡ enqueued idle research-sweep {} (idle {}/{} ticks, last sweep {})",
                        id,
                        idle_ticks,
                        threshold,
                        last.map(|t| t.to_rfc3339()).unwrap_or_else(|| "never".to_string()),
                    );
                    write_last_research_sweep(now);
                }
                Err(e) => {
                    eprintln!("  {} enqueue research-sweep: {}", "✗".red(), e);
                }
            }
        }
    }

    // ── Memory-health trigger (wp-memory-health-swarm P3.1 / ADR-2026-04-29-1320) ──
    // Run memory-health check every hour (not idle-gated like research-sweep).
    // This ensures stale task cleanup happens regularly even when queue is busy.
    {
        let now = chrono::Utc::now();
        let last = read_last_memory_health_check();
        let interval_h = 1; // TODO: load from config

        if should_enqueue_memory_health(last, now, interval_h) {
            let payload = json!({
                "trigger": "scheduled",
                "interval_h": interval_h,
                "enqueued_at": now.to_rfc3339(),
            })
            .to_string();

            match enqueue_brain_task("memory-health", &payload).await {
                Ok(id) => {
                    println!(
                        "  ⬡ enqueued memory-health check {} (last check {})",
                        id,
                        last.map(|t| t.to_rfc3339()).unwrap_or_else(|| "never".to_string()),
                    );
                    write_last_memory_health_check(now);
                }
                Err(e) => {
                    eprintln!("  {} enqueue memory-health: {}", "✗".red(), e);
                }
            }
        }
    }

    if pending.is_empty() {
        if idle_ticks >= threshold {
            println!(
                "{} (idle {}/{} ticks)",
                "No pending sched tasks to drain.".yellow(),
                idle_ticks,
                threshold,
            );
        } else {
            println!("{}", "No pending sched tasks to drain.".yellow());
        }
        return Ok(());
    }
    println!("⬡ draining {} pending sched task(s)...", pending.len());
    for task in pending {
        let id = task
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let kind = task
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let payload = task
            .get("payload")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        println!("  → executing {id} ({kind})");
        let _ = update_brain_task(&id, BrainTaskStatus::InProgress, "").await;
        let (mut ok, mut result) = execute_brain_task(&kind, &payload).await;
        // ADR-2026-04-14-2155 P2.3: reject vacuous executor output
        if ok {
            if let Err(reason) = validate_dispatch_evidence(Some(&result)) {
                ok = false;
                result.push_str(&format!("\n--- dispatch-evidence guard ---\n{reason}"));
            }
        }
        let status = if ok {
            BrainTaskStatus::Completed
        } else {
            BrainTaskStatus::Failed
        };
        update_brain_task(&id, status, &result).await?;
        println!(
            "    {} {}",
            if ok { "✓".green() } else { "✗".red() },
            status.as_str()
        );
    }
    Ok(())
}

// ─── ADR-doctor tick orchestrator (ADR-2026-04-27-0800 §1a, P3.2) ────────────
//
// Pure data-shape unit tests for `tick_adr_health_actions`. Lives in a
// module named `tick_adr_health` directly under `sched` so the workplan
// gate (`cargo test -p hex-cli sched::tick_adr_health`) substring-matches
// the full test path `commands::sched::tick_adr_health::*` and runs
// exactly this set.
//
// Filesystem + git side effects of shadow-promote are covered by the
// integration test `tests/sched_adr_health_tick.rs`; here we lock in the
// pure routing rules by passing `cfg=None` (every Tier-A/B finding then
// routes through the cfg-unavailable abort path; every Tier-C still
// notifies).
#[cfg(test)]
mod tick_adr_health {
    use super::tick_adr_health_actions;
    use crate::commands::adr::doctor::{finding, AutoFixTier, FindingKind};
    use std::path::PathBuf;

    #[tokio::test]
    async fn emits_event_with_finding_counts() {
        let findings = vec![
            finding("ADR-1", PathBuf::from("a.md"), FindingKind::UnparseableStatus, "buggy"),
            finding("ADR-2", PathBuf::from("b.md"), FindingKind::DuplicateId, "dup"),
            finding("ADR-3", PathBuf::from("c.md"), FindingKind::MissingRequiredField, "miss"),
            finding("ADR-4", PathBuf::from("d.md"), FindingKind::StaleProposed, "old"),
        ];
        // Sanity: the rule table puts these in the tiers we expect.
        assert_eq!(findings[0].tier, AutoFixTier::A);
        assert_eq!(findings[1].tier, AutoFixTier::C);
        assert_eq!(findings[2].tier, AutoFixTier::C);
        assert_eq!(findings[3].tier, AutoFixTier::B);

        // cfg=None forces the "config unavailable" path for Tier-A/B (we
        // don't want a real worktree/git here). Tier-C still notifies.
        let result = tick_adr_health_actions(&findings, None).await;

        assert_eq!(result.event.event_type, "adr_doctor_tick");
        assert_eq!(result.event.payload["total"], 4);
        assert_eq!(result.event.payload["tier_a"], 1);
        assert_eq!(result.event.payload["tier_b"], 1);
        assert_eq!(result.event.payload["tier_c"], 2);
    }

    #[tokio::test]
    async fn routes_tier_c_findings_to_p1_inbox_notifications() {
        let f = finding(
            "ADR-2026-04-27-0800",
            PathBuf::from("docs/adrs/ADR-2026-04-27-0800-x.md"),
            FindingKind::DuplicateId,
            "duplicate id detected",
        );
        assert_eq!(f.tier, AutoFixTier::C);

        let result = tick_adr_health_actions(&[f], None).await;

        assert_eq!(result.notifications.len(), 1);
        let n = &result.notifications[0];
        assert_eq!(n.kind, "adr.doctor.notify");
        assert_eq!(n.priority, 1, "Tier-C must be priority=1 (operator interrupt)");
        assert_eq!(n.body["tier"], "C");
        assert_eq!(n.body["adr_id"], "ADR-2026-04-27-0800");
        assert_eq!(n.body["kind"], "DuplicateId");
    }

    #[tokio::test]
    async fn aborts_tier_a_when_config_unavailable_with_p2_notification() {
        let f = finding(
            "ADR-2026-04-27-0800",
            PathBuf::from("docs/adrs/ADR-2026-04-27-0800-x.md"),
            FindingKind::UnparseableStatus,
            "buggy frontmatter",
        );
        assert_eq!(f.tier, AutoFixTier::A);

        let result = tick_adr_health_actions(&[f], None).await;

        assert_eq!(result.notifications.len(), 1);
        let n = &result.notifications[0];
        assert_eq!(n.kind, "adr.doctor.aborted");
        assert_eq!(n.priority, 2, "Aborted Tier-A must downgrade to priority=2");
        assert_eq!(n.body["tier"], "A");
        assert!(
            n.body["reason"].as_str().unwrap().contains("config unavailable"),
            "reason should cite missing cfg, got: {}",
            n.body["reason"],
        );
    }

    #[tokio::test]
    async fn emits_event_even_when_findings_empty() {
        let result = tick_adr_health_actions(&[], None).await;

        // Empty findings is the daemon's clean-registry case: still emit
        // the event so trend graphs see a clean tick, no notifications.
        assert_eq!(result.event.event_type, "adr_doctor_tick");
        assert_eq!(result.event.payload["total"], 0);
        assert!(result.notifications.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ADR-2026-04-14-1400 §1 P1: workplan-evidence guard ─────────────────────
    // These tests lock in the semantics of `check_evidence`: a workplan task
    // whose subprocess exits 0 but produces no new commit must return
    // success=false with a snippet that names the drift ("no git evidence").
    // If these fail, silent-drains are back and autonomy is broken again.

    #[test]
    fn test_workplan_no_evidence() {
        // exit 0, HEAD unchanged → should be treated as FAILURE
        let (success, snippet) =
            check_evidence(true, Some("abc1234"), Some("abc1234"));
        assert!(
            !success,
            "workplan with no HEAD movement must not be marked success"
        );
        assert!(
            snippet.contains("no git evidence"),
            "snippet must name the drift; got: {snippet}"
        );
        assert!(
            snippet.contains("pre_head=abc1234"),
            "snippet must include structured pre_head; got: {snippet}"
        );
        assert!(
            snippet.contains("post_head=abc1234"),
            "snippet must include structured post_head; got: {snippet}"
        );
    }

    #[test]
    fn test_workplan_with_evidence() {
        // exit 0, HEAD moved → should be SUCCESS; snippet records the delta
        let (success, snippet) =
            check_evidence(true, Some("abc1234"), Some("def5678"));
        assert!(success, "workplan with HEAD movement should succeed");
        assert!(
            snippet.contains("HEAD abc1234 → def5678"),
            "snippet must record pre/post SHAs; got: {snippet}"
        );
        assert!(
            snippet.contains("pre_head=abc1234"),
            "structured pre_head for reconcile audit; got: {snippet}"
        );
        assert!(
            snippet.contains("post_head=def5678"),
            "structured post_head for reconcile audit; got: {snippet}"
        );
    }

    #[test]
    fn test_workplan_evidence_exit_failure_never_succeeds() {
        // exit != 0 overrides evidence: a failing process is a failure even
        // if HEAD happens to have moved (e.g. partial work then crash).
        let (success, snippet) =
            check_evidence(false, Some("abc1234"), Some("def5678"));
        assert!(
            !success,
            "non-zero exit must not be overridden by HEAD movement"
        );
        assert!(
            snippet.contains("pre_head=abc1234") && snippet.contains("post_head=def5678"),
            "structured SHAs present even on exit failure; got: {snippet}"
        );
    }

    #[test]
    fn test_workplan_evidence_unreadable_head_fails() {
        // HEAD unreadable → guard treats as failure; silent drain is worse
        // than a visible failure when git itself is broken.
        let (success, snippet) = check_evidence(true, None, None);
        assert!(!success);
        assert!(
            snippet.contains("no git evidence"),
            "snippet must surface the HEAD read failure; got: {snippet}"
        );
        assert!(
            snippet.contains("pre_head=UNREADABLE"),
            "unreadable HEAD must be explicit; got: {snippet}"
        );
        assert!(
            snippet.contains("post_head=UNREADABLE"),
            "unreadable HEAD must be explicit; got: {snippet}"
        );
    }

    #[test]
    fn render_task_target_extracts_host_for_remote_shell() {
        let payload = r#"{"host":"bazzite","command":"nvidia-smi"}"#;
        assert_eq!(render_task_target("remote-shell", payload), "bazzite");
    }

    // ── wp-sched-queue-history P1.3: result-tail rendering ────────────────
    // The history table displays the LAST N chars of `result_truncated` so
    // the evidence-guard's `no git evidence` marker stays visible. A
    // head-truncation would hide the very signal operators are looking for.

    #[test]
    fn result_tail_returns_whole_string_when_short() {
        assert_eq!(result_tail("short", 60), "short");
        assert_eq!(result_tail("", 60), "");
    }

    #[test]
    fn result_tail_keeps_trailing_chars_for_long_strings() {
        let s = format!("{}{}", "x".repeat(200), "no git evidence (HEAD unchanged)");
        let out = result_tail(&s, 60);
        assert_eq!(out.chars().count(), 60);
        assert!(
            out.contains("no git evidence"),
            "evidence-guard marker must survive tail truncation; got: {out}"
        );
    }

    #[test]
    fn render_task_target_returns_dash_for_non_remote_kinds() {
        assert_eq!(render_task_target("hex-command", "hex analyze ."), "-");
        assert_eq!(render_task_target("workplan", "docs/workplans/x.json"), "-");
        assert_eq!(render_task_target("shell", "df -h"), "-");
    }

    #[test]
    fn render_task_target_returns_sentinel_for_malformed_remote_shell() {
        // A remote-shell row without a parseable host should still render —
        // the queue list must stay readable even when a record is malformed.
        assert_eq!(render_task_target("remote-shell", "not json"), "?");
        assert_eq!(render_task_target("remote-shell", "{}"), "?");
    }

    #[test]
    fn parse_since_accepts_rfc3339_utc() {
        let got = parse_since("2026-04-14T10:00:00Z").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&got).unwrap();
        assert_eq!(parsed.with_timezone(&chrono::Utc).to_rfc3339(), got);
    }

    #[test]
    fn parse_since_accepts_rfc3339_with_offset() {
        let got = parse_since("2026-04-14T12:00:00+02:00").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&got).unwrap();
        // Normalized to UTC — 12:00+02:00 == 10:00Z.
        assert_eq!(parsed.with_timezone(&chrono::Utc).to_rfc3339(), got);
        assert!(got.contains("10:00:00"));
    }

    #[test]
    fn parse_since_accepts_humantime_durations() {
        let before = chrono::Utc::now();
        let got = parse_since("1h").unwrap();
        let after = chrono::Utc::now();

        let parsed = chrono::DateTime::parse_from_rfc3339(&got)
            .unwrap()
            .with_timezone(&chrono::Utc);
        // Result must be approximately (now - 1h), bracketed by before/after.
        let lo = before - chrono::Duration::hours(1) - chrono::Duration::seconds(1);
        let hi = after - chrono::Duration::hours(1) + chrono::Duration::seconds(1);
        assert!(
            parsed >= lo && parsed <= hi,
            "parsed {parsed} outside [{lo}, {hi}]"
        );
    }

    #[test]
    fn parse_since_accepts_compound_durations() {
        // humantime supports compound like "2h15m"; make sure we don't break it.
        let got = parse_since("2h15m").unwrap();
        chrono::DateTime::parse_from_rfc3339(&got).unwrap();
    }

    #[test]
    fn parse_since_rejects_garbage() {
        let err = parse_since("not-a-time").unwrap_err().to_string();
        assert!(err.contains("--since"), "unhelpful error: {err}");
    }

    #[test]
    fn parse_since_rejects_empty() {
        assert!(parse_since("").is_err());
        assert!(parse_since("   ").is_err());
    }

    // ─── BrainNotifyConfig (wp-brain-updates P3.2) ─────────────────────────

    #[test]
    fn notify_config_default_sends_everything() {
        let cfg = BrainNotifyConfig::default();
        assert!(cfg.should_notify("brain.task.completed", 1));
        assert!(cfg.should_notify("brain.task.failed", 2));
        assert!(cfg.should_notify("brain.validate.regression", 2));
        assert!(cfg.should_notify("brain.workplan.complete", 1));
        assert!(cfg.should_notify("brain.grade.drop", 2));
    }

    #[test]
    fn notify_config_unknown_kind_passes_through() {
        // New notification kinds must not be silently dropped just because
        // the schema hasn't been updated — default-allow is the safe path.
        let cfg = BrainNotifyConfig::default();
        assert!(cfg.should_notify("brain.future.event", 1));
        assert!(cfg.should_notify("something.else.entirely", 2));
    }

    #[test]
    fn notify_config_per_kind_toggle() {
        let cfg = BrainNotifyConfig {
            on_task_complete: false,
            on_task_failure: true,
            on_validate_regression: true,
            on_workplan_complete: true,
            on_grade_drop: true,
            min_priority: 1,
        };
        assert!(!cfg.should_notify("brain.task.completed", 1));
        assert!(cfg.should_notify("brain.task.failed", 2));
    }

    #[test]
    fn notify_config_min_priority_floor() {
        let cfg = BrainNotifyConfig {
            min_priority: 2,
            ..BrainNotifyConfig::default()
        };
        // Task-complete fires at priority 1 → suppressed by floor=2.
        assert!(!cfg.should_notify("brain.task.completed", 1));
        // Failure / regression fire at priority 2 → still pass.
        assert!(cfg.should_notify("brain.task.failed", 2));
        assert!(cfg.should_notify("brain.validate.regression", 2));
    }

    #[test]
    fn notify_config_parses_partial_toml() {
        // User flips off only task-complete; every other field must keep its default.
        let src = r#"
[notify]
on_task_complete = false
"#;
        let parsed: DaemonTomlFile = toml::from_str(src).unwrap();
        let cfg = parsed.notify.unwrap();
        assert!(!cfg.on_task_complete);
        assert!(cfg.on_task_failure);
        assert!(cfg.on_validate_regression);
        assert!(cfg.on_workplan_complete);
        assert!(cfg.on_grade_drop);
        assert_eq!(cfg.min_priority, 1);
    }

    #[test]
    fn notify_config_parses_empty_toml() {
        // Missing [notify] section → defaults.
        let parsed: DaemonTomlFile = toml::from_str("").unwrap();
        assert!(parsed.notify.is_none());
    }

    #[test]
    fn notify_config_parses_min_priority() {
        let src = r#"
[notify]
min_priority = 2
"#;
        let parsed: DaemonTomlFile = toml::from_str(src).unwrap();
        let cfg = parsed.notify.unwrap();
        assert_eq!(cfg.min_priority, 2);
    }

    // ─── ADR-2026-04-14-2155 P2.3: validate_dispatch_evidence ────────────────────

    #[test]
    fn dispatch_evidence_accepts_non_empty_output() {
        assert!(validate_dispatch_evidence(Some("compiled OK")).is_ok());
    }

    #[test]
    fn dispatch_evidence_rejects_empty_string() {
        let err = validate_dispatch_evidence(Some("")).unwrap_err();
        assert!(err.contains("dispatch-evidence guard"), "got: {err}");
    }

    #[test]
    fn dispatch_evidence_rejects_whitespace_only() {
        let err = validate_dispatch_evidence(Some("   \n\t  ")).unwrap_err();
        assert!(err.contains("whitespace-only"), "got: {err}");
    }

    #[test]
    fn dispatch_evidence_rejects_none() {
        let err = validate_dispatch_evidence(None).unwrap_err();
        assert!(err.contains("no executor output"), "got: {err}");
    }

    // ─── Brain task schema (ADR-2026-04-14-1400 P0.1) ───────────────────────────
    //
    // Nested under `brain::task_schema` so the workplan gate
    // (`cargo test -p hex-cli brain::task_schema`) runs exactly this set.
    // The `#[allow(non_snake_case)]` isn't needed because Rust allows module
    // names that happen to read as lowercase identifiers.

    mod brain {
        mod task_schema {
            use super::super::super::{BrainTaskEvidence, BrainTaskRecord, BrainTaskStatus};
            use serde_json::json;

            #[test]
            fn status_serializes_as_lowercase_wire_format() {
                // Wire format stays stable across language-typed callers and
                // untyped JS dashboards — a rename here would silently break
                // live queue records.
                assert_eq!(serde_json::to_string(&BrainTaskStatus::Pending).unwrap(), "\"pending\"");
                assert_eq!(serde_json::to_string(&BrainTaskStatus::Leased).unwrap(), "\"leased\"");
                assert_eq!(
                    serde_json::to_string(&BrainTaskStatus::InProgress).unwrap(),
                    "\"in_progress\""
                );
                assert_eq!(
                    serde_json::to_string(&BrainTaskStatus::Completed).unwrap(),
                    "\"completed\""
                );
                assert_eq!(serde_json::to_string(&BrainTaskStatus::Failed).unwrap(), "\"failed\"");
            }

            #[test]
            fn status_roundtrips_through_serde() {
                for s in [
                    BrainTaskStatus::Pending,
                    BrainTaskStatus::Leased,
                    BrainTaskStatus::InProgress,
                    BrainTaskStatus::Completed,
                    BrainTaskStatus::Failed,
                ] {
                    let enc = serde_json::to_string(&s).unwrap();
                    let dec: BrainTaskStatus = serde_json::from_str(&enc).unwrap();
                    assert_eq!(s, dec);
                }
            }

            #[test]
            fn status_as_str_matches_serde_output() {
                // The hand-rolled `as_str` is the source of truth for the
                // daemon's queue filter predicates (e.g. `status_filter ==
                // status.as_str()`) — keep it aligned with serde output.
                for s in [
                    BrainTaskStatus::Pending,
                    BrainTaskStatus::Leased,
                    BrainTaskStatus::InProgress,
                    BrainTaskStatus::Completed,
                    BrainTaskStatus::Failed,
                ] {
                    let via_serde = serde_json::to_value(s).unwrap();
                    assert_eq!(via_serde.as_str().unwrap(), s.as_str());
                }
            }

            #[test]
            fn is_terminal_covers_completed_and_failed() {
                assert!(BrainTaskStatus::Completed.is_terminal());
                assert!(BrainTaskStatus::Failed.is_terminal());
                assert!(!BrainTaskStatus::Pending.is_terminal());
                assert!(!BrainTaskStatus::Leased.is_terminal());
                assert!(!BrainTaskStatus::InProgress.is_terminal());
            }

            #[test]
            fn from_wire_handles_known_variants() {
                assert_eq!(BrainTaskStatus::from_wire("pending"), Some(BrainTaskStatus::Pending));
                assert_eq!(BrainTaskStatus::from_wire("leased"), Some(BrainTaskStatus::Leased));
                assert_eq!(
                    BrainTaskStatus::from_wire("in_progress"),
                    Some(BrainTaskStatus::InProgress)
                );
                assert_eq!(
                    BrainTaskStatus::from_wire("completed"),
                    Some(BrainTaskStatus::Completed)
                );
                assert_eq!(BrainTaskStatus::from_wire("failed"), Some(BrainTaskStatus::Failed));
            }

            #[test]
            fn from_wire_returns_none_for_unknown_variant() {
                // Forward-compatibility: a newer daemon may write a status
                // string we don't recognise yet. We want `None` so the caller
                // can decide (drop the row, mark it as unknown, etc.) rather
                // than crashing the reader.
                assert_eq!(BrainTaskStatus::from_wire("brewing"), None);
                assert_eq!(BrainTaskStatus::from_wire(""), None);
            }

            #[test]
            fn record_deserializes_legacy_shape_without_lease_fields() {
                // Records written before P0 have no lease fields, no
                // evidence, and may omit result/completed_at. Every added
                // field must default cleanly or live tasks become undrainable.
                let v = json!({
                    "id": "abc",
                    "kind": "hex-command",
                    "payload": "analyze .",
                    "status": "pending",
                    "project_id": "p1",
                    "created_at": "2026-04-14T00:00:00Z",
                    "completed_at": null,
                    "result": null
                });
                let rec = BrainTaskRecord::from_value(&v).expect("parse legacy record");
                assert_eq!(rec.id, "abc");
                assert_eq!(rec.kind, "hex-command");
                assert_eq!(rec.payload, "analyze .");
                assert_eq!(rec.status, BrainTaskStatus::Pending);
                assert_eq!(rec.project_id, "p1");
                assert_eq!(rec.leased_to, None);
                assert_eq!(rec.leased_until, None);
                assert_eq!(rec.lease_attempts, 0);
                assert_eq!(rec.swarm_task_id, None);
                assert!(rec.evidence.commits.is_empty());
                assert_eq!(rec.evidence.reconcile_verdict, None);
            }

            #[test]
            fn record_deserializes_new_shape_with_lease_and_evidence() {
                let v = json!({
                    "id": "xyz",
                    "kind": "workplan",
                    "payload": "docs/workplans/wp-foo.json",
                    "status": "leased",
                    "project_id": "p1",
                    "created_at": "2026-04-14T00:00:00Z",
                    "leased_to": "swarm-42",
                    "leased_until": "2026-04-14T00:30:00Z",
                    "lease_attempts": 2,
                    "swarm_task_id": "t-1",
                    "evidence": {
                        "commits": ["abc1234", "def5678"],
                        "reconcile_verdict": "verified"
                    }
                });
                let rec = BrainTaskRecord::from_value(&v).expect("parse new-shape record");
                assert_eq!(rec.status, BrainTaskStatus::Leased);
                assert_eq!(rec.leased_to.as_deref(), Some("swarm-42"));
                assert_eq!(rec.leased_until.as_deref(), Some("2026-04-14T00:30:00Z"));
                assert_eq!(rec.lease_attempts, 2);
                assert_eq!(rec.swarm_task_id.as_deref(), Some("t-1"));
                assert_eq!(rec.evidence.commits, vec!["abc1234".to_string(), "def5678".to_string()]);
                assert_eq!(rec.evidence.reconcile_verdict.as_deref(), Some("verified"));
            }

            #[test]
            fn record_from_value_tolerates_unknown_status_by_defaulting_pending() {
                // Unknown status → pending so the queue stays drainable
                // rather than losing the row to a strict-mode parse error.
                let v = json!({
                    "id": "q",
                    "kind": "shell",
                    "payload": "echo hi",
                    "status": "brewing"
                });
                let rec = BrainTaskRecord::from_value(&v).expect("parse unknown-status record");
                assert_eq!(rec.status, BrainTaskStatus::Pending);
            }

            #[test]
            fn record_from_value_rejects_structurally_incomplete_rows() {
                // Missing the id/kind/payload triple means the row is
                // structurally unusable — no amount of defaulting lets the
                // daemon route it.
                let missing_payload = json!({"id": "x", "kind": "shell"});
                assert!(BrainTaskRecord::from_value(&missing_payload).is_none());
            }

            #[test]
            fn record_roundtrips_preserving_lease_fields() {
                let original = BrainTaskRecord {
                    id: "r1".into(),
                    kind: "workplan".into(),
                    payload: "wp.json".into(),
                    status: BrainTaskStatus::InProgress,
                    project_id: "p".into(),
                    created_at: "2026-04-14T00:00:00Z".into(),
                    completed_at: None,
                    result: None,
                    timeout_s: Some(1800),
                    leased_to: Some("swarm-7".into()),
                    leased_until: Some("2026-04-14T00:30:00Z".into()),
                    lease_attempts: 1,
                    swarm_task_id: Some("st-1".into()),
                    evidence: BrainTaskEvidence {
                        commits: vec!["c1".into()],
                        reconcile_verdict: None,
                    },
                    priority: 0,
                };
                let v = serde_json::to_value(&original).unwrap();
                let round = BrainTaskRecord::from_value(&v).expect("roundtrip");
                assert_eq!(round.status, BrainTaskStatus::InProgress);
                assert_eq!(round.leased_to, original.leased_to);
                assert_eq!(round.leased_until, original.leased_until);
                assert_eq!(round.lease_attempts, 1);
                assert_eq!(round.swarm_task_id, original.swarm_task_id);
                assert_eq!(round.evidence.commits, original.evidence.commits);
            }

            #[test]
            fn evidence_defaults_are_empty() {
                let ev = BrainTaskEvidence::default();
                assert!(ev.commits.is_empty());
                assert!(ev.reconcile_verdict.is_none());
            }
        }

        // ─── Lease durations (ADR-2026-04-14-1400 P1.1) ─────────────────────
        //
        // Nested under `brain::lease_durations` so the workplan gate
        // (`cargo test -p hex-cli brain::lease_durations`) runs exactly
        // this set.

        mod lease_durations {
            use super::super::super::{lease_for, LEASE_DURATIONS};
            use std::time::Duration;

            #[test]
            fn lease_for_known_kinds_matches_table() {
                assert_eq!(lease_for("workplan"), Duration::from_secs(60 * 60));
                assert_eq!(lease_for("hex-command"), Duration::from_secs(15 * 60));
                assert_eq!(lease_for("shell"), Duration::from_secs(10 * 60));
                assert_eq!(lease_for("remote-shell"), Duration::from_secs(5 * 60));
            }

            #[test]
            fn lease_for_unknown_kind_falls_back_to_shell_timeout() {
                // Unknown/typoed kind → 10-minute shell-style window so the
                // sweeper can still reclaim it rather than leaving it
                // leased forever.
                assert_eq!(lease_for("bogus"), Duration::from_secs(10 * 60));
                assert_eq!(lease_for(""), Duration::from_secs(10 * 60));
            }

            #[test]
            fn lease_table_covers_every_known_task_kind() {
                // Locks the kind set: if hex-nexus::TaskKind gains a
                // variant without updating this table, the new kind
                // silently inherits the fallback window. Fail loudly here
                // so the contract stays in sync across crates.
                let kinds: Vec<&str> = LEASE_DURATIONS.iter().map(|(k, _)| *k).collect();
                assert!(kinds.contains(&"workplan"));
                assert!(kinds.contains(&"hex-command"));
                assert!(kinds.contains(&"shell"));
                assert!(kinds.contains(&"remote-shell"));
                assert!(kinds.contains(&"analyze"));
                assert_eq!(kinds.len(), 5, "LEASE_DURATIONS gained a new kind — update this test and confirm the duration is tuned");
            }

            #[test]
            fn lease_table_durations_are_strictly_positive() {
                // A zero-duration lease would be instantly stale, so the
                // sweeper would reclaim every task the moment it's leased
                // — degenerate config. Keep every window > 0.
                for (kind, dur) in LEASE_DURATIONS.iter() {
                    assert!(!dur.is_zero(), "kind {kind} has zero lease window");
                }
            }
        }

        // ─── Workplan timeout_s extraction (wp-sched-daemon-terminal-signal P2.1) ──
        //
        // Verifies that enqueue reads timeout_s from a workplan JSON file
        // and that dispatch honours the stored timeout_s over the default
        // lease_for() window. Runs under
        // `cargo test -p hex-cli brain::workplan_timeout`.

        mod workplan_timeout {
            use super::super::super::lease_for;
            use std::time::Duration;

            #[test]
            fn timeout_s_from_workplan_json() {
                let dir = tempfile::tempdir().unwrap();
                let wp_path = dir.path().join("wp-test.json");
                std::fs::write(
                    &wp_path,
                    r#"{"feature":"test","timeout_s":900,"phases":[]}"#,
                ).unwrap();

                let contents = std::fs::read_to_string(&wp_path).unwrap();
                let wp: serde_json::Value = serde_json::from_str(&contents).unwrap();
                let timeout = wp.get("timeout_s").and_then(|v| v.as_u64());
                assert_eq!(timeout, Some(900));
            }

            #[test]
            fn timeout_s_absent_falls_back_to_lease_default() {
                let dir = tempfile::tempdir().unwrap();
                let wp_path = dir.path().join("wp-no-timeout.json");
                std::fs::write(
                    &wp_path,
                    r#"{"feature":"test","phases":[]}"#,
                ).unwrap();

                let contents = std::fs::read_to_string(&wp_path).unwrap();
                let wp: serde_json::Value = serde_json::from_str(&contents).unwrap();
                let timeout = wp
                    .get("timeout_s")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| lease_for("workplan").as_secs());
                assert_eq!(timeout, 60 * 60);
            }

            #[test]
            fn timeout_s_unreadable_file_falls_back() {
                let timeout: u64 = std::fs::read_to_string("/nonexistent/wp.json")
                    .ok()
                    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                    .and_then(|wp| wp.get("timeout_s")?.as_u64())
                    .unwrap_or_else(|| lease_for("workplan").as_secs());
                assert_eq!(timeout, 60 * 60);
            }

            #[test]
            fn dispatch_uses_stored_timeout_s_over_default() {
                let task = serde_json::json!({
                    "id": "test-123",
                    "kind": "workplan",
                    "timeout_s": 3600u64,
                });
                let window_secs = task
                    .get("timeout_s")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| lease_for("workplan").as_secs());
                assert_eq!(window_secs, 3600);
                assert_eq!(Duration::from_secs(window_secs), Duration::from_secs(3600));
            }

            #[test]
            fn dispatch_falls_back_when_timeout_s_missing() {
                let task = serde_json::json!({
                    "id": "test-456",
                    "kind": "workplan",
                });
                let window_secs = task
                    .get("timeout_s")
                    .and_then(|v| v.as_u64())
                    .unwrap_or_else(|| lease_for("workplan").as_secs());
                assert_eq!(window_secs, 60 * 60);
            }
        }

        // ─── Inline fallback decision (ADR-2026-04-14-1400 §1 inline-fallback) ───
        //
        // Locks the predicate `should_fallback_inline`: daemon drain MUST
        // fall back to inline execute_brain_task whenever the swarm-lease
        // path yields no live worker, so the §1 P1 evidence guard stays
        // active on the daemon hot path. Runs under
        // `cargo test -p hex-cli brain::inline_fallback`.

        mod inline_fallback {
            use super::super::super::{should_fallback_inline, DispatchOutcome};

            #[test]
            fn error_outcome_triggers_fallback() {
                let out = DispatchOutcome::Error("nexus down".into());
                assert!(
                    should_fallback_inline(&out),
                    "Err(_) from dispatch_brain_task must force inline exec"
                );
            }

            #[test]
            fn leased_empty_triggers_fallback() {
                // Today every dispatch returns LeasedEmpty because §2 worker
                // registration has not shipped — so the fallback path is the
                // one actually exercised. This test is the contract that
                // keeps the guard alive on the hot path.
                let out = DispatchOutcome::LeasedEmpty {
                    swarm_id: "s1".into(),
                    swarm_task_id: "st1".into(),
                };
                assert!(
                    should_fallback_inline(&out),
                    "empty swarm must fall back to inline exec"
                );
            }

            #[test]
            fn leased_to_worker_does_not_fall_back() {
                // Once §2 lands, a live worker-polling swarm must NOT be
                // pre-empted by inline exec — that would double-run the task.
                let out = DispatchOutcome::LeasedToWorker {
                    swarm_id: "s1".into(),
                    swarm_task_id: "st1".into(),
                    agent_id: "agent-7".into(),
                    leased_until: "2026-04-14T00:30:00Z".into(),
                };
                assert!(
                    !should_fallback_inline(&out),
                    "live worker must retain the lease — no inline exec"
                );
            }
        }

        // ─── Sweep timeout logic (ADR-2026-04-14-2155 P2.2) ────────────────────

        mod sweep_timeout {
            use super::super::super::{
                BrainTaskEvidence, BrainTaskRecord, BrainTaskStatus,
                lease_for, SWEEP_GRACE_SECS,
            };
            use serde_json::json;

            fn make_record(id: &str, kind: &str, created_at: &str, timeout_s: Option<u64>) -> BrainTaskRecord {
                BrainTaskRecord {
                    id: id.into(),
                    kind: kind.into(),
                    payload: "test".into(),
                    status: BrainTaskStatus::InProgress,
                    project_id: "p".into(),
                    created_at: created_at.into(),
                    completed_at: None,
                    result: None,
                    timeout_s,
                    leased_to: None,
                    leased_until: None,
                    lease_attempts: 0,
                    swarm_task_id: None,
                    evidence: BrainTaskEvidence::default(),
                    priority: 0,
                }
            }

            #[test]
            fn grace_period_is_30s() {
                assert_eq!(SWEEP_GRACE_SECS, 30);
            }

            #[test]
            fn timeout_s_stored_in_record_via_serde() {
                let v = json!({
                    "id": "t1",
                    "kind": "workplan",
                    "payload": "wp.json",
                    "status": "in_progress",
                    "created_at": "2026-04-14T00:00:00Z",
                    "timeout_s": 600,
                });
                let rec = BrainTaskRecord::from_value(&v).expect("parse");
                assert_eq!(rec.timeout_s, Some(600));
            }

            #[test]
            fn timeout_s_defaults_to_none_when_absent() {
                let v = json!({
                    "id": "t2",
                    "kind": "shell",
                    "payload": "echo hi",
                    "status": "in_progress",
                    "created_at": "2026-04-14T00:00:00Z",
                });
                let rec = BrainTaskRecord::from_value(&v).expect("parse");
                assert_eq!(rec.timeout_s, None);
            }

            #[test]
            fn effective_timeout_uses_stored_value_when_present() {
                let rec = make_record("t3", "workplan", "2026-04-14T00:00:00Z", Some(600));
                let effective = rec.timeout_s.unwrap_or_else(|| lease_for(&rec.kind).as_secs());
                assert_eq!(effective, 600);
            }

            #[test]
            fn effective_timeout_falls_back_to_lease_for_when_absent() {
                let rec = make_record("t4", "workplan", "2026-04-14T00:00:00Z", None);
                let effective = rec.timeout_s.unwrap_or_else(|| lease_for(&rec.kind).as_secs());
                assert_eq!(effective, 60 * 60);
            }

            #[test]
            fn sweep_deadline_includes_grace() {
                let timeout: u64 = 120;
                let deadline = timeout + SWEEP_GRACE_SECS;
                assert_eq!(deadline, 150);
            }

            #[test]
            fn task_within_deadline_is_not_swept() {
                let now = chrono::Utc::now();
                let created = now - chrono::Duration::seconds(100);
                let rec = make_record("t5", "shell", &created.to_rfc3339(), Some(120));
                let age = now.signed_duration_since(
                    chrono::DateTime::parse_from_rfc3339(&rec.created_at)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                );
                let deadline = rec.timeout_s.unwrap() + SWEEP_GRACE_SECS;
                assert!(
                    age.num_seconds() < deadline as i64,
                    "task at 100s should be within 150s deadline"
                );
            }

            #[test]
            fn task_past_deadline_is_swept() {
                let now = chrono::Utc::now();
                let created = now - chrono::Duration::seconds(200);
                let rec = make_record("t6", "shell", &created.to_rfc3339(), Some(120));
                let age = now.signed_duration_since(
                    chrono::DateTime::parse_from_rfc3339(&rec.created_at)
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                );
                let deadline = rec.timeout_s.unwrap() + SWEEP_GRACE_SECS;
                assert!(
                    age.num_seconds() >= deadline as i64,
                    "task at 200s should exceed 150s deadline"
                );
            }

            #[test]
            fn timeout_s_roundtrips_through_json() {
                let rec = make_record("t7", "workplan", "2026-04-14T00:00:00Z", Some(1800));
                let v = serde_json::to_value(&rec).unwrap();
                assert_eq!(v["timeout_s"].as_u64(), Some(1800));
                let round = BrainTaskRecord::from_value(&v).expect("roundtrip");
                assert_eq!(round.timeout_s, Some(1800));
            }
        }

        // ─── Idle-research trigger gate (wp-idle-research-swarm P1.4 / ADR-2026-04-15-1200) ──
        //
        // Locks the predicate `should_self_enqueue_research_sweep` and its
        // dependency `sweep_throttle_elapsed`. The trigger is the only
        // mechanism that turns a quiet repo into self-directed research, so
        // both gates (idle-tick threshold AND min-interval throttle) must
        // hold or autonomy regresses to operator-driven prompting.
        //
        // Runs under `cargo test -p hex-cli brain::idle_research_trigger`.

        mod idle_research_trigger {
            use super::super::super::{
                should_self_enqueue_research_sweep, sweep_throttle_elapsed,
                DEFAULT_IDLE_THRESHOLD_TICKS, DEFAULT_MIN_SWEEP_INTERVAL_H,
            };

            #[test]
            fn defaults_match_workplan_contract() {
                // wp-idle-research-swarm P1.2 specifies N=4 ticks and a 6h
                // throttle. If either default drifts the integration tests
                // below would still pass on stale numbers — anchor the
                // contract here.
                assert_eq!(DEFAULT_IDLE_THRESHOLD_TICKS, 4);
                assert_eq!(DEFAULT_MIN_SWEEP_INTERVAL_H, 6);
            }

            #[test]
            fn fires_after_four_consecutive_empty_ticks() {
                // Exactly the threshold (N=4) is enough; first sweep has no
                // prior `last_sweep`, so the throttle gate is open.
                let now = chrono::Utc::now();
                assert!(
                    should_self_enqueue_research_sweep(4, 4, None, now, 6),
                    "idle_ticks=threshold with no prior sweep must fire"
                );
                // Three idle ticks is one short of the threshold — the
                // trigger must stay quiet, otherwise we'd self-enqueue on
                // every short lull.
                assert!(
                    !should_self_enqueue_research_sweep(3, 4, None, now, 6),
                    "idle_ticks below threshold must not fire"
                );
            }

            #[test]
            fn does_not_fire_when_last_sweep_under_six_hours_ago() {
                // Fresh sweep (5h ago) with the idle gate satisfied: throttle
                // wins. Without this gate, a quiet repo would re-enqueue
                // research-sweep on every drain tick.
                let now = chrono::Utc::now();
                let last = now - chrono::Duration::hours(5);
                assert!(
                    !should_self_enqueue_research_sweep(4, 4, Some(last), now, 6),
                    "throttle must block fire when last sweep < interval"
                );
                // A 1-second gap after a fresh sweep is the worst case — the
                // gate must hold even with idle_ticks vastly exceeding N.
                let just_now = now - chrono::Duration::seconds(1);
                assert!(
                    !should_self_enqueue_research_sweep(99, 4, Some(just_now), now, 6),
                    "high idle count cannot bypass the throttle"
                );
            }

            #[test]
            fn fires_when_last_sweep_at_least_six_hours_ago() {
                // Boundary: exactly `interval_h` hours elapsed is eligible.
                // The predicate is `>=`, not strict-`>` — locking that here
                // prevents a future refactor from silently shrinking the
                // window by a tick.
                let now = chrono::Utc::now();
                let exactly_six = now - chrono::Duration::hours(6);
                assert!(
                    should_self_enqueue_research_sweep(4, 4, Some(exactly_six), now, 6),
                    "elapsed == interval must fire (>=, not strict >)"
                );
                // Well past the throttle: idle gate satisfied → fire.
                let long_ago = now - chrono::Duration::hours(24);
                assert!(
                    should_self_enqueue_research_sweep(4, 4, Some(long_ago), now, 6),
                    "elapsed >> interval must fire"
                );
            }

            #[test]
            fn idle_gate_required_even_when_throttle_is_open() {
                // Symmetry check: throttle alone (last_sweep stale) must NOT
                // fire if the queue is still busy. Both gates required —
                // dropping either turns the trigger into noise.
                let now = chrono::Utc::now();
                let long_ago = now - chrono::Duration::hours(48);
                assert!(
                    !should_self_enqueue_research_sweep(0, 4, Some(long_ago), now, 6),
                    "busy queue (idle=0) must block fire even with stale throttle"
                );
                assert!(
                    !should_self_enqueue_research_sweep(3, 4, Some(long_ago), now, 6),
                    "below-threshold idle must block fire even with stale throttle"
                );
            }

            #[test]
            fn sweep_throttle_treats_none_as_eligible() {
                // `None` = never swept. The first sweep should fire as soon
                // as the idle gate is satisfied, otherwise a brand-new
                // install would never trigger research until an operator
                // manually seeded `~/.hex/sched/last_research_sweep`.
                let now = chrono::Utc::now();
                assert!(sweep_throttle_elapsed(None, now, 6));
                assert!(sweep_throttle_elapsed(None, now, 0));
            }
        }

        // ─── Sweep preemption (wp-idle-research-swarm P4.4) ─────────────
        //
        // Locks the predicate `should_preempt_sweep`. The pure-logic gate
        // is what queue_drain uses to decide whether to write the abort
        // signal; getting it wrong either spams aborts at no sweep (and
        // leaves a stale signal that cancels the *next* sweep) or fails
        // to preempt when real work arrives (and a low-priority research
        // sweep blocks user-driven tasks).
        mod sweep_preemption_predicate {
            use super::super::super::should_preempt_sweep;

            #[test]
            fn fires_when_sweep_in_flight_and_non_research_pending() {
                // Canonical preemption case: a workplan task arrives mid-sweep.
                let pending = vec!["workplan", "research-sweep"];
                assert!(
                    should_preempt_sweep(&pending, true),
                    "non-research pending + sweep in flight must preempt"
                );
            }

            #[test]
            fn does_not_fire_when_no_sweep_in_flight() {
                // Without an active sweep there's nothing to abort. Writing
                // the abort signal anyway would leave a stale signal on
                // disk that the *next* sweep would honor — so the next
                // idle window's research would self-cancel before doing
                // any work.
                let pending = vec!["workplan", "shell"];
                assert!(
                    !should_preempt_sweep(&pending, false),
                    "must not fire abort when no sweep is running"
                );
            }

            #[test]
            fn does_not_fire_when_only_research_pending() {
                // A second research-sweep queued behind an in-flight one
                // should be coalesced by the throttle, not preempted.
                // Preempting here would force-abort a sweep just to
                // re-enqueue the same kind of work — pure churn.
                let pending = vec!["research-sweep", "research-sweep"];
                assert!(
                    !should_preempt_sweep(&pending, true),
                    "research-only pending must not preempt"
                );
            }

            #[test]
            fn does_not_fire_when_pending_empty() {
                // Empty queue = nothing to prioritize over the sweep.
                assert!(
                    !should_preempt_sweep(&[], true),
                    "empty pending must not preempt"
                );
                assert!(
                    !should_preempt_sweep(&[], false),
                    "empty pending without sweep must not preempt"
                );
            }

            #[test]
            fn fires_for_any_non_research_kind() {
                // The kind check is a single negative match — every
                // currently-defined kind except `research-sweep` is a
                // valid preemption trigger. Pin a representative set so a
                // future "low-priority" kind doesn't silently bypass the
                // gate.
                for kind in [
                    "workplan",
                    "hex-command",
                    "shell",
                    "analyze",
                    "remote-shell",
                    "unknown-future-kind",
                ] {
                    assert!(
                        should_preempt_sweep(&[kind], true),
                        "kind `{kind}` should trigger preemption"
                    );
                }
            }
        }

        // ─── last_sweep summary line (wp-idle-research-swarm P5.1) ──────
        //
        // Locks the operator-facing `last_sweep:` formatter used by both
        // `hex sched daemon status` and the no-arg `hex` status panel.
        // The two surfaces share `last_sweep_summary_line`, which composes
        // these pure helpers — testing the helpers covers both surfaces.
        mod last_sweep_summary {
            use super::super::super::{
                count_drafts, find_latest_sweep_yaml, format_last_sweep_line, format_sweep_age,
                SweepDocSummary,
            };
            use chrono::{Duration, TimeZone, Utc};
            use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};

            fn finding(id: &str, kind: ActionKind) -> Finding {
                Finding {
                    id: id.into(),
                    domain: Domain::Architecture,
                    severity: Severity::Medium,
                    title: "t".into(),
                    evidence: vec![],
                    suggested_action: SuggestedAction {
                        kind,
                        draft_ref: None,
                    },
                }
            }

            #[test]
            fn format_age_buckets_at_minute_hour_day_boundaries() {
                // Each bucket boundary is a separate code path — drift here
                // would silently flip "59m" to "0h" or "23h" to "0d".
                assert_eq!(format_sweep_age(Duration::seconds(0)), "0s");
                assert_eq!(format_sweep_age(Duration::seconds(59)), "59s");
                assert_eq!(format_sweep_age(Duration::seconds(60)), "1m");
                assert_eq!(format_sweep_age(Duration::minutes(59)), "59m");
                assert_eq!(format_sweep_age(Duration::minutes(60)), "1h");
                assert_eq!(format_sweep_age(Duration::hours(23)), "23h");
                assert_eq!(format_sweep_age(Duration::hours(24)), "1d");
                assert_eq!(format_sweep_age(Duration::days(7)), "7d");
            }

            #[test]
            fn format_age_clamps_negative_to_zero() {
                // Clock skew between sweep_at (set by nexus) and `now` (set
                // by cli) could produce a negative duration. The status
                // panel must never print "-3s ago".
                assert_eq!(format_sweep_age(Duration::seconds(-42)), "0s");
            }

            #[test]
            fn count_drafts_includes_workplan_amend_and_adr_only() {
                // The contract for "drafts" is "findings that produce an
                // on-disk artifact" — Memory + Informational don't, so
                // they must NOT count even though they're real findings.
                let xs = vec![
                    finding("w", ActionKind::DraftWorkplan),
                    finding("a", ActionKind::DraftAdr),
                    finding("m", ActionKind::AmendWorkplan),
                    finding("mem", ActionKind::Memory),
                    finding("info", ActionKind::Informational),
                ];
                assert_eq!(count_drafts(&xs), 3);
            }

            #[test]
            fn count_drafts_zero_when_empty() {
                // A sweep with zero findings (clean repo) must report 0
                // drafts, not panic and not fall through to "1".
                assert_eq!(count_drafts(&[]), 0);
            }

            #[test]
            fn format_last_sweep_line_renders_canonical_output() {
                // Locks the exact wire format the workplan calls out:
                //   "<age> ago (<n> findings, <m> drafts)"
                // The age side is formatted by `format_sweep_age` (locked
                // separately); the counts side is the contract this test
                // owns. Drift here would either rename the columns or
                // change the parens/comma layout dashboards may grep for.
                let sweep_at = Utc.with_ymd_and_hms(2026, 4, 29, 10, 0, 0).unwrap();
                let now = sweep_at + Duration::hours(5);
                let doc = SweepDocSummary {
                    sweep_at: sweep_at.to_rfc3339(),
                    findings_total: 12,
                    findings: vec![
                        finding("w1", ActionKind::DraftWorkplan),
                        finding("w2", ActionKind::DraftWorkplan),
                        finding("a1", ActionKind::DraftAdr),
                        finding("mem", ActionKind::Memory),
                    ],
                };
                assert_eq!(
                    format_last_sweep_line(&doc, now).as_deref(),
                    Some("5h ago (12 findings, 3 drafts)")
                );
            }

            #[test]
            fn format_last_sweep_line_uses_findings_total_over_emitted() {
                // `findings_total` is pre-cap; `findings.len()` is post-cap.
                // The honest "n findings" is the pre-cap count — a low cap
                // shouldn't make a noisy repo look quiet on the status
                // line. The fallback only kicks in when total is missing.
                let sweep_at = Utc.with_ymd_and_hms(2026, 4, 29, 10, 0, 0).unwrap();
                let now = sweep_at + Duration::minutes(30);
                let doc = SweepDocSummary {
                    sweep_at: sweep_at.to_rfc3339(),
                    findings_total: 25,
                    findings: vec![finding("w1", ActionKind::DraftWorkplan)],
                };
                assert_eq!(
                    format_last_sweep_line(&doc, now).as_deref(),
                    Some("30m ago (25 findings, 1 drafts)")
                );
            }

            #[test]
            fn format_last_sweep_line_falls_back_to_emitted_when_total_missing() {
                // Older YAMLs (pre-`findings_total`) deserialize with the
                // default 0. Without the fallback the panel would say
                // "0 findings, 2 drafts" — internally inconsistent.
                let sweep_at = Utc.with_ymd_and_hms(2026, 4, 29, 10, 0, 0).unwrap();
                let now = sweep_at + Duration::hours(2);
                let doc = SweepDocSummary {
                    sweep_at: sweep_at.to_rfc3339(),
                    findings_total: 0,
                    findings: vec![
                        finding("w1", ActionKind::DraftWorkplan),
                        finding("a1", ActionKind::DraftAdr),
                    ],
                };
                assert_eq!(
                    format_last_sweep_line(&doc, now).as_deref(),
                    Some("2h ago (2 findings, 2 drafts)")
                );
            }

            #[test]
            fn format_last_sweep_line_returns_none_for_unparseable_timestamp() {
                // A malformed `sweep_at` must not crash the status panel —
                // returning `None` causes the caller to omit the line, which
                // is the right behavior (no signal beats wrong signal).
                let doc = SweepDocSummary {
                    sweep_at: "not-a-timestamp".into(),
                    findings_total: 1,
                    findings: vec![finding("w1", ActionKind::DraftWorkplan)],
                };
                assert!(format_last_sweep_line(&doc, Utc::now()).is_none());
            }

            #[test]
            fn find_latest_sweep_yaml_picks_lexicographically_newest() {
                // The filename stem `idle-sweep-YYYYMMDD-HHMM` makes
                // lexicographic == chronological — locks the assumption
                // so a future stem refactor (e.g. seconds, or an ID
                // suffix) doesn't silently break "newest first".
                let dir = tempfile::tempdir().expect("tempdir");
                let analysis = dir.path().join("docs").join("analysis");
                std::fs::create_dir_all(&analysis).unwrap();
                std::fs::write(
                    analysis.join("idle-sweep-20260101-1200.yaml"),
                    "sweep_at: 2026-01-01T12:00:00Z\nfindings: []\n",
                )
                .unwrap();
                std::fs::write(
                    analysis.join("idle-sweep-20260429-0900.yaml"),
                    "sweep_at: 2026-04-29T09:00:00Z\nfindings: []\n",
                )
                .unwrap();
                std::fs::write(
                    analysis.join("idle-sweep-20260315-2359.yaml"),
                    "sweep_at: 2026-03-15T23:59:00Z\nfindings: []\n",
                )
                .unwrap();
                // Decoy files that must NOT be selected.
                std::fs::write(analysis.join("idle-sweep-20270101-0000.txt"), "").unwrap();
                std::fs::write(analysis.join("brain-string-audit.md"), "").unwrap();

                let path = find_latest_sweep_yaml(dir.path()).expect("found newest");
                assert!(
                    path.ends_with("idle-sweep-20260429-0900.yaml"),
                    "got {:?}",
                    path
                );
            }

            #[test]
            fn find_latest_sweep_yaml_returns_none_when_no_analysis_dir() {
                // Fresh repos have no `docs/analysis/`. The lookup must
                // degrade silently to `None`, not bubble an io::Error.
                let dir = tempfile::tempdir().expect("tempdir");
                assert!(find_latest_sweep_yaml(dir.path()).is_none());
            }

            #[test]
            fn find_latest_sweep_yaml_returns_none_when_no_matching_files() {
                // `docs/analysis/` exists but holds only unrelated files.
                // The function must filter strictly on the stem prefix.
                let dir = tempfile::tempdir().expect("tempdir");
                let analysis = dir.path().join("docs").join("analysis");
                std::fs::create_dir_all(&analysis).unwrap();
                std::fs::write(analysis.join("session-2604141400-insights.yaml"), "").unwrap();
                std::fs::write(analysis.join("brain-string-audit.md"), "").unwrap();
                assert!(find_latest_sweep_yaml(dir.path()).is_none());
            }
        }
    }
}
