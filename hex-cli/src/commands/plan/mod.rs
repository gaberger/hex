//! Workplan management command.
//!
//! `hex plan` — create, list, and inspect workplans.
//!
//! Workplans decompose requirements into hex-bounded tasks organized by
//! dependency tier. Plans are saved to `docs/workplans/` as JSON.

mod lint;
pub mod reconcile;
pub mod reconcile_evidence;
mod schema_validate;

use std::path::Path;

use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use tabled::Tabled;

use crate::fmt::{HexTable, status_badge, truncate, progress};
use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum PlanAction {
    /// Create a workplan from requirements
    Create {
        /// Requirements (space-separated)
        #[arg(required = true, num_args = 1..)]
        requirements: Vec<String>,

        /// Target language
        #[arg(long, default_value = "typescript")]
        lang: String,

        /// ADR reference (e.g. ADR-050). Required unless --no-adr is set.
        #[arg(long)]
        adr: Option<String>,

        /// Allow creating a workplan without an ADR reference
        #[arg(long, default_value_t = false)]
        no_adr: bool,
    },
    /// Execute a workplan — dispatches tasks through tiered inference routing
    Execute {
        /// Path to workplan JSON file
        file: String,
    },
    /// List existing workplans
    List,
    /// Show status of a specific workplan
    Status {
        /// Workplan filename (e.g. feat-secrets-plan-b.json)
        file: String,
    },
    /// Show currently active (running/paused) workplan executions
    Active,
    /// Show all past workplan executions
    History,
    /// Show aggregate report for a workplan execution (ADR-046)
    Report {
        /// Workplan execution ID
        id: String,
    },
    /// Reconcile workplan step statuses against actual code (check done conditions)
    Reconcile {
        /// Workplan filename (e.g. feat-fix-dev-pipeline.json)
        file: String,
        /// Write confirmed-done statuses back to the workplan JSON
        #[arg(long, default_value_t = false)]
        update: bool,
        /// Re-verify tasks already marked done and demote them when evidence
        /// fails. Heals JSONs corrupted by the pre-ADR-2604142200 reconcile
        /// loop. Combine with `--update` to persist demotions.
        #[arg(long, default_value_t = false)]
        audit: bool,
        /// Print per-task verdict and reasons without mutating the workplan
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Show full evidence detail for a single task
        #[arg(long)]
        why: Option<String>,
        /// Force-promote a task regardless of evidence (logs forced_by for audit)
        #[arg(long)]
        force: Option<String>,
    },
    /// Output the canonical workplan JSON schema
    Schema,
    /// Create a draft workplan from a user prompt (ADR-2604110227).
    ///
    /// Writes a stub JSON to `docs/workplans/drafts/draft-<timestamp>.json`
    /// containing the original prompt. The hex hook router auto-invokes
    /// this on T3-sized work-intent prompts; users can also invoke it
    /// directly. Drafts are not executed until approved.
    Draft {
        /// User prompt that triggered the draft (space-separated)
        #[arg(required = true, num_args = 1..)]
        prompt: Vec<String>,
        /// Suppress interactive output (used by auto-invocation from hook)
        #[arg(long, default_value_t = false)]
        background: bool,
    },
    /// Manage draft workplans (ADR-2604110227)
    Drafts {
        #[command(subcommand)]
        action: DraftsAction,
    },
    /// Validate workplan evidence (ADR-2604142200, wp-enforce-workplan-evidence E3.1).
    ///
    /// Runs `validate_workplan_evidence` on one workplan or on every
    /// `docs/workplans/wp-*.json`. Reports violations (task id + kind +
    /// remediation hint) in a table. Exit 0 when clean, non-zero when
    /// any violation is found. Intended for pre-commit hooks and CI.
    Lint {
        /// Path to a specific workplan JSON. Mutually exclusive with --all.
        #[arg(conflicts_with = "all")]
        file: Option<String>,
        /// Lint every docs/workplans/wp-*.json file.
        #[arg(long, default_value_t = false)]
        all: bool,
    },
}

/// Subcommands for `hex plan drafts` — manage auto-generated draft workplans.
#[derive(Subcommand)]
pub enum DraftsAction {
    /// List all in-flight draft workplans
    List,
    /// Delete all draft workplans (or one by name if --name is set)
    Clear {
        /// Name of the specific draft to remove (without .json extension)
        #[arg(long)]
        name: Option<String>,
    },
    /// Promote a draft to a real workplan (moves to docs/workplans/)
    Approve {
        /// Draft filename (with or without .json extension)
        name: String,
    },
    /// Garbage-collect drafts older than N days (default 7)
    Gc {
        /// Age threshold in days
        #[arg(long, default_value_t = 7)]
        days: u64,
    },
}

/// Deserialize a JSON null or missing string as empty string.
fn nullable_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// Deserialize phases that may be an array, object, number, or null — only arrays are used.
fn flexible_phases<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Phase>, D::Error> {
    let val = serde_json::Value::deserialize(d)?;
    match val {
        serde_json::Value::Array(_) => {
            serde_json::from_value(val).map_err(serde::de::Error::custom)
        }
        _ => Ok(Vec::new()), // object, number, null — treat as no phases
    }
}

/// A workplan step.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Step {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default, deserialize_with = "nullable_string")]
    pub(super) adapter: String,
    #[serde(default)]
    pub(super) tier: u8,
    #[serde(default)]
    pub(super) dependencies: Vec<String>,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) done_condition: String,
    #[serde(default)]
    pub(super) verify: String,
    #[serde(default)]
    pub(super) files: Vec<String>,
    #[serde(default)]
    pub(super) done_command: String,
}

/// A workplan document — supports both legacy (steps) and current (phases/tasks) formats.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Workplan {
    #[serde(default)]
    pub(super) title: String,
    #[serde(default)]
    pub(super) feature: String,
    #[serde(default)]
    pub(super) language: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) steps: Vec<Step>,
    #[serde(default, deserialize_with = "flexible_phases")]
    pub(super) phases: Vec<Phase>,
    #[serde(default, alias = "createdAt", alias = "created")]
    pub(super) created_at: String,
    #[serde(default)]
    pub(super) adr: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) priority: String,
    #[serde(default)]
    pub(super) superseded_by: String,
}

/// A phase in the current workplan format.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Phase {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) tasks: Vec<PhaseTask>,
}

/// A task within a phase.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct PhaseTask {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) layer: String,
    #[serde(default)]
    pub(super) files: Vec<String>,
    #[serde(default)]
    pub(super) done_command: String,
}

impl Workplan {
    /// Get the display title (prefers feature over title).
    fn display_title(&self) -> &str {
        if !self.feature.is_empty() {
            &self.feature
        } else if !self.title.is_empty() {
            &self.title
        } else {
            ""
        }
    }

    /// Count total tasks across both formats.
    fn total_tasks(&self) -> usize {
        if !self.phases.is_empty() {
            self.phases.iter().map(|p| p.tasks.len()).sum()
        } else {
            self.steps.len()
        }
    }

    /// Count completed tasks across both formats.
    fn completed_tasks(&self) -> usize {
        if !self.phases.is_empty() {
            self.phases.iter()
                .flat_map(|p| &p.tasks)
                .filter(|t| t.status == "done" || t.status == "completed")
                .count()
        } else {
            self.steps.iter()
                .filter(|s| s.status == "done" || s.status == "completed")
                .count()
        }
    }
}

// ── Tabled row types ───────────────────────────────────────────────────

#[derive(Tabled)]
struct PlanRow {
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Title")]
    title: String,
    #[tabled(rename = "ADR")]
    adr: String,
    #[tabled(rename = "Progress")]
    progress: String,
    #[tabled(rename = "Priority")]
    priority: String,
}

#[derive(Tabled)]
struct StepRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Description")]
    description: String,
    #[tabled(rename = "Adapter")]
    adapter: String,
    #[tabled(rename = "Tier")]
    tier: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Git")]
    git_evidence: String,
    #[tabled(rename = "Deps")]
    deps: String,
}

#[derive(Tabled)]
struct ExecutionRow {
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Feature")]
    feature: String,
    #[tabled(rename = "Phase")]
    phase: String,
    #[tabled(rename = "Progress")]
    progress_col: String,
}

#[derive(Tabled)]
struct HistoryRow {
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Feature")]
    feature: String,
    #[tabled(rename = "Tasks")]
    tasks: String,
    #[tabled(rename = "Started")]
    started: String,
    #[tabled(rename = "ID")]
    id: String,
}

pub async fn run(action: PlanAction) -> anyhow::Result<()> {
    match action {
        PlanAction::Create { requirements, lang, adr, no_adr } => create_plan(&requirements, &lang, adr.as_deref(), no_adr).await,
        PlanAction::Execute { file } => execute_plan(&file).await,
        PlanAction::List => list_plans().await,
        PlanAction::Status { file } => show_plan_status(&file).await,
        PlanAction::Active => show_active_executions().await,
        PlanAction::History => show_execution_history().await,
        PlanAction::Report { id } => show_execution_report(&id).await,
        PlanAction::Schema => show_schema().await,
        PlanAction::Reconcile { file, update, audit, dry_run, why, force } => {
            reconcile::run(&file, update, audit, dry_run, why.as_deref(), force.as_deref()).await
        }
        PlanAction::Draft { prompt, background } => draft_plan(&prompt, background).await,
        PlanAction::Drafts { action } => drafts_dispatch(action).await,
        PlanAction::Lint { file, all } => lint::run(file.as_deref(), all).await,
    }
}

/// Execute a workplan — dispatches tasks through tiered inference routing (ADR-2604120202).
///
/// Sends the workplan to hex-nexus for execution. Nexus routes each task through
/// Path C (headless inference for T1/T2/T2.5) or Path A (spawn agent for T3),
/// with compile gates, GBNF grammar constraints, and RL reward recording.
async fn execute_plan(file: &str) -> anyhow::Result<()> {
    let path = std::path::Path::new(file);
    let path = if path.exists() {
        path.to_path_buf()
    } else {
        let wp_path = std::path::Path::new("docs/workplans").join(file);
        if !wp_path.exists() {
            anyhow::bail!("Workplan not found: {} (also tried docs/workplans/{})", file, file);
        }
        wp_path
    };

    // Parse and validate the workplan
    let content = std::fs::read_to_string(&path)?;
    let wp: serde_json::Value = serde_json::from_str(&content)?;
    let feature = wp.get("feature").and_then(|v| v.as_str()).unwrap_or("(unnamed)");
    let phases = wp.get("phases").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    let total_tasks: usize = wp.get("phases")
        .and_then(|v| v.as_array())
        .map(|phases| phases.iter()
            .filter_map(|p| p.get("tasks").and_then(|t| t.as_array()))
            .map(|t| t.len())
            .sum())
        .unwrap_or(0);

    println!("{} Executing workplan: {}", "\u{2b21}".cyan(), feature);
    println!("  Phases: {}  Tasks: {}", phases, total_tasks);
    println!("  File:   {}", path.display());
    println!();

    // Resolve absolute path for nexus
    let _abs_path = std::fs::canonicalize(&path)?;

    // Dispatch strategy:
    // 1. Nexus reachable + Claude Code → Path B (nexus executor, Claude handles inference)
    // 2. Nexus reachable + standalone → distributed (HexFlo tasks for remote workers)
    // 3. No nexus → local fallback with Ollama + ADR-005 gates

    // Build authenticated nexus client
    let client = NexusClient::from_env();

    // Check if nexus is reachable
    match client.get("/api/health").await {
        Ok(_) => {
            let in_claude = std::env::var("CLAUDE_SESSION_ID").is_ok()
                || std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok();

            if in_claude {
                // Path B: nexus executor dispatches tasks, Claude Code handles inference
                println!("  {} Nexus + Claude Code — dispatching via Path B", "\u{2713}".green());
                println!("  Workplan sent to nexus for execution. Claude handles inference.");
                println!("  Monitor: hex plan active / hex task list");
                println!();

                // Send workplan to nexus executor (it creates swarm + uses Path B internally)
                let body = serde_json::json!({
                    "workplanPath": _abs_path.to_string_lossy(),
                });
                let dispatch_resp = match client.post_long("/api/workplan/execute", &body).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        let raw = format!("{}", e);
                        let detail = raw
                            .rsplit_once(": ")
                            .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body.trim()).ok())
                            .and_then(|v| v.get("error").and_then(|s| s.as_str()).map(|s| s.to_string()))
                            .unwrap_or(raw);
                        println!("  {} Nexus executor unavailable ({}), falling back to distributed", "!".yellow(), detail);
                        return execute_plan_distributed(&wp).await;
                    }
                };

                let execution_id = match dispatch_resp.get("execution_id").and_then(|v| v.as_str()) {
                    Some(id) => {
                        println!("{} Execution started: {}", "\u{2b21}".green(), id);
                        id.to_string()
                    }
                    None => {
                        println!("{} Execution dispatched (no execution_id): {:?}", "\u{2b21}".green(), dispatch_resp);
                        return Ok(());
                    }
                };

                // Poll for completion: 2s interval, 600s timeout, heartbeat every 30s
                let poll_interval = std::time::Duration::from_secs(2);
                let timeout = std::time::Duration::from_secs(600);
                let heartbeat_interval = std::time::Duration::from_secs(30);
                let start = std::time::Instant::now();
                let mut last_heartbeat = start;

                loop {
                    tokio::time::sleep(poll_interval).await;
                    let elapsed = start.elapsed();

                    if elapsed > timeout {
                        eprintln!("Workplan execution timed out after {}s", timeout.as_secs());
                        std::process::exit(1);
                    }

                    let status_path = format!("/api/workplan/{}", execution_id);
                    match client.get(&status_path).await {
                        Ok(resp) => {
                            let status = resp
                                .pointer("/data/status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");

                            if last_heartbeat.elapsed() >= heartbeat_interval {
                                println!("  {} [{}s] status: {}", "\u{2661}".dimmed(), elapsed.as_secs(), status);
                                last_heartbeat = std::time::Instant::now();
                            }

                            match status {
                                "completed" => {
                                    println!("{} Workplan completed ({}s)", "\u{2713}".green(), elapsed.as_secs());
                                    return Ok(());
                                }
                                "failed" => {
                                    let errors = resp
                                        .pointer("/data/errors")
                                        .and_then(|v| v.as_array())
                                        .map(|arr| arr.iter()
                                            .filter_map(|e| e.as_str())
                                            .collect::<Vec<_>>()
                                            .join("; "))
                                        .unwrap_or_default();
                                    eprintln!("Workplan failed ({}s): {}", elapsed.as_secs(), errors);
                                    std::process::exit(1);
                                }
                                _ => {} // running, paused — keep polling
                            }
                        }
                        Err(e) => {
                            if last_heartbeat.elapsed() >= heartbeat_interval {
                                println!("  {} [{}s] poll error: {}", "!".yellow(), elapsed.as_secs(), e);
                                last_heartbeat = std::time::Instant::now();
                            }
                        }
                    }
                }
            } else {
                // Standalone: create HexFlo tasks for remote workers
                println!("  {} Nexus connected — dispatching to remote workers", "\u{2713}".green());
                println!();
                return execute_plan_distributed(&wp).await;
            }
        }
        Err(_) => {
            // No nexus: local execution with Ollama
            let host = std::env::var("OLLAMA_HOST").unwrap_or_default();
            if host.is_empty() || host == "0.0.0.0" || host.starts_with("0.0.0.0:") {
                // Read Ollama host from inference config
                let cfg_host = dirs::home_dir()
                    .map(|h| h.join(".hex/inference-servers.json"))
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                    .and_then(|v| v["endpoints"].as_array().cloned())
                    .and_then(|eps| eps.iter()
                        .find(|e| e["provider"].as_str() == Some("ollama"))
                        .and_then(|e| e["url"].as_str().map(String::from)));
                std::env::set_var("OLLAMA_HOST",
                    cfg_host.as_deref().unwrap_or("http://localhost:11434"));
            }
            println!("  {} No nexus — executing locally with Ollama", "\u{2192}".dimmed());
            println!();
            return execute_plan_local(&path, &wp).await;
        }
    }

    Ok(())
}

/// Distributed workplan execution — creates HexFlo swarm tasks and waits
/// for remote workers to complete them (ADR-2604121630).
///
/// Flow: create swarm → create tasks per phase → poll until complete → run gates → next phase
/// Falls back to local execution if no workers are available.
async fn execute_plan_distributed(wp: &serde_json::Value) -> anyhow::Result<()> {
    let feature = wp.get("feature").and_then(|v| v.as_str()).unwrap_or("workplan");
    let phases = match wp.get("phases").and_then(|v| v.as_array()) {
        Some(p) => p,
        None => { anyhow::bail!("Workplan has no phases"); }
    };

    // NexusClient::from_env auto-resolves agent identity from session files + env
    let client = NexusClient::from_env();

    // Check if any workers are available before creating swarm
    let available_workers = match client.get("/api/hex-agents").await {
        Ok(resp) => {
            resp.as_array()
                .map(|agents| agents.iter()
                    .filter(|a| a["status"].as_str() == Some("active"))
                    .count())
                .unwrap_or(0)
        }
        Err(_) => 0,
    };

    if available_workers == 0 {
        println!("  {} No active workers available — falling back to local execution", "\u{2192}".dimmed());
        println!();
        return execute_plan_local(&std::path::Path::new(""), wp).await;
    }

    // Step 1: Create swarm for this execution
    let swarm_resp = client.post("/api/swarms", &serde_json::json!({
        "name": feature,
        "topology": "hierarchical",
        "projectId": feature,
    })).await?;

    let swarm_id = swarm_resp["id"].as_str().unwrap_or("").to_string();
    if swarm_id.is_empty() {
        anyhow::bail!("Failed to create swarm: {:?}", swarm_resp);
    }
    println!("{} Swarm created: {} ({})", "\u{2b21}".green(), feature, &swarm_id[..8]);

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;

    // Step 2: Execute phases sequentially
    for phase in phases {
        let phase_name = phase.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let tasks = phase.get("tasks").and_then(|v| v.as_array());
        let gate_cmd = phase.get("gate")
            .and_then(|g| g.get("command"))
            .and_then(|v| v.as_str());

        println!("{} Phase: {}", "\u{2501}".dimmed(), phase_name);

        let Some(tasks) = tasks else { continue };

        // Step 3: Create HexFlo tasks for this phase
        let mut task_ids: Vec<(String, String)> = Vec::new(); // (task_id, title)

        for task in tasks {
            let task_name = task.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let description = task.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let agent = task.get("agent").and_then(|v| v.as_str()).unwrap_or("hex-coder");
            let tier = task.get("tier").and_then(|v| v.as_str()).unwrap_or("T2");

            // Format title so worker can parse role + description
            let title = format!("{}: {}", agent, description);

            let resp = client.post(
                &format!("/api/swarms/{}/tasks", swarm_id),
                &serde_json::json!({ "title": title }),
            ).await;

            match resp {
                Ok(r) => {
                    let tid = r["id"].as_str().unwrap_or("").to_string();
                    if !tid.is_empty() {
                        println!("  {} [{}] {} → task {}", task_name, tier, agent, &tid[..8.min(tid.len())]);
                        task_ids.push((tid, task_name.to_string()));
                    } else {
                        println!("  {} [{}] {} → failed to create task", task_name, tier, agent);
                    }
                }
                Err(e) => {
                    println!("  {} Failed: {}", "!".red(), e);
                }
            }
        }

        if task_ids.is_empty() {
            println!("  {} No tasks created for phase", "!".yellow());
            continue;
        }

        // Step 4: Poll until all tasks complete (60s timeout per task)
        let timeout = std::time::Duration::from_secs(300); // 5 min per phase
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_secs(3);

        println!("  {} Waiting for {} worker(s)...", "\u{231b}".dimmed(), task_ids.len());

        loop {
            if start.elapsed() > timeout {
                println!("  {} Phase timed out after {}s", "!".red(), timeout.as_secs());
                total_failed += task_ids.len();
                break;
            }

            tokio::time::sleep(poll_interval).await;

            // Check task statuses
            let mut all_done = true;
            let mut phase_passed = 0;
            let mut phase_failed = 0;

            if let Ok(swarms_resp) = client.get("/api/swarms/active").await {
                if let Some(swarms) = swarms_resp.as_array() {
                    for swarm in swarms {
                        if swarm["id"].as_str() != Some(&swarm_id) { continue; }
                        if let Some(stasks) = swarm["tasks"].as_array() {
                            for (tid, _tname) in &task_ids {
                                if let Some(st) = stasks.iter().find(|t| t["id"].as_str() == Some(tid)) {
                                    match st["status"].as_str().unwrap_or("") {
                                        "completed" => { phase_passed += 1; }
                                        "failed" => { phase_failed += 1; }
                                        _ => { all_done = false; }
                                    }
                                } else {
                                    all_done = false;
                                }
                            }
                        }
                    }
                }
            }

            if all_done || (phase_passed + phase_failed == task_ids.len()) {
                for (tid, tname) in &task_ids {
                    // Find final status
                    let status = if let Ok(resp) = client.get("/api/swarms/active").await {
                        resp.as_array()
                            .and_then(|s| s.iter().find(|sw| sw["id"].as_str() == Some(&swarm_id)))
                            .and_then(|sw| sw["tasks"].as_array())
                            .and_then(|ts| ts.iter().find(|t| t["id"].as_str() == Some(tid.as_str())))
                            .and_then(|t| t["status"].as_str())
                            .unwrap_or("?")
                            .to_string()
                    } else { "?".to_string() };

                    if status == "completed" {
                        println!("  {} {}", "\u{2713}".green(), tname);
                        total_passed += 1;
                    } else {
                        println!("  {} {} ({})", "\u{2717}".red(), tname, status);
                        total_failed += 1;
                    }
                }
                break;
            }
        }

        // Step 5: Run phase gate
        if let Some(cmd) = gate_cmd {
            print!("  Phase gate: {} ... ", cmd);
            match run_gate(cmd).await {
                GateResult::Pass => println!("{}", "PASS".green()),
                GateResult::Fail(err) => {
                    println!("{}", "FAIL".red());
                    for line in err.lines().take(3) {
                        println!("    {}", line);
                    }
                }
            }
        }
        println!();
    }

    // Complete the swarm
    let _ = client.patch(
        &format!("/api/swarms/{}", swarm_id),
        &serde_json::json!({"status": "completed"}),
    ).await;

    println!("{} Results: {} passed, {} failed", "\u{2b21}".cyan(), total_passed, total_failed);
    println!("  Swarm: {} ({})", feature, &swarm_id[..8]);
    println!("  Workers assigned tasks automatically — use `hex task list` to review");

    if total_failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Local workplan execution fallback — runs when nexus is unavailable or no workers available.
/// Iterates phases sequentially, dispatches each task through Ollama,
/// runs compile gates, and records results.
/// ADR-005 6-gate pipeline: generate → compile → test → retry → escalate.
/// Max 5 iterations per task. Quality score must improve or escalate.
async fn execute_plan_local(_path: &std::path::Path, wp: &serde_json::Value) -> anyhow::Result<()> {
    let ollama_host = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());

    println!("{} Local execution with ADR-005 gate pipeline", "\u{2b21}".cyan());
    println!("  Ollama: {}", ollama_host);
    println!("  Gates:  compile → test → retry (max 5 iterations)");
    println!();

    let phases = match wp.get("phases").and_then(|v| v.as_array()) {
        Some(p) => p,
        None => { anyhow::bail!("Workplan has no phases"); }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let max_iterations = 5;

    for phase in phases {
        let phase_name = phase.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let tasks = phase.get("tasks").and_then(|v| v.as_array());
        let gate_cmd = phase.get("gate")
            .and_then(|g| g.get("command"))
            .and_then(|v| v.as_str());

        println!("{} Phase: {}", "\u{2501}".dimmed(), phase_name);

        if let Some(tasks) = tasks {
            for task in tasks {
                let task_id = task.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let task_name = task.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let description = task.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let tier = task.get("tier").and_then(|v| v.as_str()).unwrap_or("T2");
                let agent = task.get("agent").and_then(|v| v.as_str()).unwrap_or("hex-coder");
                let files: Vec<&str> = task.get("files")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();

                let model = match tier {
                    "T1" => "qwen3:4b",
                    "T2" => "qwen2.5-coder:32b",
                    "T2.5" => "qwen3.5:27b",
                    _ => "qwen2.5-coder:32b",
                };

                println!("  {} [{}] {} ({}, {})", task_id, tier, task_name, model, agent);

                let mut task_passed = false;
                let mut last_error = String::new();

                // ADR-005: iterate up to max_iterations with feedback
                for iteration in 1..=max_iterations {
                    if iteration > 1 {
                        println!("    {} Retry {}/{} with error feedback", "\u{21bb}".yellow(), iteration, max_iterations);
                    }

                    // Build prompt — append error feedback on retries
                    let prompt = if iteration == 1 {
                        description.to_string()
                    } else {
                        format!(
                            "{}\n\nThe previous attempt produced this error:\n```\n{}\n```\nFix ALL errors and return the COMPLETE corrected file.",
                            description,
                            last_error.chars().take(500).collect::<String>()
                        )
                    };

                    // Generate code via Ollama
                    let body = serde_json::json!({
                        "model": model,
                        "prompt": prompt,
                        "temperature": if iteration == 1 { 0.2 } else { 0.3 },
                        "stream": false,
                    });

                    let start = std::time::Instant::now();
                    let resp = client.post(format!("{}/api/generate", ollama_host))
                        .json(&body)
                        .send()
                        .await;

                    let (code, tokens) = match resp {
                        Ok(r) if r.status().is_success() => {
                            let json: serde_json::Value = r.json().await?;
                            let text = json.get("response").and_then(|v| v.as_str()).unwrap_or("");
                            let tokens = json.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0);
                            (extract_code_from_text(text), tokens)
                        }
                        Ok(r) => {
                            println!("    {} HTTP {} from Ollama", "!".red(), r.status());
                            last_error = format!("HTTP {}", r.status());
                            continue;
                        }
                        Err(e) => {
                            println!("    {} Ollama error: {}", "!".red(), e);
                            last_error = e.to_string();
                            continue;
                        }
                    };

                    let elapsed = start.elapsed();

                    // Write generated code to files
                    for target in &files {
                        if let Some(parent) = std::path::Path::new(target).parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        std::fs::write(target, &code)?;
                    }
                    let line_count = code.lines().count();

                    // === Gate 1: Compile ===
                    let compile_ok = if let Some(target) = files.first() {
                        let compile_cmd = if target.ends_with("main.rs") {
                            format!("rustc --edition 2021 {} -o /tmp/hex_gate_bin 2>&1", target)
                        } else {
                            format!("rustc --edition 2021 --crate-type lib {} 2>&1", target)
                        };
                        match run_gate(&compile_cmd).await {
                            GateResult::Pass => {
                                print!("    {} compile", "\u{2713}".green());
                                true
                            }
                            GateResult::Fail(err) => {
                                print!("    {} compile", "\u{2717}".red());
                                last_error = err;
                                false
                            }
                        }
                    } else { true };

                    // === Gate 2: Test (if file contains #[cfg(test)]) ===
                    let test_ok = if compile_ok && code.contains("#[cfg(test)]") {
                        if let Some(target) = files.first() {
                            let test_cmd = format!(
                                "rustc --edition 2021 --test {} -o /tmp/hex_gate_test 2>&1 && /tmp/hex_gate_test 2>&1",
                                target
                            );
                            match run_gate(&test_cmd).await {
                                GateResult::Pass => {
                                    print!(" {} test", "\u{2713}".green());
                                    true
                                }
                                GateResult::Fail(err) => {
                                    print!(" {} test", "\u{2717}".red());
                                    last_error = err;
                                    false
                                }
                            }
                        } else { true }
                    } else if compile_ok {
                        // No tests in file — that's a quality issue but not a gate failure
                        print!(" {} test(none)", "\u{26a0}".yellow());
                        true
                    } else { false };

                    println!(" | {} lines, {} tokens, {:.1}s", line_count, tokens, elapsed.as_secs_f64());

                    if compile_ok && test_ok {
                        task_passed = true;
                        break;
                    }

                    // ADR-005: if score stagnates for 2 iterations, escalate
                    if iteration >= max_iterations {
                        println!("    {} Max iterations reached — escalating", "!".red());
                    }
                }

                if task_passed {
                    total_passed += 1;
                } else {
                    println!("    {} Task failed after {} iterations", "!".red(), max_iterations);
                    total_failed += 1;
                }
            }
        }

        // Run phase gate (from workplan)
        if let Some(cmd) = gate_cmd {
            print!("  Phase gate: {} ... ", cmd);
            match run_gate(cmd).await {
                GateResult::Pass => println!("{}", "PASS".green()),
                GateResult::Fail(err) => {
                    println!("{}", "FAIL".red());
                    for line in err.lines().take(5) {
                        println!("    {}", line);
                    }
                }
            }
        }
        println!();
    }

    println!();
    println!("{} Results: {} passed, {} failed (ADR-005 gate pipeline)",
        "\u{2b21}".cyan(), total_passed, total_failed);
    // TODO: register execution in SpacetimeDB via nexus API so hex plan report works
    // Remote agents should write through the SSH tunnel to the coordinator's nexus.
    if total_failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

enum GateResult {
    Pass,
    Fail(String),
}

/// Run a shell command as a gate check. Returns Pass or Fail with stderr.
async fn run_gate(cmd: &str) -> GateResult {
    match tokio::process::Command::new("sh")
        .args(["-c", cmd])
        .output()
        .await
    {
        Ok(o) if o.status.success() => GateResult::Pass,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            let stdout = String::from_utf8_lossy(&o.stdout).to_string();
            GateResult::Fail(if stderr.is_empty() { stdout } else { stderr })
        }
        Err(e) => GateResult::Fail(format!("gate command failed: {}", e)),
    }
}

/// Extract code from fenced blocks or return raw text.
fn extract_code_from_text(text: &str) -> String {
    // Try ```rust fences first
    if let Some(start) = text.find("```rust") {
        let after = &text[start + 7..];
        if let Some(nl) = after.find('\n') {
            let code_start = &after[nl + 1..];
            if let Some(end) = code_start.find("```") {
                return code_start[..end].to_string();
            }
        }
    }
    // Try generic ``` fences
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(nl) = after.find('\n') {
            let code_start = &after[nl + 1..];
            if let Some(end) = code_start.find("```") {
                return code_start[..end].to_string();
            }
        }
    }
    text.to_string()
}

/// Decompose requirements into hex-bounded tasks by tier.
async fn create_plan(requirements: &[String], lang: &str, adr: Option<&str>, no_adr: bool) -> anyhow::Result<()> {
    // ADR-050: Validate ADR reference exists before creating workplan
    if !no_adr {
        match adr {
            None => {
                anyhow::bail!(
                    "Workplan requires an ADR reference. Use --adr ADR-NNN or --no-adr to skip.\n\
                     Pipeline: ADR → Workplan → HexFlo Memory → Swarm"
                );
            }
            Some(adr_ref) => {
                let adr_dir = Path::new("docs/adrs");
                if adr_dir.is_dir() {
                    let adr_slug = adr_ref.to_uppercase().replace(' ', "-");
                    let found = std::fs::read_dir(adr_dir)?
                        .filter_map(|e| e.ok())
                        .any(|e| {
                            let name = e.file_name().to_string_lossy().to_uppercase();
                            name.contains(&adr_slug)
                        });
                    if !found {
                        anyhow::bail!(
                            "ADR '{}' not found in docs/adrs/. Create the ADR first.\n\
                             Pipeline: ADR → Workplan → HexFlo Memory → Swarm",
                            adr_ref
                        );
                    }
                    println!("  {} ADR {} verified", "\u{2713}".green(), adr_ref);
                }
            }
        }
    }

    println!(
        "{} Creating workplan ({} requirement(s), language: {})",
        "\u{2b21}".cyan(),
        requirements.len(),
        lang,
    );
    println!();

    // Try nexus first for richer planning
    let nexus = NexusClient::from_env();
    if nexus.ensure_running().await.is_ok() {
        let body = serde_json::json!({
            "requirements": requirements,
            "language": lang,
        });
        match nexus.post("/api/workplan/execute", &body).await {
            Ok(data) => {
                println!("{}", serde_json::to_string_pretty(&data)?);
                return Ok(());
            }
            Err(_) => {
                // Fall through to structural decomposition
            }
        }
    }

    // Structural decomposition — no LLM needed
    let mut steps: Vec<Step> = Vec::new();

    for (i, req) in requirements.iter().enumerate() {
        let adapter = infer_adapter(req);
        let tier = infer_tier(&adapter);
        let deps = if tier > 0 { vec!["ports".to_string()] } else { vec![] };

        steps.push(Step {
            id: format!("step-{}", i + 1),
            description: req.clone(),
            adapter: adapter.clone(),
            tier,
            dependencies: deps,
            status: "pending".to_string(),
            done_condition: String::new(),
            verify: String::new(),
            files: Vec::new(),
            done_command: String::new(),
        });
    }

    // Sort by tier
    steps.sort_by_key(|s| s.tier);

    // Print the plan
    println!("  {}", "WORKPLAN".bold());
    println!("  Language: {} | Steps: {}", lang, steps.len());
    println!();

    let tier_names = [
        "Tier 0 (domain + ports)",
        "Tier 1 (secondary adapters)",
        "Tier 2 (primary adapters)",
        "Tier 3 (usecases + wiring)",
        "Tier 4 (tests)",
    ];

    for tier in 0..=4u8 {
        let tier_steps: Vec<&Step> = steps.iter().filter(|s| s.tier == tier).collect();
        if tier_steps.is_empty() {
            continue;
        }
        println!("  {}:", tier_names[tier as usize].bold());
        for s in &tier_steps {
            println!(
                "    {} [{}] {} {} {}",
                "\u{25cb}".dimmed(),
                s.id,
                s.description,
                "\u{2192}".dimmed(),
                s.adapter.dimmed(),
            );
        }
        println!();
    }

    println!("  {}", "DEPENDENCY ORDER".bold());
    println!("  Tier 0: domain + ports (no deps)");
    println!("  Tier 1: secondary adapters (depend on ports)");
    println!("  Tier 2: primary adapters (depend on ports)");
    println!("  Tier 3: usecases + composition root (depend on tiers 0-2)");
    println!("  Tier 4: integration tests (depend on everything)");
    println!();
    println!(
        "  {} Tiers 1 and 2 can run in parallel.",
        "\u{2192}".dimmed()
    );

    // Save to docs/workplans/
    let workplans_dir = Path::new("docs/workplans");
    if workplans_dir.is_dir() {
        let slug: String = requirements
            .first()
            .map(|r| {
                r.to_lowercase()
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .collect::<String>()
                    .trim_matches('-')
                    .to_string()
            })
            .unwrap_or_else(|| "plan".to_string());
        let filename = format!("feat-{}.json", &slug[..slug.len().min(40)]);
        let path = workplans_dir.join(&filename);

        let plan = Workplan {
            title: format!("Plan: {}", requirements.join(", ")),
            feature: String::new(),
            language: lang.to_string(),
            status: "planned".to_string(),
            steps,
            phases: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            adr: String::new(),
            description: String::new(),
            priority: String::new(),
            superseded_by: String::new(),
        };

        let json = serde_json::to_string_pretty(&plan)?;
        std::fs::write(&path, &json)?;
        println!(
            "  {} Saved to {}",
            "\u{2713}".green(),
            path.display()
        );
    }

    Ok(())
}

/// List workplans from docs/workplans/.
async fn list_plans() -> anyhow::Result<()> {
    let dir = Path::new("docs/workplans");
    if !dir.is_dir() {
        println!("No workplans directory found (docs/workplans/)");
        return Ok(());
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "json" || ext == "md")
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        println!("No workplans found in docs/workplans/");
        return Ok(());
    }

    println!(
        "{} {} workplan(s) in docs/workplans/",
        "\u{2b21}".cyan(),
        entries.len()
    );
    println!();

    let nexus = NexusClient::from_env();
    let nexus_available = nexus.ensure_running().await.is_ok();

    let mut rows: Vec<PlanRow> = Vec::new();

    for entry in &entries {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();

        if path.extension().map(|e| e == "json").unwrap_or(false) {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    if let Ok(plan) = serde_json::from_str::<Workplan>(&contents) {
                        let total = plan.total_tasks();
                        let done = plan.completed_tasks();
                        let title = {
                            let dt = plan.display_title();
                            if dt.is_empty() { name.to_string() } else { truncate(dt, 40) }
                        };

                        // Try to fetch live execution overlay from nexus.
                        let live_badge = if nexus_available {
                            let api_path = format!("/api/workplan/by-path?path={}", name);
                            nexus.get(&api_path).await.ok().and_then(|v| {
                                let exec_status = v.get("status")?.as_str()?.to_string();
                                let exec_done = v.get("completed_tasks")?.as_u64()? as u32;
                                let exec_total = v.get("total_tasks")?.as_u64()? as u32;
                                Some((exec_status, exec_done, exec_total))
                            })
                        } else {
                            None
                        };

                        let progress_str = if let Some((ref exec_status, exec_done, exec_total)) = live_badge {
                            let base = if exec_total == 0 {
                                "(no tasks)".dimmed().to_string()
                            } else {
                                progress(exec_done, exec_total)
                            };
                            format!("{} [{}]", base, format!("{} {}/{}", exec_status, exec_done, exec_total).cyan())
                        } else if total == 0 {
                            "(no tasks)".dimmed().to_string()
                        } else {
                            progress(done as u32, total as u32)
                        };

                        let adr_display = if plan.adr.is_empty() {
                            "\u{2014}".dimmed().to_string()
                        } else {
                            plan.adr.clone()
                        };

                        let priority_display = if plan.priority.is_empty() {
                            String::new()
                        } else {
                            plan.priority.red().to_string()
                        };

                        let status_display = if plan.status.is_empty() {
                            "\u{2014}".dimmed().to_string()
                        } else {
                            status_badge(&plan.status)
                        };

                        rows.push(PlanRow {
                            status: status_display,
                            title,
                            adr: adr_display,
                            progress: progress_str,
                            priority: priority_display,
                        });
                    } else {
                        rows.push(PlanRow {
                            status: "\u{25cb}".dimmed().to_string(),
                            title: format!("{} (parse error)", name),
                            adr: String::new(),
                            progress: String::new(),
                            priority: String::new(),
                        });
                    }
                }
                Err(_) => {
                    rows.push(PlanRow {
                        status: "\u{25cb}".dimmed().to_string(),
                        title: format!("{} (read error)", name),
                        adr: String::new(),
                        progress: String::new(),
                        priority: String::new(),
                    });
                }
            }
        } else {
            rows.push(PlanRow {
                status: "\u{25cb}".dimmed().to_string(),
                title: format!("{} (markdown)", name),
                adr: String::new(),
                progress: String::new(),
                priority: String::new(),
            });
        }
    }

    println!("{}", HexTable::render(&rows));

    Ok(())
}

/// Show detailed status of a workplan.
async fn show_plan_status(file: &str) -> anyhow::Result<()> {
    let path = Path::new("docs/workplans").join(file);
    if !path.exists() {
        // Try adding .json
        let with_ext = Path::new("docs/workplans").join(format!("{}.json", file));
        if with_ext.exists() {
            return show_plan_file(&with_ext).await;
        }
        anyhow::bail!("Workplan not found: {}", path.display());
    }
    show_plan_file(&path).await
}

async fn show_plan_file(path: &Path) -> anyhow::Result<()> {
    let contents = std::fs::read_to_string(path)?;
    let plan: Workplan = serde_json::from_str(&contents)?;

    let display_title = if !plan.feature.is_empty() {
        plan.feature.clone()
    } else if !plan.title.is_empty() {
        plan.title.clone()
    } else {
        path.file_name().unwrap().to_string_lossy().to_string()
    };

    println!("{} {}", "\u{2b21}".cyan(), display_title);
    if !plan.language.is_empty() {
        println!("  Language: {}", plan.language);
    }
    if !plan.created_at.is_empty() {
        println!("  Created: {}", plan.created_at);
    }

    let total = plan.total_tasks();
    let done = plan.completed_tasks();
    println!("  Tasks: {} ({} done)", total, done);
    println!();

    if !plan.phases.is_empty() {
        // Current format: phases → tasks
        for phase in &plan.phases {
            println!("  {} {}", "\u{25b6}".cyan(), if phase.name.is_empty() { &phase.id } else { &phase.name });
            for task in &phase.tasks {
                let git = git_evidence_label(&task.files, &plan.created_at);
                let verified = if !task.done_command.is_empty() && run_done_command(&task.done_command) {
                    " [verified]".green().to_string()
                } else {
                    String::new()
                };
                println!(
                    "    {} {:<10} {:<40} {} {}",
                    status_badge(&task.status),
                    task.id,
                    truncate(&task.name, 40),
                    git,
                    verified,
                );
            }
            println!();
        }
    } else if !plan.steps.is_empty() {
        // Legacy format: steps
        let rows: Vec<StepRow> = plan.steps.iter().map(|step| {
            let deps = if step.dependencies.is_empty() {
                "\u{2014}".dimmed().to_string()
            } else {
                step.dependencies.join(", ")
            };
            let git = git_evidence_label(&step.files, &plan.created_at);
            let verified = if !step.done_command.is_empty() && run_done_command(&step.done_command) {
                " [verified]".green().to_string()
            } else if !step.verify.is_empty() && run_done_command(&step.verify) {
                " [verified]".green().to_string()
            } else {
                String::new()
            };
            StepRow {
                id: step.id.clone(),
                description: truncate(&step.description, 50),
                adapter: step.adapter.clone(),
                tier: step.tier.to_string(),
                status: status_badge(&step.status),
                git_evidence: format!("{}{}", git, verified),
                deps,
            }
        }).collect();

        println!("{}", HexTable::render(&rows));
    } else {
        println!("  (no tasks defined)");
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// WORKPLAN EXECUTION COMMANDS (ADR-046)
// ═══════════════════════════════════════════════════════════

/// Show currently active (running/paused) workplan executions via nexus.
async fn show_active_executions() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let data = nexus.get("/api/workplan/list").await?;
    let executions = data["data"]["executions"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let active: Vec<_> = executions
        .iter()
        .filter(|e| {
            let status = e["status"].as_str().unwrap_or("");
            status == "running" || status == "paused"
        })
        .collect();

    if active.is_empty() {
        println!("No active workplan executions.");
        return Ok(());
    }

    println!(
        "{} {} active workplan execution(s)",
        "\u{2b21}".cyan(),
        active.len()
    );
    println!();

    let rows: Vec<ExecutionRow> = active.iter().map(|exec| {
        let id = exec["id"].as_str().unwrap_or("?");
        let status = exec["status"].as_str().unwrap_or("?");
        let feature_val = exec["feature"].as_str().unwrap_or("");
        let phase = exec["currentPhase"].as_str().unwrap_or("?");
        let completed = exec["completedPhases"].as_u64().unwrap_or(0);
        let total = exec["totalPhases"].as_u64().unwrap_or(0);

        let label = if feature_val.is_empty() {
            id.to_string()
        } else {
            format!("{} ({})", feature_val, &id[..8.min(id.len())])
        };

        ExecutionRow {
            status: status_badge(status),
            feature: label,
            phase: phase.to_string(),
            progress_col: progress(completed as u32, total as u32),
        }
    }).collect();

    println!("{}", HexTable::render(&rows));

    Ok(())
}

/// Show all past workplan executions (history) via nexus.
async fn show_execution_history() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let data = nexus.get("/api/workplan/list").await?;
    let executions = data["data"]["executions"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    if executions.is_empty() {
        println!("No workplan executions found.");
        return Ok(());
    }

    let total = data["data"]["total"].as_u64().unwrap_or(0);
    let active = data["data"]["activeCount"].as_u64().unwrap_or(0);

    println!(
        "{} {} workplan execution(s) ({} active, {} completed)",
        "\u{2b21}".cyan(),
        total,
        active,
        total.saturating_sub(active),
    );
    println!();

    let rows: Vec<HistoryRow> = executions.iter().map(|exec| {
        let id = exec["id"].as_str().unwrap_or("?");
        let status = exec["status"].as_str().unwrap_or("?");
        let feature_val = exec["feature"].as_str().unwrap_or("");
        let started = exec["startedAt"].as_str().unwrap_or("?");
        let tasks_done = exec["completedTasks"].as_u64().unwrap_or(0);
        let tasks_total = exec["totalTasks"].as_u64().unwrap_or(0);

        let label = if feature_val.is_empty() {
            id[..8.min(id.len())].to_string()
        } else {
            feature_val.to_string()
        };

        HistoryRow {
            status: status_badge(status),
            feature: label,
            tasks: progress(tasks_done as u32, tasks_total as u32),
            started: started.to_string(),
            id: id.dimmed().to_string(),
        }
    }).collect();

    println!("{}", HexTable::render(&rows));

    Ok(())
}

/// Show aggregate report for a workplan execution, including git correlation.
async fn show_execution_report(id: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let data = nexus.get(&format!("/api/workplan/{}/report", id)).await?;

    if let Some(error) = data["error"].as_str() {
        anyhow::bail!("{}", error);
    }

    let report = &data["data"];
    let workplan = &report["workplan"];
    let summary = &report["summary"];

    // Header
    let feature = workplan["feature"].as_str().unwrap_or("(unnamed)");
    let status = workplan["status"].as_str().unwrap_or("?");
    println!(
        "{} Workplan Report: {} [{}]",
        "\u{2b21}".cyan(),
        feature.bold(),
        status
    );
    println!("  ID: {}", workplan["id"].as_str().unwrap_or("?"));
    println!("  Path: {}", workplan["workplanPath"].as_str().unwrap_or("?"));
    println!(
        "  Started: {} | Updated: {}",
        workplan["startedAt"].as_str().unwrap_or("?"),
        workplan["updatedAt"].as_str().unwrap_or("?"),
    );
    println!();

    // Summary
    println!("  {}", "SUMMARY".bold());
    if let Some(dur) = summary["durationMinutes"].as_i64() {
        println!("    Duration: {} min", dur);
    }
    println!(
        "    Phases:  {}/{}",
        summary["phasesCompleted"].as_u64().unwrap_or(0),
        summary["phasesTotal"].as_u64().unwrap_or(0),
    );
    println!(
        "    Tasks:   {}/{} ({} failed)",
        summary["tasksCompleted"].as_u64().unwrap_or(0),
        summary["tasksTotal"].as_u64().unwrap_or(0),
        summary["tasksFailed"].as_u64().unwrap_or(0),
    );
    println!(
        "    Gates:   {} passed, {} failed",
        summary["gatesPassed"].as_u64().unwrap_or(0),
        summary["gatesFailed"].as_u64().unwrap_or(0),
    );
    if let Some(agents) = summary["agentsUsed"].as_array() {
        if !agents.is_empty() {
            let names: Vec<_> = agents.iter().filter_map(|a| a.as_str()).collect();
            println!("    Agents:  {}", names.join(", "));
        }
    }
    println!();

    // Phase results
    if let Some(phases) = report["phases"].as_array() {
        if !phases.is_empty() {
            println!("  {}", "PHASES".bold());
            for p in phases {
                let name = p["phase"].as_str().unwrap_or("?");
                let pstatus = p["status"].as_str().unwrap_or("?");
                let icon = match pstatus {
                    "completed" => "\u{2713}".green(),
                    "failed" => "\u{2717}".red(),
                    _ => "\u{25cb}".dimmed(),
                };
                println!("    {} {} [{}]", icon, name, pstatus);
                if let Some(errs) = p["errors"].as_array() {
                    for err in errs {
                        if let Some(e) = err.as_str() {
                            println!("      {} {}", "\u{2717}".red(), e);
                        }
                    }
                }
            }
            println!();
        }
    }

    // Gate results
    if let Some(gates) = report["gates"].as_array() {
        if !gates.is_empty() {
            println!("  {}", "GATES".bold());
            for g in gates {
                let phase = g["phase"].as_str().unwrap_or("?");
                let cmd = g["gateCommand"].as_str().unwrap_or("?");
                let passed = g["passed"].as_bool().unwrap_or(false);
                let icon = if passed {
                    "\u{2713}".green()
                } else {
                    "\u{2717}".red()
                };
                println!("    {} {} ({})", icon, phase, cmd.dimmed());
            }
            println!();
        }
    }

    // Git correlation (ADR-046)
    if let Some(commits) = report["commits"].as_array() {
        if !commits.is_empty() {
            println!("  {}", "GIT COMMITS".bold());
            for c in commits {
                let sha = c["commitShort"].as_str().unwrap_or("?");
                let msg = c["commitMessage"].as_str().unwrap_or("?");
                let author = c["author"].as_str().unwrap_or("?");
                println!("    {} {} — {} ({})", sha.yellow(), msg, author.dimmed(),
                    c["agentName"].as_str().unwrap_or("manual").dimmed());
            }
            println!();
        }
    }

    Ok(())
}

/// Output the canonical workplan JSON schema.
async fn show_schema() -> anyhow::Result<()> {
    let schema = crate::assets::Assets::get_str("schemas/workplan.schema.json")
        .ok_or_else(|| anyhow::anyhow!("Workplan schema not found in embedded assets"))?;
    print!("{}", schema);
    Ok(())
}

/// Infer which adapter boundary a requirement targets.
fn infer_adapter(req: &str) -> String {
    let lower = req.to_lowercase();
    if lower.contains("http") || lower.contains("api") || lower.contains("rest") || lower.contains("server") {
        "primary/http-adapter".to_string()
    } else if lower.contains("cli") || lower.contains("command") {
        "primary/cli-adapter".to_string()
    } else if lower.contains("browser") || lower.contains("ui") || lower.contains("display") || lower.contains("canvas") {
        "primary/browser-adapter".to_string()
    } else if lower.contains("websocket") || lower.contains("ws") {
        "primary/ws-adapter".to_string()
    } else if lower.contains("sqlite") || lower.contains("database") || lower.contains("db") || lower.contains("storage") || lower.contains("persist") {
        "secondary/storage-adapter".to_string()
    } else if lower.contains("redis") || lower.contains("cache") {
        "secondary/cache-adapter".to_string()
    } else if lower.contains("auth") || lower.contains("jwt") || lower.contains("token") {
        "secondary/auth-adapter".to_string()
    } else if lower.contains("email") || lower.contains("notification") || lower.contains("notify") {
        "secondary/notification-adapter".to_string()
    } else if lower.contains("file") || lower.contains("fs") {
        "secondary/filesystem-adapter".to_string()
    } else if lower.contains("test") {
        "tests/unit".to_string()
    } else {
        "core/domain".to_string()
    }
}

// ── Git Evidence ──────────────────────────────────────────────────────

/// Check if a file has git commits since `since` (ISO-8601 date string).
/// Returns true if `git log` finds at least one commit touching the file.
fn file_has_git_evidence(file: &str, since: &str) -> bool {
    if since.is_empty() || file.is_empty() {
        return false;
    }
    let output = std::process::Command::new("git")
        .args(["log", "--oneline", &format!("--since={}", since), "--", file])
        .output();
    match output {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Check if a task ID appears in recent git commit messages, scoped to a workplan.
/// Matches patterns like "p1.1", "P1.1", "(p1.1)", "p1.1:" in commit subjects.
/// When `adr_scope` is non-empty, the commit must ALSO contain the ADR id — this
/// prevents cross-workplan false positives where generic task IDs like "P1.1"
/// appear in unrelated commits. When `adr_scope` is empty, falls back to the
/// legacy unscoped match (for workplans without an ADR link).
/// Uses --fixed-strings to avoid regex interpretation of dots in task IDs.
/// Uses a 24h buffer before created_at to account for timezone differences.
fn task_id_in_git_log(task_id: &str, since: &str, adr_scope: &str) -> bool {
    if task_id.is_empty() {
        return false;
    }
    // Use 24h before created_at to account for UTC vs local timezone drift.
    // If no created_at, search last 7 days.
    let since_arg = if since.is_empty() {
        "--since=7.days".to_string()
    } else {
        // Parse and subtract 1 day for buffer
        let buffered = since
            .parse::<chrono::DateTime<chrono::Utc>>()
            .map(|dt| (dt - chrono::Duration::hours(24)).to_rfc3339())
            .unwrap_or_else(|_| since.to_string());
        format!("--since={}", buffered)
    };
    let mut args: Vec<String> = vec![
        "log".to_string(),
        "--oneline".to_string(),
        since_arg,
        "--fixed-strings".to_string(),
        "-i".to_string(),
        "--grep".to_string(),
        task_id.to_string(),
    ];
    if !adr_scope.is_empty() {
        // Require BOTH task_id AND adr_scope to appear — prevents unrelated
        // commits that happen to share a generic task ID (P1.1, P2.1) from
        // matching a different workplan.
        args.push("--all-match".to_string());
        args.push("--grep".to_string());
        args.push(adr_scope.to_string());
    }
    let output = std::process::Command::new("git").args(&args).output();
    match output {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Check if any commit since `since` modified `file` AND mentions `adr_scope` in
/// its message. This narrows the plain "file was modified" heuristic so a commit
/// on another workplan that happens to touch the same file doesn't register as
/// evidence for an unrelated task. When `adr_scope` is empty, falls back to the
/// legacy unscoped file-modified check.
fn file_has_scoped_git_evidence(file: &str, since: &str, adr_scope: &str) -> bool {
    if adr_scope.is_empty() {
        return file_has_git_evidence(file, since);
    }
    if since.is_empty() || file.is_empty() {
        return false;
    }
    let output = std::process::Command::new("git")
        .args([
            "log",
            "--oneline",
            &format!("--since={}", since),
            "--fixed-strings",
            "--grep",
            adr_scope,
            "--",
            file,
        ])
        .output();
    match output {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => false,
    }
}

/// Check git evidence for a list of files. Returns a summary string.
/// - All files modified: "[git: modified]" (green)
/// - Some files modified: "[git: partial N/M]" (yellow)
/// - No files / no created_at: "" (empty)
pub(super) fn git_evidence_label(files: &[String], created_at: &str) -> String {
    if files.is_empty() || created_at.is_empty() {
        return String::new();
    }
    let modified_count = files.iter().filter(|f| file_has_git_evidence(f, created_at)).count();
    if modified_count == files.len() {
        "[git: modified]".green().to_string()
    } else if modified_count > 0 {
        format!("[git: {}/{}]", modified_count, files.len()).yellow().to_string()
    } else {
        String::new()
    }
}

/// Run a done_command and return true if exit code is 0.
fn run_done_command(cmd: &str) -> bool {
    if cmd.is_empty() {
        return false;
    }
    std::process::Command::new("sh")
        .args(["-c", cmd])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// Reconcile logic extracted to reconcile.rs (ADR-2604142200).

/// Extract identifiers worth grepping from a done_condition string.
/// Takes snake_case/camelCase words ≥5 chars and single-quoted strings.
fn extract_identifiers(condition: &str) -> Vec<String> {
    let mut ids = Vec::new();

    // Single-quoted strings like 'sonnet', 'prior_errors'
    let mut in_quote = false;
    let mut current = String::new();
    for ch in condition.chars() {
        if ch == '\'' {
            if in_quote && !current.is_empty() {
                ids.push(current.clone());
                current.clear();
            }
            in_quote = !in_quote;
        } else if in_quote {
            current.push(ch);
        }
    }
    // Word tokens: snake_case or CamelCase, ≥5 chars, not common prose words
    let skip = ["cargo", "check", "build", "passes", "returns", "reads", "files", "calls",
                "found", "added", "output", "result", "using", "value", "field", "never",
                "always", "every", "should", "where", "which", "other", "after", "first",
                "then", "from", "with", "into", "that", "this", "have", "does", "when"];
    for word in condition.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let w = word.trim();
        if w.len() >= 5 && (w.contains('_') || w.chars().any(|c| c.is_uppercase()))
            && !skip.iter().any(|s| w.to_lowercase() == *s) {
                ids.push(w.to_string());
            }
    }

    ids.sort();
    ids.dedup();
    ids
}

/// Grep for each identifier in the project source. Returns true if ≥1 found.
fn check_identifiers(identifiers: &[String]) -> bool {
    if identifiers.is_empty() { return false; }
    for id in identifiers {
        let output = std::process::Command::new("grep")
            .args(["-r", "--include=*.rs", "-l", id.as_str(), "hex-cli/src", "hex-nexus/src", "hex-core/src"])
            .output();
        if let Ok(out) = output {
            if !out.stdout.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Run cargo check/build to verify compilation. Returns true if passes.
fn check_cargo(condition: &str) -> bool {
    let (cmd, pkg) = if condition.contains("cargo test") {
        ("test", extract_cargo_pkg(condition))
    } else if condition.contains("cargo build") {
        ("build", extract_cargo_pkg(condition))
    } else {
        ("check", extract_cargo_pkg(condition))
    };

    let mut args = vec![cmd];
    if let Some(pkg) = pkg.as_deref() {
        args.push("-p");
        args.push(pkg);
    }

    std::process::Command::new("cargo")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn extract_cargo_pkg(condition: &str) -> Option<String> {
    // Match "-p hex-cli" or "-p hex-nexus" patterns in condition text
    if condition.contains("hex-cli") { return Some("hex-cli".to_string()); }
    if condition.contains("hex-nexus") { return Some("hex-nexus".to_string()); }
    if condition.contains("hex-core") { return Some("hex-core".to_string()); }
    None
}

/// Map adapter path to dependency tier.
fn infer_tier(adapter: &str) -> u8 {
    if adapter.contains("test") {
        4
    } else if adapter.starts_with("primary/") {
        2
    } else if adapter.starts_with("secondary/") {
        1
    } else if adapter.contains("usecase") || adapter.contains("composition") {
        3
    } else {
        0 // domain + ports
    }
}

// ── ADR-2604110227: Draft workplans ──────────────────────────────────

/// Directory where auto-invoked draft workplans are quarantined until
/// the user approves, edits, or clears them.
fn drafts_dir() -> std::path::PathBuf {
    Path::new("docs/workplans/drafts").to_path_buf()
}

/// Derive a short slug from a user prompt for filename purposes.
/// E.g. "implement OAuth login with refresh tokens" → "implement-oauth-login".
fn slug_from_prompt(prompt: &str) -> String {
    let lower = prompt.to_lowercase();
    let mut slug: String = lower
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        slug = "unnamed".to_string();
    }
    if slug.len() > 48 {
        slug.truncate(48);
    }
    slug
}

/// Create a draft workplan stub from a user prompt.
///
/// The stub is a minimal JSON document capturing the original prompt,
/// the tier classification, and a "pending-planner" status. The hook
/// router auto-invokes this on T3-sized prompts; the draft surfaces in
/// Claude Code context so the agent can pick it up via /hex-feature-dev
/// (or the user can approve/edit it manually).
///
/// This function deliberately does NOT spawn Claude subagents directly —
/// that happens upstream in Claude Code when it reads the banner and
/// notices the draft file. Keeping the spawn visible preserves the ADR
/// guarantee that auto-invocation never creates worktrees or dispatches
/// coders without user review.
async fn draft_plan(prompt_parts: &[String], background: bool) -> anyhow::Result<()> {
    use std::io::Write;

    // Respect HEX_AUTO_PLAN=0 opt-out even on direct invocation
    if std::env::var("HEX_AUTO_PLAN").ok().as_deref() == Some("0") {
        if !background {
            eprintln!("hex plan draft: disabled via HEX_AUTO_PLAN=0");
        }
        return Ok(());
    }

    let prompt = prompt_parts.join(" ");
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        anyhow::bail!("hex plan draft: prompt is empty");
    }

    // Ensure drafts dir exists
    let dir = drafts_dir();
    std::fs::create_dir_all(&dir)?;

    // Build a timestamped filename: draft-YYMMDDHHMM-<slug>.json
    let ts = chrono::Local::now().format("%y%m%d%H%M").to_string();
    let slug = slug_from_prompt(trimmed);
    let filename = format!("draft-{}-{}.json", ts, slug);
    let path = dir.join(&filename);

    // Draft stub: captures prompt, tier, origin, and pending status.
    // Deliberately minimal — the planner agent will expand this into a
    // full workplan when the user (or Claude Code) picks it up.
    let draft_id = format!("draft-{}-{}", ts, slug);
    let draft = serde_json::json!({
        "id": draft_id,
        "kind": "workplan-draft",
        "status": "pending-planner",
        "adr": "ADR-2604110227",
        "created_at": chrono::Local::now().to_rfc3339(),
        "origin": "auto-invoke",
        "prompt": trimmed,
        "next_steps": [
            "Run /hex-feature-dev to expand this draft into a full workplan",
            format!("Or run `hex plan drafts approve {}`", filename),
            format!("Or run `hex plan drafts clear --name {}`", filename.trim_end_matches(".json")),
        ],
        "notes": "This is a draft created by ADR-2604110227 auto-invoke. It contains only the original prompt — no specs, steps, or tiers have been generated yet. The planner agent will fill these in when the draft is picked up."
    });

    let mut file = std::fs::File::create(&path)?;
    file.write_all(serde_json::to_string_pretty(&draft)?.as_bytes())?;

    if !background {
        println!(
            "{} draft created: {}",
            "\u{2713}".green(),
            path.display().to_string().cyan()
        );
        println!("  {} run `/hex-feature-dev` to expand into a full workplan", "\u{2192}".dimmed());
        println!("  {} run `hex plan drafts list` to see all drafts", "\u{2192}".dimmed());
    }

    Ok(())
}

async fn drafts_dispatch(action: DraftsAction) -> anyhow::Result<()> {
    match action {
        DraftsAction::List => list_drafts().await,
        DraftsAction::Clear { name } => clear_drafts(name.as_deref()).await,
        DraftsAction::Approve { name } => approve_draft(&name).await,
        DraftsAction::Gc { days } => gc_drafts(days).await,
    }
}

async fn list_drafts() -> anyhow::Result<()> {
    let dir = drafts_dir();
    if !dir.is_dir() {
        println!("No drafts directory (nothing to list).");
        return Ok(());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().and_then(|s| s.to_str()) == Some("json")
        })
        .collect();

    if entries.is_empty() {
        println!("No draft workplans.");
        return Ok(());
    }

    // Sort newest first
    entries.sort_by(|a, b| {
        b.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            .cmp(
                &a.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            )
    });

    println!("{}", "Draft workplans:".bold());
    for e in &entries {
        let path = e.path();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let age = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|d| format!("{}h ago", d.as_secs() / 3600))
            .unwrap_or_else(|| "?".to_string());
        // Try to read the prompt
        let prompt_snippet = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
            .and_then(|v| v["prompt"].as_str().map(|s| truncate(s, 60)))
            .unwrap_or_else(|| "?".to_string());
        println!(
            "  {} {} {}",
            age.dimmed(),
            name.cyan(),
            prompt_snippet
        );
    }

    println!();
    println!(
        "  {} {} to promote a draft to a real workplan",
        "\u{2192}".dimmed(),
        "hex plan drafts approve <name>".white()
    );
    println!(
        "  {} {} to delete all drafts",
        "\u{2192}".dimmed(),
        "hex plan drafts clear".white()
    );

    Ok(())
}

async fn clear_drafts(name: Option<&str>) -> anyhow::Result<()> {
    let dir = drafts_dir();
    if !dir.is_dir() {
        println!("No drafts directory (nothing to clear).");
        return Ok(());
    }

    if let Some(n) = name {
        let filename = if n.ends_with(".json") {
            n.to_string()
        } else {
            format!("{}.json", n)
        };
        let path = dir.join(&filename);
        if !path.exists() {
            anyhow::bail!("draft not found: {}", path.display());
        }
        std::fs::remove_file(&path)?;
        println!("{} removed {}", "\u{2713}".green(), filename.cyan());
        return Ok(());
    }

    // Clear all
    let mut count = 0;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
            std::fs::remove_file(entry.path())?;
            count += 1;
        }
    }
    println!("{} removed {} draft(s)", "\u{2713}".green(), count);
    Ok(())
}

async fn approve_draft(name: &str) -> anyhow::Result<()> {
    let filename = if name.ends_with(".json") {
        name.to_string()
    } else {
        format!("{}.json", name)
    };
    let src = drafts_dir().join(&filename);
    if !src.exists() {
        anyhow::bail!("draft not found: {}", src.display());
    }

    // Promote to docs/workplans/ with an unambiguous "approved-" prefix
    // so the user can see it was auto-generated and rename it if they want.
    let dst_name = if filename.starts_with("draft-") {
        filename.replacen("draft-", "approved-", 1)
    } else {
        format!("approved-{}", filename)
    };
    let dst = Path::new("docs/workplans").join(&dst_name);

    std::fs::create_dir_all("docs/workplans")?;
    std::fs::rename(&src, &dst)?;

    println!(
        "{} approved: {} {} {}",
        "\u{2713}".green(),
        filename.dimmed(),
        "\u{2192}".dimmed(),
        dst.display().to_string().cyan()
    );
    println!(
        "  {} the draft still needs expansion into real specs + steps — run `/hex-feature-dev` or edit the file directly",
        "\u{2192}".dimmed()
    );

    Ok(())
}

async fn gc_drafts(days: u64) -> anyhow::Result<()> {
    let dir = drafts_dir();
    if !dir.is_dir() {
        println!("No drafts directory (nothing to gc).");
        return Ok(());
    }

    let threshold = std::time::Duration::from_secs(days * 24 * 60 * 60);
    let mut removed = 0;

    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if let Some(age) = age {
            if age > threshold {
                std::fs::remove_file(&path)?;
                removed += 1;
            }
        }
    }

    println!(
        "{} gc removed {} draft(s) older than {} day(s)",
        "\u{2713}".green(),
        removed,
        days
    );
    Ok(())
}
