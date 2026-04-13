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

#[derive(Deserialize, Debug)]
struct TrustTree {
    #[serde(default)]
    project: String,
    #[serde(default)]
    scopes: Vec<TrustScope>,
}

#[derive(Deserialize, Debug)]
struct TrustScope {
    #[serde(default)]
    path: String,
    #[serde(default)]
    level: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    children: Vec<TrustScope>,
}

#[derive(Deserialize, Debug)]
struct TrustEvent {
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    from_level: String,
    #[serde(default)]
    to_level: String,
    #[serde(default)]
    reason: String,
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
        Some(p) => format!("/api/trust/{}", p),
        None => "/api/trust".to_string(),
    };

    match nexus.get(&path).await {
        Ok(value) => {
            let trees: Vec<TrustTree> = if value.is_array() {
                serde_json::from_value(value)?
            } else {
                vec![serde_json::from_value(value)?]
            };

            for tree in &trees {
                println!("{}", tree.project.bold());
                render_scopes(&tree.scopes, "", true);
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

fn render_scopes(scopes: &[TrustScope], prefix: &str, _is_root: bool) {
    let len = scopes.len();
    for (i, scope) in scopes.iter().enumerate() {
        let is_last = i == len - 1;
        let connector = if is_last { "\u{2514}\u{2500}\u{2500} " } else { "\u{251c}\u{2500}\u{2500} " };
        let child_prefix = if is_last { "    " } else { "\u{2502}   " };

        let level_colored = colorize_level(&scope.level);
        let pin_marker = if scope.pinned { " [pinned]".dimmed().to_string() } else { String::new() };
        let note = if scope.note.is_empty() {
            String::new()
        } else {
            format!("  ({})", scope.note).dimmed().to_string()
        };

        println!(
            "{}{}{:<20} {}{}{}",
            prefix, connector, scope.path, level_colored, note, pin_marker
        );

        if !scope.children.is_empty() {
            let new_prefix = format!("{}{}", prefix, child_prefix);
            render_scopes(&scope.children, &new_prefix, false);
        }
    }
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

    let body = json!({
        "path": scope_path,
        "level": level,
        "direction": direction,
    });

    match nexus.post("/api/trust/change", &body).await {
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

    let body = json!({
        "path": scope_path,
        "pin": true,
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

async fn history(project: Option<&str>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let path = match project {
        Some(p) => format!("/api/trust/{}/history", p),
        None => "/api/trust/history".to_string(),
    };

    match nexus.get(&path).await {
        Ok(value) => {
            let events: Vec<TrustEvent> = serde_json::from_value(
                value.get("events").cloned().unwrap_or(value.clone()),
            )?;

            if events.is_empty() {
                println!("{} No trust changes recorded.", "\u{2b21}".dimmed());
                return Ok(());
            }

            println!("{} Trust change history:", "\u{2b21}".cyan());
            println!();

            for event in &events {
                let arrow = if event.to_level == "act" || event.to_level == "silent" {
                    "\u{2191}".green()
                } else {
                    "\u{2193}".red()
                };
                println!(
                    "  {} {} {} {} \u{2192} {}",
                    event.timestamp.dimmed(),
                    event.scope.bold(),
                    arrow,
                    colorize_level(&event.from_level),
                    colorize_level(&event.to_level),
                );
                if !event.reason.is_empty() {
                    println!("    {}", event.reason.dimmed());
                }
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
