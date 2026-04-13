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
    project_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    events: Vec<BriefEvent>,
    #[serde(default)]
    pending_decisions: Vec<BriefDecision>,
    #[serde(default)]
    summary: BriefSummary,
}

#[derive(Deserialize, Debug, Default)]
struct BriefSummary {
    #[serde(default)]
    agent_count: u32,
    #[serde(default)]
    event_count: u32,
    #[serde(default)]
    decision_count: u32,
    #[serde(default)]
    health: u32,
    #[serde(default)]
    spend: f64,
}

#[derive(Deserialize, Debug)]
struct BriefEvent {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    agent_id: String,
}

#[derive(Deserialize, Debug)]
struct BriefDecision {
    #[serde(default)]
    id: serde_json::Value,
    #[serde(default)]
    priority: u32,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    payload: String,
    #[serde(default)]
    created_at: String,
    /// ADR-2604131500 P1.1: What hex will do if no response (e.g. "approve", "pause").
    #[serde(default)]
    default_action: String,
    /// ADR-2604131500 P1.1: Why hex chose this default (trust level, scope, rules).
    #[serde(default)]
    reasoning: String,
    /// ADR-2604131500 P1.1: When auto-resolution fires (RFC 3339 timestamp, empty = never).
    #[serde(default)]
    deadline_at: String,
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
            render_briefing(&briefing, decisions);
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

fn render_briefing(briefing: &BriefingResponse, decisions_only: bool) {
    let date = if briefing.generated_at.is_empty() {
        chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
    } else {
        briefing.generated_at.chars().take(16).collect::<String>()
    };

    let bar = "\u{2550}".repeat(58);
    println!("{}", bar.dimmed());
    println!(
        "  {}{}",
        "hex briefing".bold(),
        format!("{:>46}", date).dimmed()
    );
    println!("{}", bar.dimmed());

    if briefing.projects.is_empty() {
        println!("\n  {}\n", "No projects registered.".dimmed());
        println!("{}", bar.dimmed());
        return;
    }

    let mut total_decisions = 0usize;

    for proj in &briefing.projects {
        let proj_name = if proj.name.is_empty() {
            &proj.project_id
        } else {
            &proj.name
        };

        println!();
        println!("  {}", proj_name.bold().cyan());
        println!("  {}", "\u{2500}".repeat(56).dimmed());

        // ── What happened (completed events) ──
        if !decisions_only {
            let completed: Vec<&BriefEvent> =
                proj.events.iter().filter(|e| e.status == "completed").collect();
            let in_progress: Vec<&BriefEvent> =
                proj.events.iter().filter(|e| e.status == "in_progress").collect();

            if !completed.is_empty() {
                println!("  {} {}", "\u{2714}".green(), "What happened:".bold());
                for ev in &completed {
                    println!("    {} {}", "\u{2022}".dimmed(), ev.title);
                }
            }

            // ── What's happening (active tasks) ──
            if !in_progress.is_empty() {
                println!(
                    "  {} {} ({} agent{})",
                    "\u{25B6}".blue(),
                    "What's happening:".bold(),
                    proj.summary.agent_count,
                    if proj.summary.agent_count == 1 { "" } else { "s" }
                );
                for ev in &in_progress {
                    let agent_tag = if ev.agent_id.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", format!("[{}]", &ev.agent_id[..8.min(ev.agent_id.len())]).dimmed())
                    };
                    println!("    {} {}{}", "\u{25CF}".blue(), ev.title, agent_tag);
                }
            }

            if completed.is_empty() && in_progress.is_empty() {
                println!("  {}", "No recent activity.".dimmed());
            }
        }

        // ── What needs input (pending decisions) ──
        if !proj.pending_decisions.is_empty() {
            total_decisions += proj.pending_decisions.len();
            println!(
                "  {} {} ({})",
                "\u{26A0}".yellow(),
                "Needs your input:".bold().yellow(),
                proj.pending_decisions.len()
            );
            for (i, dec) in proj.pending_decisions.iter().enumerate() {
                let priority_tag = match dec.priority {
                    2 => " CRITICAL".red().bold().to_string(),
                    1 => " important".yellow().to_string(),
                    _ => String::new(),
                };

                // Try to extract structured fields from top-level or parsed payload JSON
                let parsed: Option<serde_json::Value> =
                    serde_json::from_str(&dec.payload).ok();
                let default_action = if !dec.default_action.is_empty() {
                    dec.default_action.clone()
                } else {
                    parsed.as_ref()
                        .and_then(|v| v["default_action"].as_str())
                        .unwrap_or("")
                        .to_string()
                };
                let reasoning = if !dec.reasoning.is_empty() {
                    dec.reasoning.clone()
                } else {
                    parsed.as_ref()
                        .and_then(|v| v["reasoning"].as_str())
                        .unwrap_or("")
                        .to_string()
                };
                let deadline_at = if !dec.deadline_at.is_empty() {
                    dec.deadline_at.clone()
                } else {
                    parsed.as_ref()
                        .and_then(|v| v["deadline_at"].as_str())
                        .unwrap_or("")
                        .to_string()
                };

                // Display phase name from payload if available, else raw payload preview
                let summary = parsed.as_ref()
                    .and_then(|v| v["phase"].as_str())
                    .map(|p| format!("Phase: {}", p));
                let payload_preview = if let Some(ref s) = summary {
                    s.clone()
                } else if dec.payload.len() > 60 {
                    format!("{}...", &dec.payload[..57])
                } else {
                    dec.payload.clone()
                };

                println!(
                    "    {}. {}{} {}",
                    i + 1,
                    payload_preview,
                    priority_tag,
                    format!("[{}]", dec.kind).dimmed()
                );

                // Show default action + deadline
                if !default_action.is_empty() {
                    let deadline_display = if deadline_at.is_empty() {
                        "no auto-resolution".to_string()
                    } else {
                        // Show relative time if possible, else raw timestamp
                        let trimmed = deadline_at.chars().take(16).collect::<String>();
                        format!("auto at {}", trimmed)
                    };
                    println!(
                        "       {} Default: {} {}",
                        "\u{2192}".dimmed(),
                        default_action.bold(),
                        format!("({})", deadline_display).dimmed()
                    );
                }

                // Show reasoning
                if !reasoning.is_empty() {
                    println!(
                        "       {} Reason: {}",
                        "\u{2192}".dimmed(),
                        reasoning.dimmed()
                    );
                }

                println!(
                    "       {} hex decide {} {} approve",
                    "\u{2192}".dimmed(),
                    proj_name,
                    dec.id
                );
            }
        }

        // ── Summary line ──
        if !decisions_only {
            let health_str = if proj.summary.health > 0 {
                let h = proj.summary.health;
                let colored = if h >= 80 {
                    format!("{}/100", h).green().to_string()
                } else if h >= 50 {
                    format!("{}/100", h).yellow().to_string()
                } else {
                    format!("{}/100", h).red().to_string()
                };
                format!("Health: {}", colored)
            } else {
                String::new()
            };

            let spend_str = if proj.summary.spend > 0.0 {
                format!(" | Spend: {}", format!("${:.2}", proj.summary.spend).yellow())
            } else {
                String::new()
            };

            if !health_str.is_empty() || !spend_str.is_empty() {
                println!("  {}{}", health_str, spend_str);
            }
        }
    }

    println!("{}", bar.dimmed());

    // ── Footer ──
    if total_decisions > 0 {
        println!(
            "  {} {} pending decision{}. Run {} to view.",
            "\u{25CF}".yellow(),
            total_decisions,
            if total_decisions == 1 { "" } else { "s" },
            "hex brief --decisions".bold()
        );
    } else {
        println!("  {} All clear — no decisions needed.", "\u{2714}".green());
    }
    println!();
}
