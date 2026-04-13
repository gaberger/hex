//! Emergency pause/resume (ADR-2604131500 §1 Layer 4).

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum PauseAction {
    /// Pause the active workplan
    Pause,
    /// Resume a paused workplan
    Resume,
}

pub async fn run_pause() -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    let resp = client.post("/api/workplan/pause", &json!({})).await?;

    let feature = resp["feature"].as_str().unwrap_or("");
    if feature.is_empty() {
        println!("{} Nothing running.", "\u{23f8}".dimmed());
    } else {
        println!("{} Paused: {}", "\u{23f8}".yellow(), feature.bold());
    }

    Ok(())
}

pub async fn run_resume() -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    let resp = client.post("/api/workplan/resume", &json!({})).await?;

    let feature = resp["feature"].as_str().unwrap_or("");
    if feature.is_empty() {
        println!("{} Nothing paused.", "\u{25b6}".dimmed());
    } else {
        println!("{} Resumed: {}", "\u{25b6}".green(), feature.bold());
    }

    Ok(())
}
