//! Project briefing command (ADR-2604131500).
//!
//! `hex brief show` — summarizes project status, pending decisions, inference costs.

use clap::Subcommand;
use colored::Colorize;
use serde::Deserialize;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum BriefAction {
    /// Show briefing for all projects (default)
    Show {
        /// Filter to a specific project
        #[arg(long, short)]
        project: Option<String>,
        /// Only show pending decisions
        #[arg(long)]
        decisions: bool,
        /// Only show inference costs
        #[arg(long)]
        costs: bool,
        /// Show events since this time (e.g. "1h", "yesterday")
        #[arg(long)]
        since: Option<String>,
    },
}

#[derive(Deserialize, Debug)]
struct BriefingResponse {
    #[serde(default)]
    projects: Vec<ProjectBrief>,
    #[serde(default)]
    generated_at: String,
}

#[derive(Deserialize, Debug)]
struct ProjectBrief {
    #[serde(default)]
    name: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    phase_step: u32,
    #[serde(default)]
    phase_total: u32,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    health_score: u32,
    #[serde(default)]
    spend: String,
    #[serde(default)]
    active_agents: u32,
    #[serde(default)]
    pending_decisions: u32,
}

pub async fn run(action: BriefAction) -> anyhow::Result<()> {
    match action {
        BriefAction::Show {
            project,
            decisions,
            costs,
            since,
        } => show_briefing(project, decisions, costs, since).await,
    }
}

async fn show_briefing(
    project: Option<String>,
    decisions: bool,
    costs: bool,
    since: Option<String>,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    // Build query string
    let mut params = Vec::new();
    if let Some(ref p) = project {
        params.push(format!("project={}", p));
    }
    if decisions {
        params.push("decisions=true".to_string());
    }
    if costs {
        params.push("costs=true".to_string());
    }
    if let Some(ref s) = since {
        params.push(format!("since={}", s));
    }
    let qs = if params.is_empty() {
        String::new()
    } else {
        format!("?{}", params.join("&"))
    };

    let path = format!("/api/briefing{}", qs);

    match nexus.get(&path).await {
        Ok(value) => {
            let briefing: BriefingResponse = serde_json::from_value(value)?;
            render_briefing(&briefing);
        }
        Err(_) => {
            eprintln!(
                "{} hex-nexus briefing endpoint not available. Ensure hex-nexus is running with AIOS support.",
                "!".yellow().bold()
            );
        }
    }

    Ok(())
}

fn render_briefing(briefing: &BriefingResponse) {
    let date = if briefing.generated_at.is_empty() {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    } else {
        briefing.generated_at.clone()
    };

    let bar = "\u{2550}".repeat(55);
    println!("{}", bar.dimmed());
    println!(
        "  {}{}",
        "hex briefing".bold(),
        format!("{:>43}", date).dimmed()
    );
    println!("{}", bar.dimmed());

    if briefing.projects.is_empty() {
        println!("\n  {}\n", "No projects registered.".dimmed());
        println!("{}", bar.dimmed());
        return;
    }

    for proj in &briefing.projects {
        println!();
        let phase_info = if proj.phase_total > 0 {
            format!(
                " \u{2014} Phase: {} ({} of {})",
                proj.phase.to_uppercase(),
                proj.phase_step,
                proj.phase_total
            )
        } else if !proj.phase.is_empty() {
            format!(" \u{2014} Phase: {}", proj.phase.to_uppercase())
        } else {
            String::new()
        };
        println!("  {}{}", proj.name.bold().cyan(), phase_info);
        println!("  {}", "\u{2500}".repeat(53).dimmed());

        if !proj.summary.is_empty() {
            println!("  {}", proj.summary);
        }

        println!(
            "  Health: {} | Spend: {} | Agents: {}",
            format!("{}/100", proj.health_score).green(),
            proj.spend.yellow(),
            proj.active_agents
        );
        println!();
        println!("  Pending decisions: {}", proj.pending_decisions);
    }

    println!("{}", bar.dimmed());
}
