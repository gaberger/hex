//! Project status command.
//!
//! `hex status` — shows project info, git state, and service health.

use colored::Colorize;

use crate::nexus_client::NexusClient;

pub async fn run() -> anyhow::Result<()> {
    println!("{} hex project status", "\u{2b21}".cyan());
    println!();

    // Detect project root
    let cwd = std::env::current_dir()?;
    println!("  Project: {}", cwd.display());

    // Check for hex configuration
    let hex_dir = cwd.join(".hex");
    if hex_dir.is_dir() {
        println!("  Config:  {}", ".hex/ found".green());
    } else {
        println!("  Config:  {}", "no .hex/ directory".yellow());
    }

    // Check git status
    let git_output = tokio::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(&cwd)
        .output()
        .await;

    match git_output {
        Ok(output) if output.status.success() => {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("  Git:     {}", hash);
        }
        _ => {
            println!("  Git:     {}", "not a git repository".dimmed());
        }
    }

    // Check branch
    let branch_output = tokio::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&cwd)
        .output()
        .await;

    if let Ok(output) = branch_output {
        if output.status.success() {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("  Branch:  {}", branch);
        }
    }

    // Service health
    println!();
    println!("  {}", "Services:".bold());

    // hex-nexus — use NexusClient with auto port discovery
    let nexus = NexusClient::from_env();
    match nexus.ensure_running().await {
        Ok(()) => {
            println!("    hex-nexus:   {} ({})", "running".green(), nexus.url());

            // Get version
            if let Ok(ver) = nexus.get("/api/version").await {
                if let Some(v) = ver["version"].as_str() {
                    println!("    version:     {}", v);
                }
            }

            // SpacetimeDB status
            let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
                .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .ok();
            if let Some(client) = client {
                let stdb_ok = client
                    .get(format!("{}{}", stdb_host, hex_core::SPACETIMEDB_PING_PATH))
                    .send()
                    .await
                    .map(|r| r.status().is_success())
                    .unwrap_or(false);
                if stdb_ok {
                    println!("    spacetimedb: {} ({})", "running".green(), stdb_host);
                } else {
                    println!("    spacetimedb: {}", "not running".dimmed());
                }
            }
        }
        Err(_) => {
            println!("    hex-nexus:   {}", "not running".dimmed());
            println!("    spacetimedb: {}", "unknown".dimmed());
        }
    }

    Ok(())
}
