//! Persistent memory commands.
//!
//! `hex memory store|get|search` — delegates to hex-nexus HexFlo memory API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum MemoryAction {
    /// Store a key-value pair
    Store {
        /// Key name
        key: String,
        /// Value to store
        value: String,
    },
    /// Retrieve a value by key
    Get {
        /// Key name
        key: String,
    },
    /// Search stored memory
    Search {
        /// Search query
        query: String,
    },
}

pub async fn run(action: MemoryAction) -> anyhow::Result<()> {
    match action {
        MemoryAction::Store { key, value } => store(&key, &value).await,
        MemoryAction::Get { key } => get(&key).await,
        MemoryAction::Search { query } => search(&query).await,
    }
}

async fn store(key: &str, value: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    nexus
        .post(
            "/api/hexflo/memory",
            &json!({
                "key": key,
                "value": value,
            }),
        )
        .await?;

    println!("{} Memory stored", "\u{2b21}".green());
    println!("  Key:   {}", key.bold());
    println!("  Value: {} bytes", value.len());

    Ok(())
}

async fn get(key: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let path = format!("/api/hexflo/memory/{}", key);
    match nexus.get(&path).await {
        Ok(resp) => {
            let value = resp["value"].as_str().unwrap_or("");
            println!("{} Memory lookup", "\u{2b21}".cyan());
            println!("  Key:   {}", key.bold());
            println!("  Value: {}", value);
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("404") {
                println!("{} Key '{}' not found", "\u{2b21}".yellow(), key);
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

async fn search(query: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let path = format!(
        "/api/hexflo/memory/search?q={}",
        urlencoded(query)
    );
    let resp = nexus.get(&path).await?;

    let results = resp
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    if results.is_empty() {
        println!(
            "{} No results for '{}'",
            "\u{2b21}".dimmed(),
            query
        );
        return Ok(());
    }

    println!(
        "{} Memory search: '{}' ({} results)",
        "\u{2b21}".cyan(),
        query.bold(),
        results.len()
    );
    println!();

    for entry in &results {
        let key = entry["key"].as_str().unwrap_or("-");
        let value = entry["value"].as_str().unwrap_or("");
        let preview = if value.len() > 60 {
            format!("{}...", &value[..57])
        } else {
            value.to_string()
        };
        println!("  {} {}", key.bold(), preview.dimmed());
    }

    Ok(())
}

/// Minimal percent-encoding for query parameters.
fn urlencoded(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('+', "%2B")
        .replace('#', "%23")
}
