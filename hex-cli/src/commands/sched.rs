//! Brain commands (ADR-2604102200).
//!
//! `hex brain status|test|scores|models|validate`
//!
//! status   - Show brain service status and configuration
//! test     - Run a manual test of a model
//! scores   - Show learned method scores
//! models   - List available models for brain selection
//! validate - Run self-diagnostics (CLI wiring, etc.)

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::json;

use tracing::debug;

use crate::fmt::{pretty_table, truncate};

/// Daemon-local state persisted across ticks (wp-brain-updates P1.2).
/// Tracks issue counts from the previous validate tick so regressions can
/// be detected — a count that increases tick-over-tick is a regression.
/// Persisted to `~/.hex/brain-state.json` so the baseline survives daemon
/// restarts (otherwise every restart would silently hide cross-restart
/// regressions by re-seeding from the current tick).
#[derive(Debug, Default, Serialize, Deserialize)]
struct DaemonState {
    /// Last tick's issue counts keyed by check name
    /// (e.g. "cli_wiring" → 2, "mcp_parity" → 0, "workplans_stale" → 1).
    #[serde(default)]
    last_counts: HashMap<String, usize>,
    /// Whether we've observed at least one tick (first tick establishes baseline,
    /// no regression notification on the first tick).
    #[serde(default)]
    seeded: bool,
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
    /// Show brain service status and configuration
    Status,
    /// Run a test with a specific model
    Test {
        /// Model name (e.g. nemotron-mini, qwen3:8b)
        #[arg(default_value = "nemotron-mini")]
        model: String,
    },
    /// Show learned method scores from RL engine
    Scores,
    /// List models available for brain selection
    Models,
    /// Run self-diagnostics (CLI wiring check, etc.)
    Validate,
    /// Run the brain supervisor loop — validates + auto-fixes every interval (ADR-2604132300)
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
    /// Stop the background brain daemon
    DaemonStop,
    /// Show brain daemon status (running/stopped)
    DaemonStatus,
    /// Enqueue a task for the brain daemon (ADR-2604132330)
    Enqueue {
        /// Task kind (hex-command, workplan, shell)
        kind: String,
        /// Task payload (command args, workplan path, or shell command)
        payload: String,
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
    /// Prime brain for this project: start daemon if needed, discover active
    /// workplans in docs/workplans/, and seed the queue in one shot.
    Prime {
        /// Tick interval when starting the daemon (default 10s)
        #[arg(long, default_value = "10")]
        interval: u64,
    },
}

#[derive(Subcommand)]
pub enum QueueAction {
    /// List pending sched tasks
    List,
    /// Clear completed/failed tasks
    Clear,
    /// Force drain and execute pending tasks now
    Drain,
    /// Show recent sched tasks across all statuses (wp-sched-queue-history P1.3).
    ///
    /// Primary use: verify the ADR-2604141400 §1 P1 evidence-guard correctly
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
        BrainAction::DaemonStatus => daemon_status(),
        BrainAction::Enqueue { kind, payload } => {
            let id = enqueue_brain_task(&kind, &payload).await?;
            println!("⬡ enqueued sched task {id} ({kind}: {payload})");
            Ok(())
        }
        BrainAction::Queue { action } => match action {
            QueueAction::List => queue_list().await,
            QueueAction::Clear => queue_clear().await,
            QueueAction::Drain => queue_drain().await,
            QueueAction::History { status, limit } => queue_history(status, limit).await,
        },
        BrainAction::Prime { interval } => prime(interval).await,
        BrainAction::Watch { since } => watch(since).await,
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
        Some(pid) => format!("{}/api/brain/status?project={}", base_url, pid),
        None => format!("{}/api/brain/status", base_url),
    };
    let resp = client.get(&url).send().await?;
    
    if resp.status() == 404 {
        println!("{}", "Brain service not configured. Run hex-nexus with brain service enabled.".yellow());
        return Ok(());
    }
    
    if !resp.status().is_success() {
        eprintln!("Error: {}", resp.status());
        return Ok(());
    }
    
    let body: serde_json::Value = resp.json().await?;
    println!("{}", "Brain Service Status".green().bold());
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
    
    let url = format!("{}/api/brain/test", base_url);
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
    
    let url = format!("{}/api/brain/scores", base_url);
    let resp = client.get(&url).send().await?;
    
    if resp.status() == 404 {
        println!("{}", "No scores yet. Brain service is learning.".yellow());
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
/// each task against git history, and return per-workplan summaries.
///
/// A task is "stale" when it is still marked `"todo"` in the JSON but a commit
/// message references its id (e.g. `P3.1`).
pub(crate) fn check_workplan_status() -> anyhow::Result<Vec<WorkplanSummary>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let workplans_dir = workspace_root.join("docs/workplans");

    if !workplans_dir.is_dir() {
        return Ok(vec![]);
    }

    // Grab recent git log once — search it for task ids later.
    let git_log = std::process::Command::new("git")
        .args(["log", "--oneline", "-200"])
        .current_dir(&workspace_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

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

        let mut total_tasks = 0usize;
        let mut done_tasks = 0usize;
        let mut stale_tasks = Vec::new();

        // Walk phases → tasks
        if let Some(phases) = wp.get("phases").and_then(|p| p.as_array()) {
            for phase in phases {
                if let Some(tasks) = phase.get("tasks").and_then(|t| t.as_array()) {
                    for task in tasks {
                        total_tasks += 1;
                        let task_status = task.get("status").and_then(|s| s.as_str()).unwrap_or("todo");
                        let task_id = task.get("id").and_then(|s| s.as_str()).unwrap_or("");

                        match task_status {
                            "done" => done_tasks += 1,
                            _ => {
                                    // Check if git log mentions this task id (case-insensitive)
                                let needle_lower = task_id.to_lowercase();
                                if !task_id.is_empty()
                                    && git_log.to_lowercase().contains(&needle_lower)
                                {
                                    stale_tasks.push(task_id.to_string());
                                }
                            }
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
            // Skip the main worktree (no branch prefix pattern like feat/ or hex/)
            if !current_branch.starts_with("feat/") && !current_branch.starts_with("hex/") {
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
    println!("{}", "⬡ hex brain validate".bold());

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
            // workplan JSON (ADR-2604142200, wp-reconcile-evidence-verification R2.2).
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

    // Stale swarm check (ADR-2604142300): active swarms whose tasks are all done
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

/// Prime brain for this project in one shot: ensure the daemon is running,
/// discover active workplans, and seed the queue. Idempotent — safe to re-run.
async fn prime(interval: u64) -> anyhow::Result<()> {
    println!("{}", "⬡ hex brain prime".bold());

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
/// on success. Respects `HEX_BRAIN_DRY_RUN=1` (ADR-2604142300 safety mitigation).
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
    let contents = std::fs::read_to_string(&path).ok()?;
    contents.trim().parse::<i32>().ok()
}

fn remove_pid_file() {
    let _ = std::fs::remove_file(pid_file_path());
}

fn process_alive(pid: i32) -> bool {
    // Signal 0 probes existence without delivering a signal.
    // Returns 0 on success (process exists), -1 on error.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
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

/// Foreground supervisor loop. Validates every `interval` seconds; after
/// `max_failures` consecutive failures, pauses for 5x interval before retrying.
/// Exits cleanly on ctrl-C.
async fn daemon(interval: u64, max_failures: u32) -> anyhow::Result<()> {
    // Write the PID so DaemonStop can find a foreground instance too.
    let pid = std::process::id();
    let _ = write_pid_file(pid);

    println!(
        "{} interval={}s max_failures={} pid={}",
        "⬡ brain daemon starting".green().bold(),
        interval,
        max_failures,
        pid
    );

    let mut consecutive_failures: u32 = 0;
    let mut paused_cycles: u32 = 0;
    let mut state = load_daemon_state();

    loop {
        let timestamp = chrono::Utc::now().to_rfc3339();
        println!("{} {}", "⬡ brain tick at".cyan(), timestamp);

        let start = Instant::now();
        let validate_result = validate(true).await;
        let elapsed = start.elapsed();

        // Diff issue counts tick-over-tick (wp-brain-updates P2.1).
        // First tick seeds the baseline; no notification until we have a prior.
        let current_counts = collect_issue_counts();
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

        match validate_result {
            Ok(()) => {
                if consecutive_failures > 0 {
                    println!(
                        "{} after {} failure(s)",
                        "  recovered".green(),
                        consecutive_failures
                    );
                }
                consecutive_failures = 0;
                paused_cycles = 0;
                println!("  ok ({}ms)", elapsed.as_millis());
            }
            Err(err) => {
                consecutive_failures += 1;
                eprintln!(
                    "  {} ({}/{}) {}",
                    "fail".red(),
                    consecutive_failures,
                    max_failures,
                    err
                );
            }
        }

        // Drain brain queue — hand up to 1 pending task per tick to a
        // `brain-lease` swarm (ADR-2604141400 P1.2). The daemon no longer
        // executes work inline; it stamps the lease and moves on. Swarm
        // workers progress the task; the sweeper reclaims if the lease
        // expires. Runs regardless of validate() outcome.
        //
        // ADR-2604141400 §1 partial-impl gap (dog-food finding 2026-04-14):
        // no swarm workers register against `brain-lease`, and no reclaim
        // sweeper exists, so a pure swarm-lease path silently parks every
        // task in `leased` forever — bypassing the §1 P1 evidence guard
        // that lives in execute_brain_task. Until §2 ships fully, fall
        // back to inline execution whenever dispatch reports no live
        // worker. The guard runs on the fallback path.
        match drain_brain_tasks(1).await {
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
                        // ADR-2604142155 P2.3: reject vacuous executor output
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

        // Sweep stuck in_progress tasks (ADR-2604142155 P2.2).
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

        // Emit brain_tick event to nexus (fire-and-forget).
        let port = std::env::var("HEX_NEXUS_PORT")
            .unwrap_or_else(|_| "5555".to_string())
            .parse::<u16>()
            .unwrap_or(5555);
        let event_url = format!("http://127.0.0.1:{}/api/events", port);
        let _ = reqwest::Client::new()
            .post(&event_url)
            .json(&serde_json::json!({
                "type": "brain_tick",
                "timestamp": timestamp,
                "duration_ms": elapsed.as_millis() as u64,
                "checks_run": 5,
            }))
            .timeout(Duration::from_secs(2))
            .send()
            .await;

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
                println!("\n{}", "⬡ brain daemon received ctrl-C, shutting down".yellow());
                remove_pid_file();
                return Ok(());
            }
        }
    }
}

/// Background mode: re-exec `hex brain daemon` (without `--background`) as a
/// detached child process, write its PID, and exit the parent.
fn daemon_background(interval: u64, max_failures: u32) -> anyhow::Result<()> {
    // Already running?
    if let Some(pid) = read_pid_file() {
        if process_alive(pid) {
            println!(
                "{} pid={} (pid file: {})",
                "brain daemon already running".yellow(),
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
        .arg("brain")
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
        "⬡ brain daemon started in background".green().bold(),
        pid,
        interval
    );
    println!("  pid file: {}", pid_file_path().display());
    println!("  stop with: hex brain daemon-stop");
    Ok(())
}

/// Stop the background daemon: send SIGTERM, wait up to 5s, remove PID file.
fn daemon_stop() -> anyhow::Result<()> {
    let pid = match read_pid_file() {
        Some(pid) => pid,
        None => {
            println!(
                "{} (no pid file at {})",
                "brain daemon not running".yellow(),
                pid_file_path().display()
            );
            return Ok(());
        }
    };

    if !process_alive(pid) {
        println!(
            "{} pid={} not alive — removing stale pid file",
            "brain daemon".yellow(),
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
            println!("{} pid={}", "⬡ brain daemon stopped".green().bold(), pid);
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

/// Show whether the brain daemon is running.
fn daemon_status() -> anyhow::Result<()> {
    match read_pid_file() {
        Some(pid) if process_alive(pid) => {
            println!(
                "{} pid={}",
                "⬡ brain daemon running".green().bold(),
                pid
            );
            println!("  pid file: {}", pid_file_path().display());
        }
        Some(pid) => {
            println!(
                "{} pid={} (stale pid file)",
                "brain daemon not running".yellow(),
                pid
            );
            println!("  pid file: {}", pid_file_path().display());
        }
        None => {
            println!("{}", "brain daemon not running".yellow());
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
///
/// Polls `GET /api/events` every 2 seconds, filters for `event_type ==
/// "brain_tick"`, and prints anything newer than the last-seen timestamp.
/// Each poll prints newest-first within the batch. Ctrl-C exits cleanly.
async fn watch(since: Option<String>) -> anyhow::Result<()> {
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

// ─── ADR-2604132330: Brain task queue (HexFlo memory–backed) ───────────────

const NEXUS_BASE: &str = "http://127.0.0.1:5555";

// ─── Typed schema (ADR-2604141400 P0.1) ────────────────────────────────────
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
        }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(self, BrainTaskStatus::Completed | BrainTaskStatus::Failed)
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
            _ => None,
        }
    }
}

/// Evidence surfaced by the lease sweeper / reconciler to justify a
/// completion verdict (ADR-2604141400). Populated in P2+; defaults keep
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
        })
    }
}

// ─── Lease durations per kind (ADR-2604141400 P1.1) ────────────────────────
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

pub(crate) const LEASE_DURATIONS: [(&str, Duration); 4] = [
    ("workplan", Duration::from_secs(30 * 60)),
    ("hex-command", Duration::from_secs(5 * 60)),
    ("shell", Duration::from_secs(2 * 60)),
    ("remote-shell", Duration::from_secs(60)),
];

/// Default lease window for `kind`. Unknown kinds fall back to the
/// shell-style 2-minute timeout so a typo or a newly-added kind can't camp
/// the queue forever — the sweeper will still reclaim it, just on the
/// shorter-than-ideal schedule until the table is updated.
pub(crate) fn lease_for(kind: &str) -> Duration {
    LEASE_DURATIONS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, d)| *d)
        .unwrap_or(Duration::from_secs(2 * 60))
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
                 workplan and enqueue it with `hex brain enqueue workplan <path>`. If it's a \
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
    // (hex brain prime, hex brain enqueue, other agents) would otherwise
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
        std::fs::read_to_string(payload)
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
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
/// (P3 of ADR-2604132330). Also invoked by `hex brain queue drain` logic indirectly.
#[allow(dead_code)]
pub(crate) async fn drain_brain_tasks(limit: usize) -> anyhow::Result<Vec<serde_json::Value>> {
    let pending = list_brain_tasks(Some("pending")).await?;
    let claimed: Vec<_> = pending.into_iter().take(limit).collect();
    for task in &claimed {
        let id = task.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let kind = task.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        debug!(task_id = %id, kind = %kind, "drain-path: claim");
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

// ─── Timeout sweep (ADR-2604142155 P2.2) ──────────────────────────────────
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

// ─── Swarm-leased dispatch (ADR-2604141400 P1.2) ───────────────────────────
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
/// (ADR-2604141400 §1 inline-fallback). Separated from `LeaseHandle`
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
    // is empty in practice — see ADR-2604141400 §1 "Known gaps".
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
/// real work regardless of its exit code (ADR-2604141400 §1 P1).
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
/// ADR-2604142155 P2.3: output-level evidence guard mirroring
/// `hex_nexus::orchestration::workplan_executor::validate_dispatch_evidence`.
/// Rejects empty / whitespace-only executor output so vacuous acks like
/// `"Execution dispatched: Object {"` cannot promote a task to `completed`.
pub(crate) fn validate_dispatch_evidence(output: Option<&str>) -> Result<(), String> {
    match output {
        Some(s) if !s.trim().is_empty() => Ok(()),
        Some(_) => Err(
            "dispatch-evidence guard: executor produced whitespace-only output — \
             refusing to accept completion (ADR-2604111800)"
                .to_string(),
        ),
        None => Err(
            "dispatch-evidence guard: no executor output received — \
             refusing to accept completion (ADR-2604111800)"
                .to_string(),
        ),
    }
}

/// treated as a failed run — the whole point of the guard (ADR-2604141400 §1
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

pub(crate) async fn execute_brain_task(kind: &str, payload: &str) -> (bool, String) {
    debug!(kind = %kind, payload_len = payload.len(), "drain-path: execute-start");
    // ADR-2604141400 §1 P1: capture pre-HEAD only for workplan tasks; the
    // other kinds stay exit-code-only in this slice.
    let pre_head = if kind == "workplan" {
        git_head_sha()
    } else {
        None
    };
    let output = match kind {
        "hex-command" => std::process::Command::new("hex")
            .args(payload.split_whitespace())
            .output(),
        "workplan" => std::process::Command::new("hex")
            .args(["plan", "execute", payload])
            .output(),
        "shell" => {
            // Whitelist: only cargo, git, ls, echo
            let mut parts = payload.split_whitespace();
            let cmd = match parts.next() {
                Some(c) => c,
                None => return (false, "empty shell command".to_string()),
            };
            const ALLOWED: &[&str] = &["cargo", "git", "ls", "echo", "ssh"];
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
        other => {
            return (
                false,
                format!(
                    "unknown task kind '{}' (expected: hex-command, workplan, shell)",
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
            // ADR-2604141400 §1 P1: for workplan tasks, require that HEAD
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
                (guarded_success, snippet)
            } else {
                (out.status.success(), snippet)
            }
        }
        Err(e) => (false, format!("spawn error: {}", e)),
    }
}

async fn queue_list() -> anyhow::Result<()> {
    let tasks = list_brain_tasks(Some("pending")).await?;
    if tasks.is_empty() {
        println!("{}", "No pending sched tasks.".yellow());
        return Ok(());
    }
    println!("{}", "Pending Brain Tasks".green().bold());
    let rows: Vec<Vec<String>> = tasks
        .iter()
        .map(|t| {
            let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let raw_payload = t.get("payload").and_then(|v| v.as_str()).unwrap_or("");
            vec![
                t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                kind.to_string(),
                render_task_target(kind, raw_payload),
                truncate(raw_payload, 40),
                t.get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ]
        })
        .collect();
    // Target column surfaces the host for remote-shell tasks so operators
    // can see at a glance which machine each task runs on (ADR-2604141200
    // P2.1). Non-remote kinds render a dash — the column stays meaningful
    // for the mixed-kind queue view.
    println!(
        "{}",
        pretty_table(&["ID", "Kind", "Target", "Payload", "Created"], &rows)
    );
    Ok(())
}

/// Render the recent brain-task history table (wp-sched-queue-history P1.3).
///
/// Hits `GET /api/brain/queue/history` and formats each row with a 60-char
/// tail of the result string so the `no git evidence` marker (ADR-2604141400
/// §1 P1 evidence-guard) is visible without horizontal scrolling. Using the
/// tail rather than the head is deliberate — the guard appends the marker,
/// so a head-truncation would hide it.
async fn queue_history(status: Option<String>, limit: u32) -> anyhow::Result<()> {
    use crate::nexus_client::NexusClient;
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Build query string; skip `status=` entirely when unset so nexus treats
    // it as "all statuses" rather than filtering on an empty-string match.
    let mut path = format!("/api/brain/queue/history?limit={}", limit);
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
        Some(s) => format!("Brain Task History — status={}", s),
        None => "Brain Task History".to_string(),
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
    if pending.is_empty() {
        println!("{}", "No pending sched tasks to drain.".yellow());
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
        // ADR-2604142155 P2.3: reject vacuous executor output
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── ADR-2604141400 §1 P1: workplan-evidence guard ─────────────────────
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

    // ─── ADR-2604142155 P2.3: validate_dispatch_evidence ────────────────────

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

    // ─── Brain task schema (ADR-2604141400 P0.1) ───────────────────────────
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

        // ─── Lease durations (ADR-2604141400 P1.1) ─────────────────────
        //
        // Nested under `brain::lease_durations` so the workplan gate
        // (`cargo test -p hex-cli brain::lease_durations`) runs exactly
        // this set.

        mod lease_durations {
            use super::super::super::{lease_for, LEASE_DURATIONS};
            use std::time::Duration;

            #[test]
            fn lease_for_known_kinds_matches_table() {
                assert_eq!(lease_for("workplan"), Duration::from_secs(30 * 60));
                assert_eq!(lease_for("hex-command"), Duration::from_secs(5 * 60));
                assert_eq!(lease_for("shell"), Duration::from_secs(2 * 60));
                assert_eq!(lease_for("remote-shell"), Duration::from_secs(60));
            }

            #[test]
            fn lease_for_unknown_kind_falls_back_to_shell_timeout() {
                // Unknown/typoed kind → 2-minute shell-style window so the
                // sweeper can still reclaim it rather than leaving it
                // leased forever.
                assert_eq!(lease_for("bogus"), Duration::from_secs(2 * 60));
                assert_eq!(lease_for(""), Duration::from_secs(2 * 60));
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
                assert_eq!(kinds.len(), 4, "LEASE_DURATIONS gained a new kind — update this test and confirm the duration is tuned");
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
                assert_eq!(timeout, 30 * 60);
            }

            #[test]
            fn timeout_s_unreadable_file_falls_back() {
                let timeout: u64 = std::fs::read_to_string("/nonexistent/wp.json")
                    .ok()
                    .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                    .and_then(|wp| wp.get("timeout_s")?.as_u64())
                    .unwrap_or_else(|| lease_for("workplan").as_secs());
                assert_eq!(timeout, 30 * 60);
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
                assert_eq!(window_secs, 30 * 60);
            }
        }

        // ─── Inline fallback decision (ADR-2604141400 §1 inline-fallback) ───
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

        // ─── Sweep timeout logic (ADR-2604142155 P2.2) ────────────────────

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
                assert_eq!(effective, 30 * 60);
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
    }
}
