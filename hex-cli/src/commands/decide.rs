//! Decision resolution commands (ADR-2604131500).
//!
//! `hex decide resolve|approve-all|explain` — manage pending project decisions.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum DecideAction {
    /// Resolve a pending decision
    Resolve {
        /// Project name
        project: String,
        /// Decision ID (from hex brief output)
        decision_id: u64,
        /// Action: approve, reject, override
        action: String,
        /// Value for override action
        #[arg(long, short)]
        value: Option<String>,
    },
    /// Approve all pending decisions (use defaults)
    ApproveAll {
        /// Project name (or all projects if omitted)
        #[arg(long, short)]
        project: Option<String>,
    },
    /// Explain a decision's tradeoffs
    Explain {
        /// Project name
        project: String,
        /// Decision ID
        decision_id: u64,
    },
}

pub async fn run(action: DecideAction) -> anyhow::Result<()> {
    match action {
        DecideAction::Resolve {
            project,
            decision_id,
            action,
            value,
        } => resolve(&project, decision_id, &action, value.as_deref()).await,
        DecideAction::ApproveAll { project } => approve_all(project.as_deref()).await,
        DecideAction::Explain {
            project,
            decision_id,
        } => explain(&project, decision_id).await,
    }
}

async fn resolve(
    project: &str,
    decision_id: u64,
    action: &str,
    value: Option<&str>,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let body = json!({
        "action": action,
        "value": value,
    });

    let path = format!("/api/{}/decisions/{}", project, decision_id);

    match nexus.post(&path, &body).await {
        Ok(resp) => {
            let summary = resp["summary"]
                .as_str()
                .unwrap_or("decision resolved");
            println!(
                "{} Decision #{} resolved: {}",
                "\u{2713}".green(),
                decision_id,
                summary
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") {
                eprintln!(
                    "{} hex-nexus decisions endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

async fn approve_all(project: Option<&str>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let body = json!({
        "project": project,
        "action": "approve_all",
    });

    match nexus.post("/api/decisions/approve-all", &body).await {
        Ok(resp) => {
            let count = resp["approved_count"].as_u64().unwrap_or(0);
            let scope = project.unwrap_or("all projects");
            println!(
                "{} Approved {} pending decision(s) for {}",
                "\u{2713}".green(),
                count,
                scope
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") {
                eprintln!(
                    "{} hex-nexus decisions endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

async fn explain(project: &str, decision_id: u64) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let path = format!("/api/{}/decisions/{}/explain", project, decision_id);

    match nexus.get(&path).await {
        Ok(resp) => {
            let title = resp["title"].as_str().unwrap_or("Unknown decision");
            let explanation = resp["explanation"].as_str().unwrap_or("");
            let options = resp["options"].as_array();

            println!("{} Decision #{}: {}", "\u{2b21}".cyan(), decision_id, title.bold());
            println!();

            if !explanation.is_empty() {
                println!("  {}", explanation);
                println!();
            }

            if let Some(opts) = options {
                println!("  {}:", "Options".bold());
                for opt in opts {
                    let label = opt["label"].as_str().unwrap_or("-");
                    let desc = opt["description"].as_str().unwrap_or("");
                    let is_default = opt["default"].as_bool().unwrap_or(false);
                    let marker = if is_default {
                        " (default)".dimmed().to_string()
                    } else {
                        String::new()
                    };
                    println!("    - {}{}: {}", label.bold(), marker, desc);
                }
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") {
                eprintln!(
                    "{} hex-nexus decisions endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}
