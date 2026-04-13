//! Brain commands (ADR-2604102200).
//!
//! `hex brain status|test|scores|models|validate`
//!
//! status   - Show brain service status and configuration
//! test     - Run a manual test of a model
//! scores   - Show learned method scores
//! models   - List available models for brain selection
//! validate - Run self-diagnostics (CLI wiring, etc.)

use std::path::PathBuf;
use std::time::SystemTime;

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::fmt::{pretty_table, truncate};

/// Summary of a single workplan's reconciliation status.
#[derive(Debug)]
struct WorkplanSummary {
    id: String,
    feature: String,
    status: String,
    total_tasks: usize,
    done_tasks: usize,
    /// Tasks still marked "todo" but with git evidence (commit messages mentioning the task id).
    stale_tasks: Vec<String>,
    /// Path to the workplan JSON file (needed for auto-fix writes).
    path: PathBuf,
}

/// A stale git worktree detected during validation.
#[derive(Debug)]
struct StaleWorktree {
    path: String,
    branch: String,
    /// Seconds since the last commit on this worktree's branch.
    age_secs: u64,
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
}

pub async fn run(action: BrainAction) -> anyhow::Result<()> {
    match action {
        BrainAction::Status => status().await,
        BrainAction::Test { model } => test(&model).await,
        BrainAction::Scores => scores().await,
        BrainAction::Models => models().await,
        BrainAction::Validate => validate().await,
    }
}

async fn status() -> anyhow::Result<()> {
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    
    let url = format!("{}/api/brain/status", base_url);
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
    println!("  Interval: {} seconds", body.get("interval_secs").unwrap_or(&json!(600)));
    println!("  Last Test: {}", body.get("last_test").unwrap_or(&json!("never")));
    
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
fn check_workplan_status() -> anyhow::Result<Vec<WorkplanSummary>> {
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
fn check_stale_worktrees() -> anyhow::Result<Vec<StaleWorktree>> {
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
fn autofix_workplan(wp: &WorkplanSummary) -> anyhow::Result<usize> {
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

async fn validate() -> anyhow::Result<()> {
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

            // Auto-fix: reconcile stale tasks whose git evidence proves completion
            if total_stale > 0 {
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
                let effective_done = wp.done_tasks + wp.stale_tasks.len();
                let progress = if wp.total_tasks > 0 {
                    format!("{}/{}", effective_done, wp.total_tasks)
                } else {
                    "0/0".to_string()
                };
                let stale_note = if wp.stale_tasks.is_empty() {
                    String::new()
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

    Ok(())
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
}