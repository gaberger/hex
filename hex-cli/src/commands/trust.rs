//! Trust level management commands (ADR-2604131500).
//!
//! `hex trust show|elevate|reduce|pin|history` — manage delegation trust levels per scope.

use clap::Subcommand;
use colored::Colorize;
use serde::Deserialize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum TrustAction {
    /// Show trust levels for a project
    Show {
        /// Project name (shows all if omitted)
        project: Option<String>,
    },
    /// Elevate trust for a scope
    Elevate {
        /// Project/scope path (e.g. "myproject/domain" or "myproject/adapters/secondary")
        path: String,
        /// Target level: observe, suggest, act, silent
        level: String,
    },
    /// Reduce trust for a scope
    Reduce {
        /// Project/scope path
        path: String,
        /// Target level
        level: String,
    },
    /// Pin trust to prevent auto-decay
    Pin {
        /// Project/scope path
        path: String,
    },
    /// Show trust change history
    History {
        /// Project name
        project: Option<String>,
    },
}

/// A single trust entry as returned by GET /api/trust from hex-nexus.
///
/// Trust data is stored in HexFlo memory with keys like `trust:<project>:<scope>`.
/// The nexus endpoint returns a flat JSON array of these entries.
#[derive(Deserialize, Debug)]
struct TrustEntry {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    updated_at: String,
}

pub async fn run(action: TrustAction) -> anyhow::Result<()> {
    match action {
        TrustAction::Show { project } => show(project.as_deref()).await,
        TrustAction::Elevate { path, level } => change_trust(&path, &level, "elevate").await,
        TrustAction::Reduce { path, level } => change_trust(&path, &level, "reduce").await,
        TrustAction::Pin { path } => pin(&path).await,
        TrustAction::History { project } => history(project.as_deref()).await,
    }
}

async fn show(project: Option<&str>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let path = match project {
        Some(p) => format!("/api/trust?project={}", p),
        None => "/api/trust".to_string(),
    };

    match nexus.get(&path).await {
        Ok(value) => {
            // The nexus returns a flat array of TrustEntry objects.
            let entries: Vec<TrustEntry> = if value.is_array() {
                serde_json::from_value(value)?
            } else {
                // Single object — wrap in vec
                vec![serde_json::from_value(value)?]
            };

            if entries.is_empty() {
                println!(
                    "{} No trust entries found{}.",
                    "\u{2b21}".dimmed(),
                    project.map(|p| format!(" for {}", p)).unwrap_or_default()
                );
                return Ok(());
            }

            // Group entries by project_id for display
            let mut by_project: std::collections::BTreeMap<String, Vec<&TrustEntry>> =
                std::collections::BTreeMap::new();
            for entry in &entries {
                by_project
                    .entry(entry.project_id.clone())
                    .or_default()
                    .push(entry);
            }

            for (proj, scopes) in &by_project {
                println!("{}", proj.bold());
                let len = scopes.len();
                for (i, entry) in scopes.iter().enumerate() {
                    let is_last = i == len - 1;
                    let connector = if is_last { "\u{2514}\u{2500}\u{2500} " } else { "\u{251c}\u{2500}\u{2500} " };
                    let level_colored = colorize_level(&entry.level);
                    let pin_marker = if entry.pinned {
                        " [pinned]".dimmed().to_string()
                    } else {
                        String::new()
                    };
                    println!(
                        "  {}{:<24} {}{}",
                        connector, entry.scope, level_colored, pin_marker
                    );
                }
                println!();
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus trust endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

fn colorize_level(level: &str) -> String {
    match level {
        "act" | "silent" => level.green().to_string(),
        "suggest" => level.yellow().to_string(),
        "observe" => level.red().to_string(),
        _ => level.dimmed().to_string(),
    }
}

async fn change_trust(scope_path: &str, level: &str, direction: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    // scope_path is "project/scope" — split into project_id and scope
    let (project_id, scope) = match scope_path.split_once('/') {
        Some((p, s)) => (p.to_string(), s.to_string()),
        None => (scope_path.to_string(), String::new()),
    };

    let body = json!({
        "project_id": project_id,
        "scope": scope,
        "level": level,
    });

    match nexus.patch("/api/trust", &body).await {
        Ok(resp) => {
            let prev = resp["previous_level"].as_str().unwrap_or("unknown");
            let arrow = if direction == "elevate" {
                "\u{2191}".green()
            } else {
                "\u{2193}".red()
            };
            println!(
                "{} {} trust: {} {} \u{2192} {}",
                "\u{2713}".green(),
                scope_path.bold(),
                colorize_level(prev),
                arrow,
                colorize_level(level)
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus trust endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

async fn pin(scope_path: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    // scope_path is "project/scope" — split into project_id and scope
    let (project_id, scope) = match scope_path.split_once('/') {
        Some((p, s)) => (p.to_string(), s.to_string()),
        None => (scope_path.to_string(), String::new()),
    };

    let body = json!({
        "project_id": project_id,
        "scope": scope,
    });

    match nexus.post("/api/trust/pin", &body).await {
        Ok(_) => {
            println!(
                "{} {} trust level pinned (auto-decay disabled)",
                "\u{2713}".green(),
                scope_path.bold()
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus trust endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

/// A single trust history entry as returned by GET /api/trust/history.
#[derive(Deserialize, Debug)]
struct TrustHistoryEntry {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    old_level: String,
    #[serde(default)]
    new_level: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    changed_at: String,
}

/// Trust level ordering for determining elevation vs decay.
fn trust_level_rank(level: &str) -> u8 {
    match level {
        "observe" => 0,
        "suggest" => 1,
        "act" => 2,
        "silent" => 3,
        _ => 0,
    }
}

async fn history(project: Option<&str>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let path = match project {
        Some(p) => format!("/api/trust/history?project={}", p),
        None => "/api/trust/history".to_string(),
    };

    match nexus.get(&path).await {
        Ok(value) => {
            let entries: Vec<TrustHistoryEntry> = if value.is_array() {
                serde_json::from_value(value)?
            } else {
                vec![serde_json::from_value(value)?]
            };

            if entries.is_empty() {
                println!("{} No trust changes recorded.", "\u{2b21}".dimmed());
                return Ok(());
            }

            let header = format!(
                "  hex trust history{}",
                project.map(|p| format!(" \u{2014} {}", p)).unwrap_or_default()
            );
            println!("{}", header.bold());
            let separator = "\u{2500}".repeat(55);
            println!("  {}", separator.dimmed());

            for entry in &entries {
                // Format timestamp: trim to "YYYY-MM-DD HH:MM" if RFC3339
                let ts_display = if entry.changed_at.len() >= 16 {
                    entry.changed_at[..16].replace('T', " ")
                } else if entry.changed_at.is_empty() {
                    "unknown".to_string()
                } else {
                    entry.changed_at.clone()
                };

                let is_elevation =
                    trust_level_rank(&entry.new_level) > trust_level_rank(&entry.old_level);

                let arrow_str = if entry.old_level.is_empty() {
                    format!("\u{2192} {}", colorize_level(&entry.new_level))
                } else if is_elevation {
                    format!(
                        "{} \u{2192} {}",
                        colorize_level(&entry.old_level),
                        entry.new_level.green()
                    )
                } else {
                    format!(
                        "{} \u{2192} {}",
                        colorize_level(&entry.old_level),
                        entry.new_level.red()
                    )
                };

                println!(
                    "  {}  {:<16} {}  {}",
                    ts_display.dimmed(),
                    entry.scope,
                    arrow_str,
                    entry.reason.dimmed(),
                );
            }

            println!("  {}", separator.dimmed());
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus trust endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}
