//! Remote agent management commands.
//!
//! `hex agent list|info|connect|spawn-remote|disconnect|fleet` — delegates to hex-nexus agent API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

use std::path::PathBuf;

#[derive(Subcommand)]
pub enum AgentAction {
    /// List all agents (local + remote)
    List,
    /// Show detailed info for a specific agent
    Info {
        /// Agent ID
        agent_id: String,
    },
    /// Show detailed status for a remote agent
    Status {
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
        /// Remote project directory (where hex-agent runs)
        #[arg(long)]
        project_dir: Option<String>,
        /// Remote source directory to sync project files to before spawning
        #[arg(long)]
        source_dir: Option<String>,
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
        AgentAction::Status { agent_id } => agent_status(&agent_id).await,
        AgentAction::Connect { nexus_url } => connect(&nexus_url).await,
        AgentAction::SpawnRemote {
            target,
            project_dir,
            source_dir,
        } => spawn_remote(&target, project_dir, source_dir).await,
        AgentAction::Disconnect { agent_id } => disconnect(&agent_id).await,
        AgentAction::Fleet => fleet().await,
    }
}

async fn list() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();

    // Try nexus first; fall back to local session files if offline
    match nexus.ensure_running().await {
        Ok(()) => list_from_nexus(&nexus).await,
        Err(_) => list_from_local_sessions().await,
    }
}

async fn list_from_nexus(nexus: &NexusClient) -> anyhow::Result<()> {
    let resp = nexus.get("/api/agents").await?;
    // Nexus returns { "agents": [...] } — unwrap the wrapper
    let agents = resp["agents"].as_array().cloned()
        .or_else(|| resp.as_array().cloned())
        .unwrap_or_default();

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
        // Support both camelCase (legacy) and snake_case (current API)
        let id = agent["agentId"].as_str()
            .or_else(|| agent["id"].as_str())
            .unwrap_or("?");
        let id_short = if id.len() > 12 { &id[..12] } else { id };

        let name = agent["name"].as_str().unwrap_or("?");
        let host = agent["host"].as_str()
            .or_else(|| agent["project_dir"].as_str())
            .unwrap_or("local");
        let status = agent["status"].as_str().unwrap_or("?");

        let models = agent["capabilities"]["models"]
            .as_array()
            .map(|m| {
                m.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| agent["model"].as_str().unwrap_or("").to_string());

        let status_colored = match status {
            "online" | "active" | "connected" | "running" => status.green().to_string(),
            "idle" | "spawning" => status.yellow().to_string(),
            "offline" | "disconnected" | "failed" => status.red().to_string(),
            "stale" | "completed" => status.dimmed().to_string(),
            _ => status.to_string(),
        };

        // Show [local] tag for auto-spawned agents (ADR-037)
        let name_display = if name.contains("(local)") {
            format!("{} {}", name.replace(" (local)", ""), "[local]".dimmed())
        } else {
            name.to_string()
        };

        println!(
            "  {:<14} {:<16} {:<20} {:<19} {}",
            id_short, name_display, host, status_colored, models,
        );
    }

    Ok(())
}

/// Fallback: read local session files when nexus is offline.
/// Provides visibility into Claude Code sessions even without the daemon.
async fn list_from_local_sessions() -> anyhow::Result<()> {
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hex/sessions");

    if !sessions_dir.exists() {
        println!("{} No agents connected.", "\u{2b21}".dimmed());
        println!(
            "  {} nexus is offline — no local session files found either",
            "\u{26a0}".yellow()
        );
        return Ok(());
    }

    let mut sessions: Vec<serde_json::Value> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with("agent-") || !name.ends_with(".json") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    sessions.push(val);
                }
            }
        }
    }

    if sessions.is_empty() {
        println!("{} No agents connected.", "\u{2b21}".dimmed());
        return Ok(());
    }

    // Sort by registeredAt descending (most recent first)
    sessions.sort_by(|a, b| {
        let ta = a["registeredAt"].as_str().unwrap_or("");
        let tb = b["registeredAt"].as_str().unwrap_or("");
        tb.cmp(ta)
    });

    println!(
        "{} Agents — {} (nexus offline, showing local sessions)",
        "\u{2b21}".cyan(),
        format!("{} sessions", sessions.len()).yellow(),
    );
    println!();
    println!(
        "  {:<14} {:<24} {:<12} {}",
        "ID".bold(),
        "SESSION".bold(),
        "STATUS".bold(),
        "REGISTERED".bold(),
    );
    println!("  {}", "\u{2500}".repeat(70).dimmed());

    for session in &sessions {
        let id = session["agentId"].as_str().unwrap_or("?");
        let id_short = if id.len() > 12 { &id[..12] } else { id };

        let session_id = session["sessionId"].as_str().unwrap_or("?");

        let registered = session["registeredAt"]
            .as_str()
            .unwrap_or("?");

        // Show a compact timestamp (strip date if today, keep time)
        let time_display = if registered.len() >= 16 {
            &registered[11..16] // HH:MM
        } else {
            registered
        };

        // Infer liveness: check if session file was modified recently (within 2 min)
        let status = {
            let session_file = sessions_dir.join(format!(
                "agent-{}.json",
                session_id
            ));
            match std::fs::metadata(&session_file) {
                Ok(meta) => {
                    if let Ok(modified) = meta.modified() {
                        let age = std::time::SystemTime::now()
                            .duration_since(modified)
                            .unwrap_or_default();
                        if age.as_secs() < 120 {
                            "recent".green().to_string()
                        } else if age.as_secs() < 3600 {
                            "stale".yellow().to_string()
                        } else {
                            "old".dimmed().to_string()
                        }
                    } else {
                        "unknown".dimmed().to_string()
                    }
                }
                Err(_) => "unknown".dimmed().to_string(),
            }
        };

        println!(
            "  {:<14} {:<24} {:<21} {}",
            id_short, session_id, status, time_display,
        );
    }

    println!();
    println!(
        "  {} Start nexus for live agent tracking: {}",
        "\u{2139}\u{fe0f}".dimmed(),
        "hex nexus start".bold()
    );

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

async fn agent_status(agent_id: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let path = format!("/api/agents/{}", agent_id);
    let agent = nexus.get(&path).await?;

    let status = agent["status"].as_str().unwrap_or("unknown");
    let status_colored = match status {
        "online" | "active" | "connected" => status.green().to_string(),
        "stale" | "idle" => status.yellow().to_string(),
        "dead" | "offline" | "disconnected" => status.red().to_string(),
        _ => status.to_string(),
    };

    println!("{} Agent Status", "\u{2b21}".cyan());
    println!();
    println!("  {:<22} {}", "Name:".bold(), agent["name"].as_str().unwrap_or("-"));
    println!("  {:<22} {}", "Host:".bold(), agent["host"].as_str().unwrap_or("-"));
    println!("  {:<22} {}", "Status:".bold(), status_colored);
    println!("  {:<22} {}", "Project Dir:".bold(), agent["project_dir"].as_str().unwrap_or("-"));
    println!("  {:<22} {}", "Tunnel ID:".bold(), agent["tunnel_id"].as_str().unwrap_or("-"));
    println!("  {:<22} {}", "Last Heartbeat:".bold(), agent["last_heartbeat"].as_str().unwrap_or("-"));
    println!("  {:<22} {}", "Connected At:".bold(), agent["connected_at"].as_str().unwrap_or("-"));

    // Models
    let models = agent["models"]
        .as_array()
        .map(|m| {
            m.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "-".to_string());
    println!("  {:<22} {}", "Models:".bold(), models);

    // Tools
    let tools = agent["tools"]
        .as_array()
        .map(|t| {
            t.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "-".to_string());
    println!("  {:<22} {}", "Tools:".bold(), tools);

    if let Some(max) = agent["max_concurrent_tasks"].as_u64() {
        println!("  {:<22} {}", "Max Concurrent Tasks:".bold(), max);
    }

    if let Some(vram) = agent["gpu_vram_mb"].as_u64() {
        println!("  {:<22} {} MB", "GPU VRAM:".bold(), vram);
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

async fn spawn_remote(
    target: &str,
    project_dir: Option<String>,
    source_dir: Option<String>,
) -> anyhow::Result<()> {
    // Parse user@host format
    let (user, host) = match target.split_once('@') {
        Some((u, h)) => (u.to_string(), h.to_string()),
        None => {
            anyhow::bail!(
                "Invalid target format: expected user@host (e.g. deploy@192.168.1.10), got '{}'",
                target
            );
        }
    };

    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let effective_project_dir = project_dir.unwrap_or_else(|| "~/project".to_string());

    println!(
        "{} Deploying hex-agent to {}...",
        "\u{2b21}".cyan(),
        target.bold()
    );
    println!("  Host:        {}", host);
    println!("  User:        {}", user);
    println!("  Project dir: {}", effective_project_dir);
    if let Some(ref sd) = source_dir {
        println!("  Source sync:  {}", sd);
    }
    println!();

    let mut body = json!({
        "host": host,
        "user": user,
        "projectDir": effective_project_dir,
    });

    if let Some(sd) = source_dir {
        body["remoteSourceDir"] = serde_json::Value::String(sd);
    }

    println!("{} Provisioning and launching agent...", "\u{2b21}".cyan());

    let resp = nexus.post("/api/agents/spawn-remote", &body).await?;

    if let Some(err) = resp.get("error") {
        let msg = err.as_str().unwrap_or("unknown error");
        eprintln!("{} Spawn failed: {}", "\u{2b21}".red(), msg);
        if msg.contains("tunnel") || msg.contains("SSH") || msg.contains("ssh") {
            eprintln!("  Hint: check that you can `ssh {}` without a password prompt", target);
        }
        if msg.contains("provision") || msg.contains("binary") {
            eprintln!("  Hint: ensure hex-agent is built on the remote or use --source-dir to sync sources");
        }
        anyhow::bail!("Remote agent spawn failed: {}", msg);
    }

    let agent_id = resp["agentId"].as_str().unwrap_or("-");
    let status = resp["status"].as_str().unwrap_or("online");
    let name = resp["name"].as_str().unwrap_or(target);

    println!("{} Remote agent deployed successfully", "\u{2b21}".green());
    println!("  Agent ID: {}", agent_id);
    println!("  Name:     {}", name);
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
