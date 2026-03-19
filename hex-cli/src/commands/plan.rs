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
    },
    /// List existing workplans
    List,
    /// Show status of a specific workplan
    Status {
        /// Workplan filename (e.g. feat-secrets-plan-b.json)
        file: String,
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
        PlanAction::Create { requirements, lang } => create_plan(&requirements, &lang).await,
        PlanAction::List => list_plans().await,
        PlanAction::Status { file } => show_plan_status(&file).await,
    }
}

/// Decompose requirements into hex-bounded tasks by tier.
async fn create_plan(requirements: &[String], lang: &str) -> anyhow::Result<()> {
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
