//! Project steering commands (ADR-2604131500).
//!
//! `hex steer direct` — send natural-language directives to a project.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum SteerAction {
    /// Send a natural language directive to a project
    Direct {
        /// Project name
        project: String,
        /// The directive (natural language)
        directive: String,
    },
}

pub async fn run(action: SteerAction) -> anyhow::Result<()> {
    match action {
        SteerAction::Direct {
            project,
            directive,
        } => direct(&project, &directive).await,
    }
}

async fn direct(project: &str, directive: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let body = json!({
        "project_id": project,
        "directive": directive,
    });

    match nexus.post("/api/steer", &body).await {
        Ok(resp) => {
            let classification = resp["classification"]
                .as_str()
                .unwrap_or("directive");
            let applied_to = resp["applied_to"]
                .as_str()
                .unwrap_or(project);
            println!(
                "{} Directive received. {} applied to {}.",
                "\u{2713}".green(),
                classification.bold(),
                applied_to.cyan()
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus steer endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}
