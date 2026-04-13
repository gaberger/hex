//! Taste graph management commands (ADR-2604131500).
//!
//! `hex taste list|set|forget|pin` — manage developer preferences per scope/category.

use clap::Subcommand;
use colored::Colorize;
use serde::Deserialize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum TasteAction {
    /// List taste preferences (optionally filtered by scope/category)
    List {
        /// Filter by scope (e.g. "universal", "lang:rust", "domain:api")
        #[arg(long)]
        scope: Option<String>,
        /// Filter by category (e.g. "naming", "structure", "testing")
        #[arg(long)]
        category: Option<String>,
    },
    /// Set a taste preference
    Set {
        /// Scope (e.g. "universal", "lang:rust", "domain:api")
        scope: String,
        /// Category (e.g. "naming", "structure", "testing")
        category: String,
        /// Preference name (e.g. "snake_case_rust")
        name: String,
        /// Preference value / description
        value: String,
    },
    /// Forget (delete) a taste preference
    Forget {
        /// Preference key to remove
        key: String,
    },
    /// Pin a taste preference (prevent auto-decay)
    Pin {
        /// Preference key to pin
        key: String,
    },
}

/// A single taste entry as returned by GET /api/taste from hex-nexus.
#[derive(Deserialize, Debug)]
struct TasteEntry {
    #[serde(default)]
    key: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    source: String,
}

pub async fn run(action: TasteAction) -> anyhow::Result<()> {
    match action {
        TasteAction::List { scope, category } => list(scope.as_deref(), category.as_deref()).await,
        TasteAction::Set {
            scope,
            category,
            name,
            value,
        } => set(&scope, &category, &name, &value).await,
        TasteAction::Forget { key } => forget(&key).await,
        TasteAction::Pin { key } => pin(&key).await,
    }
}

async fn list(scope: Option<&str>, category: Option<&str>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let mut path = "/api/taste".to_string();
    let mut params: Vec<String> = Vec::new();
    if let Some(s) = scope {
        params.push(format!("scope={}", s));
    }
    if let Some(c) = category {
        params.push(format!("category={}", c));
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }

    match nexus.get(&path).await {
        Ok(value) => {
            let entries: Vec<TasteEntry> = if value.is_array() {
                serde_json::from_value(value)?
            } else {
                vec![serde_json::from_value(value)?]
            };

            if entries.is_empty() {
                println!("{} No taste preferences found.", "\u{2b21}".dimmed());
                return Ok(());
            }

            // Group entries by category
            let mut by_category: std::collections::BTreeMap<String, Vec<&TasteEntry>> =
                std::collections::BTreeMap::new();
            for entry in &entries {
                by_category
                    .entry(entry.category.clone())
                    .or_default()
                    .push(entry);
            }

            let header = "  hex taste \u{2014} developer preferences";
            println!("{}", header.bold());
            let separator = "\u{2500}".repeat(55);
            println!("  {}", separator.dimmed());

            for (cat, prefs) in &by_category {
                println!(
                    "  {} ({} preference{})",
                    cat.bold(),
                    prefs.len(),
                    if prefs.len() == 1 { "" } else { "s" }
                );
                for entry in prefs {
                    let source_colored = match entry.source.as_str() {
                        "manual" => "manual".cyan().to_string(),
                        "observed" => "observed".yellow().to_string(),
                        "inferred" => "inferred".dimmed().to_string(),
                        other => other.dimmed().to_string(),
                    };
                    println!(
                        "    {:<20} {:30} confidence: {:.2}  {}",
                        entry.name.green(),
                        format!("\"{}\"", entry.value).dimmed(),
                        entry.confidence,
                        source_colored,
                    );
                }
                println!();
            }

            println!("  {}", separator.dimmed());
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus taste endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

async fn set(scope: &str, category: &str, name: &str, value: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let body = json!({
        "scope": scope,
        "category": category,
        "name": name,
        "value": value,
    });

    match nexus.post("/api/taste", &body).await {
        Ok(_) => {
            println!(
                "{} taste set: {} / {} / {} = \"{}\"",
                "\u{2713}".green(),
                scope.dimmed(),
                category.dimmed(),
                name.bold(),
                value,
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus taste endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

async fn forget(key: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let path = format!("/api/taste/{}", key);

    match nexus.delete(&path).await {
        Ok(_) => {
            println!(
                "{} taste forgotten: {}",
                "\u{2713}".green(),
                key.bold(),
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus taste endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

async fn pin(key: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    let path = format!("/api/taste/{}/pin", key);
    let body = json!({});

    match nexus.patch(&path, &body).await {
        Ok(_) => {
            println!(
                "{} taste pinned: {} (auto-decay disabled)",
                "\u{2713}".green(),
                key.bold(),
            );
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("not found") || msg.contains("Cannot reach") {
                eprintln!(
                    "{} hex-nexus taste endpoint not available. Ensure hex-nexus is running with AIOS support.",
                    "!".yellow().bold()
                );
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}
