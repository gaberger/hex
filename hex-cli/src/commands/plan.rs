//! Workplan management command.
//!
//! `hex plan` — create, list, and inspect workplans.
//!
//! Workplans decompose requirements into hex-bounded tasks organized by
//! dependency tier. Plans are saved to `docs/workplans/` as JSON.

use std::path::Path;

use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};

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
}

/// A workplan step.
#[derive(Debug, Serialize, Deserialize)]
struct Step {
    id: String,
    description: String,
    adapter: String,
    tier: u8,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    status: String,
}

/// A workplan document.
#[derive(Debug, Serialize, Deserialize)]
struct Workplan {
    #[serde(default)]
    title: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default, rename = "createdAt")]
    created_at: String,
}

pub async fn run(action: PlanAction) -> anyhow::Result<()> {
    match action {
        PlanAction::Create { requirements, lang, adr, no_adr } => create_plan(&requirements, &lang, adr.as_deref(), no_adr).await,
        PlanAction::List => list_plans().await,
        PlanAction::Status { file } => show_plan_status(&file).await,
        PlanAction::Active => show_active_executions().await,
        PlanAction::History => show_execution_history().await,
        PlanAction::Report { id } => show_execution_report(&id).await,
    }
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
            language: lang.to_string(),
            steps,
            created_at: chrono::Utc::now().to_rfc3339(),
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

    for entry in &entries {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy();

        if path.extension().map(|e| e == "json").unwrap_or(false) {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    if let Ok(plan) = serde_json::from_str::<Workplan>(&contents) {
                        let total = plan.steps.len();
                        let done = plan
                            .steps
                            .iter()
                            .filter(|s| s.status == "completed")
                            .count();
                        let title = if plan.title.is_empty() {
                            name.to_string()
                        } else {
                            plan.title.clone()
                        };

                        let progress = if total == 0 {
                            "(empty)".dimmed().to_string()
                        } else if done == total {
                            format!("{}/{} {}", done, total, "\u{2713}".green())
                        } else {
                            format!("{}/{}", done, total)
                        };

                        println!("  {} {} {}", "\u{25cb}".dimmed(), title, progress);
                    } else {
                        println!("  {} {} (parse error)", "\u{25cb}".dimmed(), name);
                    }
                }
                Err(_) => println!("  {} {} (read error)", "\u{25cb}".dimmed(), name),
            }
        } else {
            println!("  {} {} (markdown)", "\u{25cb}".dimmed(), name);
        }
    }

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

    println!(
        "{} {}",
        "\u{2b21}".cyan(),
        if plan.title.is_empty() {
            path.file_name().unwrap().to_string_lossy().to_string()
        } else {
            plan.title.clone()
        }
    );
    if !plan.language.is_empty() {
        println!("  Language: {}", plan.language);
    }
    if !plan.created_at.is_empty() {
        println!("  Created: {}", plan.created_at);
    }
    println!("  Steps: {}", plan.steps.len());
    println!();

    if plan.steps.is_empty() {
        println!("  (no steps defined)");
        return Ok(());
    }

    for step in &plan.steps {
        let icon = match step.status.as_str() {
            "completed" => "\u{2713}".green(),
            "in_progress" => "\u{25cf}".yellow(),
            _ => "\u{25cb}".dimmed(),
        };
        let deps = if step.dependencies.is_empty() {
            String::new()
        } else {
            format!(" (deps: {})", step.dependencies.join(", "))
        };
        println!(
            "  {} [{}] {}{}",
            icon,
            step.id,
            step.description,
            deps.dimmed(),
        );
        println!("     adapter: {} | tier: {}", step.adapter, step.tier);
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

    for exec in &active {
        let id = exec["id"].as_str().unwrap_or("?");
        let status = exec["status"].as_str().unwrap_or("?");
        let feature = exec["feature"].as_str().unwrap_or("");
        let phase = exec["currentPhase"].as_str().unwrap_or("?");
        let completed = exec["completedPhases"].as_u64().unwrap_or(0);
        let total = exec["totalPhases"].as_u64().unwrap_or(0);

        let status_icon = if status == "running" {
            "\u{25cf}".yellow()
        } else {
            "\u{25a0}".dimmed()
        };

        let label = if feature.is_empty() {
            id.to_string()
        } else {
            format!("{} ({})", feature, &id[..8.min(id.len())])
        };

        println!("  {} {} [{}] phase: {} ({}/{})",
            status_icon, label, status, phase, completed, total);
    }

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

    for exec in &executions {
        let id = exec["id"].as_str().unwrap_or("?");
        let status = exec["status"].as_str().unwrap_or("?");
        let feature = exec["feature"].as_str().unwrap_or("");
        let started = exec["startedAt"].as_str().unwrap_or("?");
        let tasks_done = exec["completedTasks"].as_u64().unwrap_or(0);
        let tasks_total = exec["totalTasks"].as_u64().unwrap_or(0);

        let status_icon = match status {
            "completed" => "\u{2713}".green(),
            "running" => "\u{25cf}".yellow(),
            "paused" => "\u{25a0}".dimmed(),
            "failed" => "\u{2717}".red(),
            _ => "\u{25cb}".dimmed(),
        };

        let label = if feature.is_empty() {
            id[..8.min(id.len())].to_string()
        } else {
            format!("{}", feature)
        };

        println!(
            "  {} {} [{}] tasks: {}/{} started: {}",
            status_icon, label, status, tasks_done, tasks_total, started
        );
        println!("    id: {}", id.dimmed());
    }

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
