//! Persistent memory commands.
//!
//! `hex memory store|get|search|sync-check|validate` — delegates to hex-nexus HexFlo memory API.

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
    /// Verify cross-agent memory sync (WP P4-3)
    SyncCheck {
        /// Key name (default: unique `sync-check-<uuid>`)
        #[arg(long)]
        key: Option<String>,
        /// Value to round-trip (default: unique timestamped value)
        #[arg(long)]
        value: Option<String>,
        /// Keep the test key after verification (default: delete it)
        #[arg(long)]
        keep: bool,
    },
    /// Validate memory sync across agents by performing a store→get roundtrip
    Validate {
        /// Optional key to use for the roundtrip (default: auto-generated)
        #[arg(long)]
        key: Option<String>,
        /// Optional value to store (default: auto-generated UUID-like token)
        #[arg(long)]
        value: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: MemoryAction) -> anyhow::Result<()> {
    match action {
        MemoryAction::Store { key, value } => store(&key, &value).await,
        MemoryAction::Get { key } => get(&key).await,
        MemoryAction::Search { query } => search(&query).await,
        MemoryAction::SyncCheck { key, value, keep } => sync_check(key, value, keep).await,
        MemoryAction::Validate { key, value, json } => validate(key, value, json).await,
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

/// Simulates two agents: "agent A" stores, "agent B" reads back.
async fn sync_check(
    key: Option<String>,
    value: Option<String>,
    keep: bool,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let ts = chrono::Utc::now().timestamp_millis();
    let agent_hint = std::env::var("HEX_AGENT_ID")
        .or_else(|_| std::env::var("CLAUDE_SESSION_ID"))
        .unwrap_or_else(|_| "local".to_string());

    let key = key.unwrap_or_else(|| format!("sync-check-{}-{}", agent_hint, ts));
    let value = value.unwrap_or_else(|| format!("value-{}", ts));

    nexus
        .post(
            "/api/hexflo/memory",
            &json!({ "key": &key, "value": &value }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("store failed: {}", e))?;

    let path = format!("/api/hexflo/memory/{}", key);
    let resp = nexus
        .get(&path)
        .await
        .map_err(|e| anyhow::anyhow!("readback failed: {}", e))?;

    let observed = resp["value"].as_str().unwrap_or("");
    let backend = resp["backend"].as_str().unwrap_or("unknown");
    let matches = observed == value;

    if matches {
        println!("{} Memory sync OK", "\u{2b21}".green());
        println!("  Key:     {}", key.bold());
        println!("  Value:   {}", value);
        println!("  Backend: {}", backend);
    } else {
        println!("{} Memory sync MISMATCH", "\u{2b21}".red());
        println!("  Key:      {}", key.bold());
        println!("  Expected: {}", value);
        println!("  Observed: {}", observed);
        println!("  Backend:  {}", backend);
    }

    if !keep {
        let _ = nexus.delete(&path).await;
    }

    if !matches {
        anyhow::bail!("memory sync mismatch");
    }
    Ok(())
}

/// Perform a store→get roundtrip against the nexus memory API to validate
/// that multi-agent memory sync is intact. The value flows through
/// SpacetimeDB; if STDB is unreachable the roundtrip fails. Satisfies
/// P4-3: a second agent calling `hex memory get` on the same key would
/// observe the stored value.
async fn validate(
    key: Option<String>,
    value: Option<String>,
    json_output: bool,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let ts = chrono::Utc::now().timestamp_millis();
    let agent_hint = std::env::var("HEX_AGENT_ID")
        .or_else(|_| std::env::var("CLAUDE_SESSION_ID"))
        .unwrap_or_else(|_| "local".to_string());

    let key = key.unwrap_or_else(|| format!("p4-3-sync-{}-{}", agent_hint, ts));
    let value = value.unwrap_or_else(|| format!("value-{}", ts));

    // Store
    nexus
        .post(
            "/api/hexflo/memory",
            &json!({ "key": &key, "value": &value }),
        )
        .await
        .map_err(|e| anyhow::anyhow!("store failed for key '{}': {}", key, e))?;

    // Readback — this is what a second agent would do.
    let path = format!("/api/hexflo/memory/{}", key);
    let resp = nexus
        .get(&path)
        .await
        .map_err(|e| anyhow::anyhow!("readback failed for key '{}': {}", key, e))?;

    let observed = resp["value"].as_str().unwrap_or("");
    let backend = resp["backend"].as_str().unwrap_or("unknown");
    let matches = observed == value;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "ok": matches,
                "key": key,
                "expected": value,
                "observed": observed,
                "backend": backend,
            }))?
        );
        if !matches {
            anyhow::bail!("memory sync mismatch");
        }
        return Ok(());
    }

    if matches {
        println!("{} Memory sync OK", "\u{2b21}".green());
        println!("  Key:     {}", key.bold());
        println!("  Value:   {}", value);
        println!("  Backend: {}", backend);
    } else {
        println!("{} Memory sync MISMATCH", "\u{2b21}".red());
        println!("  Key:      {}", key.bold());
        println!("  Expected: {}", value);
        println!("  Observed: {}", observed);
        println!("  Backend:  {}", backend);
        anyhow::bail!("memory sync mismatch");
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
