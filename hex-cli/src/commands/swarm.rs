//! Swarm coordination commands.
//!
//! `hex swarm init|status|list` — delegates to hex-nexus HexFlo API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum SwarmAction {
    /// Initialize a new swarm
    Init {
        /// Swarm name
        name: String,
        /// Topology type
        #[arg(short, long, default_value = "hierarchical")]
        topology: String,
    },
    /// Show current swarm status
    Status,
    /// List all swarms
    List,
}

pub async fn run(action: SwarmAction) -> anyhow::Result<()> {
    match action {
        SwarmAction::Init { name, topology } => init(&name, &topology).await,
        SwarmAction::Status => status().await,
        SwarmAction::List => list().await,
    }
}

async fn init(name: &str, topology: &str) -> anyhow::Result<()> {
    match topology {
        "hierarchical" | "mesh" | "pipeline" => {}
        other => {
            anyhow::bail!(
                "Unknown topology '{}'. Supported: hierarchical, mesh, pipeline",
                other
            );
        }
    }

    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Derive projectId from current directory (matches hex-nexus make_project_id)
    let cwd = std::env::current_dir()?;
    let project_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let resp = nexus
        .post(
            "/api/swarms",
            &json!({
                "projectId": project_name,
                "name": name,
                "topology": topology,
            }),
        )
        .await?;

    let id = resp["id"].as_str().unwrap_or("-");
    println!("{} Swarm initialized", "\u{2b21}".green());
    println!("  ID:       {}", id);
    println!("  Name:     {}", name.bold());
    println!("  Topology: {}", topology);

    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/api/swarms/active").await?;
    // Response is a plain array of swarm objects
    let swarms = resp.as_array().cloned().unwrap_or_default();

    if swarms.is_empty() {
        println!("{} No active swarms", "\u{2b21}".dimmed());
        return Ok(());
    }

    // Show the most recent/active swarm
    let swarm = &swarms[0];
    let name = swarm["name"].as_str().unwrap_or("-");
    let id = swarm["id"].as_str().unwrap_or("-");
    let topology = swarm["topology"].as_str().unwrap_or("-");
    let task_count = swarm["tasks"]
        .as_array()
        .map(|t| t.len())
        .unwrap_or(0);

    println!("{} Active swarm", "\u{2b21}".cyan());
    println!("  ID:       {}", id);
    println!("  Name:     {}", name.bold());
    println!("  Topology: {}", topology);
    println!("  Tasks:    {}", task_count);

    // Show task summary if any
    if let Some(tasks) = swarm["tasks"].as_array() {
        if !tasks.is_empty() {
            println!();
            println!(
                "  {:<36} {:<12} {}",
                "TASK ID".bold(),
                "STATUS".bold(),
                "TITLE".bold()
            );
            println!("  {}", "\u{2500}".repeat(70).dimmed());

            for task in tasks {
                let tid = task["id"].as_str().unwrap_or("-");
                let title = task["title"].as_str().unwrap_or("-");
                let status = task["status"].as_str().unwrap_or("unknown");

                let status_colored = match status {
                    "completed" => status.green().to_string(),
                    "in_progress" | "running" => status.yellow().to_string(),
                    "pending" => status.dimmed().to_string(),
                    "failed" => status.red().to_string(),
                    _ => status.to_string(),
                };

                let tid_short = if tid.len() > 34 { &tid[..34] } else { tid };
                println!("  {:<36} {:<21} {}", tid_short, status_colored, title);
            }
        }
    }

    Ok(())
}

async fn list() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/api/swarms/active").await?;
    let swarms = resp.as_array().cloned().unwrap_or_default();

    if swarms.is_empty() {
        println!("{} No registered swarms", "\u{2b21}".dimmed());
        return Ok(());
    }

    println!("{} Swarms ({})", "\u{2b21}".cyan(), swarms.len());
    println!();
    println!(
        "  {:<36} {:<20} {:<15} {}",
        "ID".bold(),
        "NAME".bold(),
        "TOPOLOGY".bold(),
        "TASKS".bold()
    );
    println!("  {}", "\u{2500}".repeat(80).dimmed());

    for swarm in &swarms {
        let id = swarm["id"].as_str().unwrap_or("-");
        let name = swarm["name"].as_str().unwrap_or("-");
        let topology = swarm["topology"].as_str().unwrap_or("-");
        let task_count = swarm["tasks"]
            .as_array()
            .map(|t| t.len())
            .unwrap_or(0);

        let id_short = if id.len() > 34 { &id[..34] } else { id };
        println!(
            "  {:<36} {:<20} {:<15} {}",
            id_short, name, topology, task_count
        );
    }

    Ok(())
}
