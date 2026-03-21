//! Remote agent management commands.
//!
//! `hex agent list|info|connect|spawn-remote|disconnect|fleet` — delegates to hex-nexus agent API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum AgentAction {
    /// List all agents (local + remote)
    List,
    /// Show detailed info for a specific agent
    Info {
        /// Agent ID
        agent_id: String,
    },
    /// Register this machine as a remote agent to a nexus instance
    Connect {
        /// Nexus URL to connect to (e.g. http://192.168.1.10:5555)
        nexus_url: String,
    },
    /// Tell nexus to SSH into a remote host and start hex-agent
    SpawnRemote {
        /// Remote host in user@host format
        target: String,
    },
    /// Disconnect a remote agent
    Disconnect {
        /// Agent ID to disconnect
        agent_id: String,
    },
    /// Show fleet capacity summary
    Fleet,
}

pub async fn run(action: AgentAction) -> anyhow::Result<()> {
    match action {
        AgentAction::List => list().await,
        AgentAction::Info { agent_id } => info(&agent_id).await,
        AgentAction::Connect { nexus_url } => connect(&nexus_url).await,
        AgentAction::SpawnRemote { target } => spawn_remote(&target).await,
        AgentAction::Disconnect { agent_id } => disconnect(&agent_id).await,
        AgentAction::Fleet => fleet().await,
    }
}

async fn list() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/api/agents").await?;
    let agents = resp.as_array().cloned().unwrap_or_default();

    if agents.is_empty() {
        println!("{} No agents connected.", "\u{2b21}".dimmed());
        return Ok(());
    }

    println!("{} Agents ({})", "\u{2b21}".cyan(), agents.len());
    println!();
    println!(
        "  {:<14} {:<16} {:<20} {:<10} {}",
        "ID".bold(),
        "NAME".bold(),
        "HOST".bold(),
        "STATUS".bold(),
        "MODELS".bold(),
    );
    println!("  {}", "\u{2500}".repeat(80).dimmed());

    for agent in &agents {
        let id = agent["agentId"].as_str().unwrap_or("?");
        let id_short = if id.len() > 12 { &id[..12] } else { id };

        let name = agent["name"].as_str().unwrap_or("?");
        let host = agent["host"].as_str().unwrap_or("local");
        let status = agent["status"].as_str().unwrap_or("?");

        let models = agent["capabilities"]["models"]
            .as_array()
            .map(|m| {
                m.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();

        let status_colored = match status {
            "online" | "active" | "connected" => status.green().to_string(),
            "idle" => status.yellow().to_string(),
            "offline" | "disconnected" => status.red().to_string(),
            "stale" => status.dimmed().to_string(),
            _ => status.to_string(),
        };

        println!(
            "  {:<14} {:<16} {:<20} {:<19} {}",
            id_short, name, host, status_colored, models,
        );
    }

    Ok(())
}

async fn info(agent_id: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let path = format!("/api/agents/{}", agent_id);
    let agent = nexus.get(&path).await?;

    println!("{} Agent Details", "\u{2b21}".cyan());
    println!();
    println!("  {:<16} {}", "ID:".bold(), agent["agentId"].as_str().unwrap_or("-"));
    println!("  {:<16} {}", "Name:".bold(), agent["name"].as_str().unwrap_or("-"));
    println!("  {:<16} {}", "Host:".bold(), agent["host"].as_str().unwrap_or("local"));
    println!("  {:<16} {}", "Status:".bold(), agent["status"].as_str().unwrap_or("-"));
    println!("  {:<16} {}", "Version:".bold(), agent["version"].as_str().unwrap_or("-"));

    if let Some(caps) = agent.get("capabilities") {
        println!();
        println!("  {}", "Capabilities:".bold());

        if let Some(models) = caps["models"].as_array() {
            let model_list: Vec<&str> = models.iter().filter_map(|v| v.as_str()).collect();
            println!("    {:<14} {}", "Models:", model_list.join(", "));
        }

        if let Some(max) = caps["maxConcurrent"].as_u64() {
            println!("    {:<14} {}", "Max concurrent:", max);
        }

        if let Some(gpu) = caps["gpu"].as_bool() {
            println!("    {:<14} {}", "GPU:", if gpu { "yes" } else { "no" });
        }

        if let Some(mem) = caps["memoryMb"].as_u64() {
            println!("    {:<14} {} MB", "Memory:", mem);
        }
    }

    if let Some(tasks) = agent.get("activeTasks").and_then(|t| t.as_array()) {
        if !tasks.is_empty() {
            println!();
            println!("  {} ({})", "Active Tasks:".bold(), tasks.len());
            for task in tasks {
                let tid = task["id"].as_str().unwrap_or("-");
                let title = task["title"].as_str().unwrap_or("-");
                let status = task["status"].as_str().unwrap_or("-");
                println!("    - [{}] {} ({})", status, title, tid);
            }
        }
    }

    if let Some(last_seen) = agent["lastHeartbeat"].as_str() {
        println!();
        println!("  {:<16} {}", "Last heartbeat:".bold(), last_seen);
    }

    Ok(())
}

async fn connect(nexus_url: &str) -> anyhow::Result<()> {
    // Connect to the specified nexus URL, not our local one
    let nexus = NexusClient::new(nexus_url.to_string());
    nexus.ensure_running().await?;

    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .to_string();

    let body = json!({
        "host": hostname,
        "capabilities": {
            "models": [],
            "maxConcurrent": 4,
        },
    });

    let resp = nexus.post("/api/agents/connect", &body).await?;

    let agent_id = resp["agentId"].as_str().unwrap_or("-");

    println!("{} Connected to nexus", "\u{2b21}".green());
    println!("  Nexus URL: {}", nexus_url);
    println!("  Agent ID:  {}", agent_id);
    println!("  Host:      {}", hostname);

    Ok(())
}

async fn spawn_remote(target: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let body = json!({
        "target": target,
    });

    println!(
        "{} Requesting nexus to spawn agent on {}...",
        "\u{2b21}".cyan(),
        target.bold()
    );

    let resp = nexus.post("/api/agents/spawn-remote", &body).await?;

    let agent_id = resp["agentId"].as_str().unwrap_or("-");
    let status = resp["status"].as_str().unwrap_or("pending");

    println!("{} Remote agent spawn initiated", "\u{2b21}".green());
    println!("  Target:   {}", target);
    println!("  Agent ID: {}", agent_id);
    println!("  Status:   {}", status);

    Ok(())
}

async fn disconnect(agent_id: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let path = format!("/api/agents/{}", agent_id);
    nexus.delete(&path).await?;

    println!("{} Agent disconnected", "\u{2b21}".green());
    println!("  Agent ID: {}", agent_id);

    Ok(())
}

async fn fleet() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/api/agents/fleet").await?;

    println!("{} Fleet Capacity Summary", "\u{2b21}".cyan());
    println!();

    let total = resp["totalAgents"].as_u64().unwrap_or(0);
    let online = resp["onlineAgents"].as_u64().unwrap_or(0);
    let total_slots = resp["totalSlots"].as_u64().unwrap_or(0);
    let used_slots = resp["usedSlots"].as_u64().unwrap_or(0);
    let available_slots = resp["availableSlots"].as_u64().unwrap_or(0);

    println!("  {:<20} {}", "Total agents:".bold(), total);
    println!("  {:<20} {}", "Online:".bold(), format!("{}", online).green());
    println!(
        "  {:<20} {}",
        "Offline:".bold(),
        if total > online {
            format!("{}", total - online).red().to_string()
        } else {
            "0".to_string()
        }
    );
    println!();
    println!("  {:<20} {}", "Total slots:".bold(), total_slots);
    println!("  {:<20} {}", "Used:".bold(), used_slots);
    println!("  {:<20} {}", "Available:".bold(), format!("{}", available_slots).green());

    if let Some(models) = resp["models"].as_array() {
        if !models.is_empty() {
            println!();
            println!("  {}", "Available Models:".bold());
            for model in models {
                let name = model["name"].as_str().unwrap_or("-");
                let count = model["agentCount"].as_u64().unwrap_or(0);
                println!("    - {} ({} agent{})", name, count, if count == 1 { "" } else { "s" });
            }
        }
    }

    Ok(())
}
