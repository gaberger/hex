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
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show current swarm status
    Status,
    /// List all swarms
    List,
}

pub async fn run(action: SwarmAction) -> anyhow::Result<()> {
    match action {
        SwarmAction::Init { name, topology, json } => init(&name, &topology, json).await,
        SwarmAction::Status => status().await,
        SwarmAction::List => list().await,
    }
}

async fn init(name: &str, topology: &str, json_output: bool) -> anyhow::Result<()> {
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

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        let id = resp["id"].as_str().unwrap_or("-");
        println!("{} Swarm initialized", "\u{2b21}".green());
        println!("  ID:       {}", id);
        println!("  Name:     {}", name.bold());
        println!("  Topology: {}", topology);
    }

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

    // Show task summary with agent assignments
    if let Some(tasks) = swarm["tasks"].as_array() {
        if !tasks.is_empty() {
            let completed = tasks.iter().filter(|t| t["status"].as_str() == Some("completed")).count();
            let total = tasks.len();
            println!(
                "  Progress: {}/{} completed",
                completed, total
            );
            println!();
            println!(
                "  {:<12} {:<16} {:<36} {}",
                "STATUS".bold(),
                "AGENT".bold(),
                "TASK ID".bold(),
                "TITLE".bold()
            );
            println!("  {}", "\u{2500}".repeat(90).dimmed());

            for task in tasks {
                let tid = task["id"].as_str().unwrap_or("-");
                let title = task["title"].as_str().unwrap_or("-");
                let status = task["status"].as_str().unwrap_or("unknown");
                let agent_id = task["agentId"].as_str()
                    .or_else(|| task["agent_id"].as_str())
                    .unwrap_or("");

                let status_colored = match status {
                    "completed" => status.green().to_string(),
                    "in_progress" | "running" => status.yellow().to_string(),
                    "pending" => status.dimmed().to_string(),
                    "failed" => status.red().to_string(),
                    _ => status.to_string(),
                };

                let agent_display = if agent_id.is_empty() {
                    "—".dimmed().to_string()
                } else if agent_id.len() > 14 {
                    agent_id[..14].to_string()
                } else {
                    agent_id.to_string()
                };

                let tid_short = if tid.len() > 34 { &tid[..34] } else { tid };
                println!(
                    "  {:<21} {:<16} {:<36} {}",
                    status_colored, agent_display, tid_short, title
                );
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
        "  {:<36} {:<20} {:<15} {:<10} {}",
        "ID".bold(),
        "NAME".bold(),
        "TOPOLOGY".bold(),
        "STATUS".bold(),
        "TASKS".bold(),
    );
    println!("  {}", "\u{2500}".repeat(95).dimmed());

    for swarm in &swarms {
        let id = swarm["id"].as_str().unwrap_or("-");
        let name = swarm["name"].as_str().unwrap_or("-");
        let topology = swarm["topology"].as_str().unwrap_or("-");
        let swarm_status = swarm["status"].as_str().unwrap_or("active");
        let tasks = swarm["tasks"].as_array();
        let total = tasks.map(|t| t.len()).unwrap_or(0);
        let completed = tasks
            .map(|t| t.iter().filter(|tk| tk["status"].as_str() == Some("completed")).count())
            .unwrap_or(0);
        let in_progress = tasks
            .map(|t| t.iter().filter(|tk| {
                let s = tk["status"].as_str().unwrap_or("");
                s == "in_progress" || s == "running"
            }).count())
            .unwrap_or(0);
        let pending = total - completed - in_progress;

        let status_colored = match swarm_status {
            "active" => swarm_status.green().to_string(),
            "completed" => swarm_status.dimmed().to_string(),
            _ => swarm_status.to_string(),
        };

        // Format: "3/5 done (1 active)"
        let task_summary = if total == 0 {
            "0".dimmed().to_string()
        } else if completed == total {
            format!("{}", format!("{}/{} done", completed, total).green())
        } else if in_progress > 0 {
            format!(
                "{}/{} done ({} active, {} pending)",
                completed, total, in_progress, pending
            )
        } else {
            format!("{}/{} done ({} pending)", completed, total, pending)
        };

        let id_short = if id.len() > 34 { &id[..34] } else { id };
        println!(
            "  {:<36} {:<20} {:<15} {:<19} {}",
            id_short, name, topology, status_colored, task_summary
        );
    }

    Ok(())
}
