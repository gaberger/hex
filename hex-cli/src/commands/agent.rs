//! Remote agent management commands.
//!
//! `hex agent list|info|connect|spawn-remote|disconnect|fleet` — delegates to hex-nexus agent API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;
use tabled::Tabled;

use crate::fmt::{HexTable, status_badge, truncate};
use crate::nexus_client::NexusClient;

use std::path::PathBuf;

#[derive(Subcommand)]
pub enum AgentAction {
    /// Show the current agent's ID (who am I?)
    Id,
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
    /// Audit recent commits against HexFlo task tracking (ADR-2603221939)
    Audit,
    /// Run as a persistent agent worker for a specific role
    Worker {
        /// Agent role (hex-coder, hex-tester, hex-reviewer, hex-documenter, hex-ux, hex-fixer)
        #[arg(long)]
        role: String,

        /// Swarm ID to join (worker only processes tasks from this swarm)
        #[arg(long)]
        swarm_id: Option<String>,

        /// Poll interval in seconds (default 5)
        #[arg(long, default_value_t = 5)]
        poll_interval: u64,
    },
}

// ── Tabled row types ───────────────────────────────────────────────────

#[derive(Tabled)]
struct AgentRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Swarm")]
    swarm: String,
    #[tabled(rename = "Tasks")]
    tasks: String,
}

#[derive(Tabled)]
struct LocalSessionRow {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Session")]
    session: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Registered")]
    registered: String,
}

#[derive(Tabled)]
struct AuditRow {
    #[tabled(rename = "")]
    icon: String,
    #[tabled(rename = "Commit")]
    hash: String,
    #[tabled(rename = "Message")]
    message: String,
    #[tabled(rename = "Tracking")]
    tracking: String,
}

pub async fn run(action: AgentAction) -> anyhow::Result<()> {
    match action {
        AgentAction::Id => show_agent_id().await,
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
        AgentAction::Audit => audit().await,
        AgentAction::Worker {
            role,
            swarm_id,
            poll_interval,
        } => worker(&role, swarm_id, poll_interval).await,
    }
}

async fn show_agent_id() -> anyhow::Result<()> {
    use crate::nexus_client::resolve_agent_id_detailed;

    match resolve_agent_id_detailed() {
        Some(resolved) => {
            let short_id = if resolved.agent_id.len() >= 8 {
                &resolved.agent_id[..8]
            } else {
                &resolved.agent_id
            };

            println!("{} Agent Identity", "\u{2b21}".cyan());
            println!();
            println!("  {:<18} {}", "Agent ID:".bold(), resolved.agent_id);
            println!("  {:<18} {}", "Short ID:".bold(), short_id);
            println!("  {:<18} {}", "Resolved via:".bold(), resolved.method);

            if let Some(ref path) = resolved.session_file {
                println!("  {:<18} {}", "Session file:".bold(), path.display());
            }

            // Show session data if available
            if let Some(ref data) = resolved.session_data {
                if let Some(name) = data["name"].as_str() {
                    println!("  {:<18} {}", "Name:".bold(), name);
                }
                if let Some(project) = data["project"].as_str().filter(|s| !s.is_empty()) {
                    println!("  {:<18} {}", "Project:".bold(), project);
                }
                if let Some(pid) = data["claude_pid"].as_u64() {
                    println!("  {:<18} {}", "Claude PID:".bold(), pid);
                }
                if let Some(heartbeat) = data["last_heartbeat"]
                    .as_str()
                    .or_else(|| data["registered_at"].as_str())
                    .or_else(|| data["registeredAt"].as_str())
                {
                    println!("  {:<18} {}", "Last seen:".bold(), heartbeat);
                }
            }

            // Fetch live status from nexus if available
            let nexus = NexusClient::from_env();
            if nexus.ensure_running().await.is_ok() {
                let path = format!("/api/hex-agents/{}", resolved.agent_id);
                match nexus.get(&path).await {
                    Ok(agent) => {
                        let status = agent["status"].as_str().unwrap_or("unknown");
                        let status_colored = match status {
                            "online" | "active" | "connected" => status.green().to_string(),
                            "stale" | "idle" => status.yellow().to_string(),
                            "dead" | "offline" => status.red().to_string(),
                            _ => status.to_string(),
                        };
                        println!();
                        println!("  {:<18} {} {}", "Nexus status:".bold(), status_colored, "(live)".dimmed());
                    }
                    Err(_) => {
                        // ADR-065: auto-reconnect — re-register with nexus using session data
                        println!();
                        println!(
                            "  {:<18} {} {}",
                            "Nexus status:".bold(),
                            "unregistered".yellow(),
                            "(reconnecting...)".dimmed()
                        );

                        let hostname = resolved.session_data.as_ref()
                            .and_then(|d| d["name"].as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let project_dir = resolved.session_data.as_ref()
                            .and_then(|d| d["project_dir"].as_str())
                            .unwrap_or("")
                            .to_string();
                        // Use CWD if session didn't have project_dir
                        let project_dir = if project_dir.is_empty() {
                            std::env::current_dir()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default()
                        } else {
                            project_dir
                        };

                        let reconnect_body = serde_json::json!({
                            "host": hostname,
                            "project_dir": project_dir,
                            "agent_id": resolved.agent_id,
                            "capabilities": {
                                "models": [],
                                "maxConcurrent": 4,
                            },
                        });

                        match nexus.post("/api/hex-agents/connect", &reconnect_body).await {
                            Ok(resp) => {
                                let new_id = resp["agentId"].as_str().unwrap_or(&resolved.agent_id);
                                let id_changed = new_id != resolved.agent_id;
                                println!(
                                    "  {:<18} {} {}",
                                    "".bold(),
                                    "reconnected".green(),
                                    "(re-registered with nexus)".dimmed()
                                );
                                if id_changed {
                                    println!(
                                        "  {:<18} {} → {}",
                                        "New Agent ID:".bold(),
                                        new_id,
                                        "(updated session file)".dimmed()
                                    );
                                }
                                // Update session file with server-assigned ID + new data
                                if let Some(ref file_path) = resolved.session_file {
                                    if let Some(mut data) = resolved.session_data.clone() {
                                        let now = chrono::Utc::now().to_rfc3339();
                                        data["agentId"] = serde_json::Value::String(new_id.to_string());
                                        data["last_heartbeat"] = serde_json::Value::String(now);
                                        if let Some(pid) = resp["projectId"].as_str() {
                                            data["project"] = serde_json::Value::String(pid.to_string());
                                        }
                                        let _ = std::fs::write(
                                            file_path,
                                            serde_json::to_string_pretty(&data).unwrap_or_default(),
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                println!(
                                    "  {:<18} {} {}",
                                    "".bold(),
                                    "failed".red(),
                                    format!("({})", e).dimmed()
                                );
                            }
                        }
                    }
                }
            }
        }
        None => {
            eprintln!("{} Cannot resolve agent ID", "\u{2b21}".red());
            eprintln!();
            eprintln!("  No agent identity found. Resolution tried:");
            eprintln!("    1. CLAUDE_SESSION_ID env var  {}", "(not set)".dimmed());
            eprintln!("    2. HEX_AGENT_ID env var       {}", "(not set)".dimmed());
            eprintln!("    3. claude_pid PPID chain       {}", "(no match)".dimmed());
            eprintln!("    4. Newest session file         {}", "(none within 2h)".dimmed());
            eprintln!();
            eprintln!("  To fix, try one of:");
            eprintln!("    {} Connect to nexus", "hex agent connect <nexus-url>".bold());
            eprintln!("    {} Set manually", "export HEX_AGENT_ID=<uuid>".bold());
            anyhow::bail!("Agent ID resolution failed");
        }
    }

    Ok(())
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
    let resp = nexus.get("/api/hex-agents").await?;
    // Nexus returns { "agents": [...] } — unwrap the wrapper
    let agents = resp["agents"].as_array().cloned()
        .or_else(|| resp.as_array().cloned())
        .unwrap_or_default();

    if agents.is_empty() {
        println!("{} No agents connected.", "\u{2b21}".dimmed());
        return Ok(());
    }

    // Cross-reference: fetch active swarms to show agent→swarm mapping
    let swarms_resp = nexus.get("/api/swarms/active").await.ok();
    let swarms = swarms_resp
        .as_ref()
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    // Build agent→swarm lookup: agent_id → (swarm_name, pending, completed, total)
    let mut agent_swarm_map: std::collections::HashMap<String, (String, usize, usize, usize)> =
        std::collections::HashMap::new();
    for swarm in &swarms {
        let swarm_name = swarm["name"].as_str().unwrap_or("-");
        if let Some(tasks) = swarm["tasks"].as_array() {
            for task in tasks {
                let agent_id = task["agentId"].as_str()
                    .or_else(|| task["agent_id"].as_str())
                    .unwrap_or("");
                if agent_id.is_empty() {
                    continue;
                }
                let status = task["status"].as_str().unwrap_or("pending");
                let entry = agent_swarm_map
                    .entry(agent_id.to_string())
                    .or_insert_with(|| (swarm_name.to_string(), 0, 0, 0));
                entry.3 += 1; // total
                match status {
                    "completed" => entry.2 += 1,
                    "pending" => entry.1 += 1,
                    _ => {} // in_progress, failed, etc.
                }
            }
        }
    }

    // Resolve our own agent ID to mark it in the list
    let my_id = crate::nexus_client::read_session_agent_id();

    println!("{} Agents ({})", "\u{2b21}".cyan(), agents.len());
    println!();
    println!(
        "  {:<38} {:<16} {:<10} {:<18} {}",
        "ID".bold(),
        "NAME".bold(),
        "STATUS".bold(),
        "SWARM".bold(),
        "TASKS".bold(),
    );
    println!("  {}", "\u{2500}".repeat(100).dimmed());

    for agent in &agents {
        // ADR-058: hex_agent table uses `id` as primary key
        let id = agent["id"].as_str()
            .or_else(|| agent["agentId"].as_str())
            .unwrap_or("?");

        let name = agent["name"].as_str().unwrap_or("?");
        let status = agent["status"].as_str().unwrap_or("?");

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

        // Mark our own agent with an arrow
        let is_me = my_id.as_deref() == Some(id);
        let id_display = if is_me {
            format!("{} {}", id, "\u{25c0} you".cyan())
        } else {
            id.to_string()
        };

        // Agent→swarm cross-reference
        let (swarm_display, task_display) = if let Some((swarm_name, _pending, completed, total)) =
            agent_swarm_map.get(id)
        {
            let swarm_short = if swarm_name.len() > 16 {
                format!("{}…", &swarm_name[..15])
            } else {
                swarm_name.clone()
            };
            let tasks = format!("{}/{} done", completed, total);
            (swarm_short, tasks)
        } else {
            ("—".dimmed().to_string(), "—".dimmed().to_string())
        };

        println!(
            "  {:<50} {:<16} {:<19} {:<18} {}",
            id_display, name_display, status_colored, swarm_display, task_display,
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

    let path = format!("/api/hex-agents/{}", agent_id);
    let agent = nexus.get(&path).await?;

    println!("{} Agent Details", "\u{2b21}".cyan());
    println!();
    println!("  {:<16} {}", "ID:".bold(), agent["id"].as_str().unwrap_or("-"));
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

    let path = format!("/api/hex-agents/{}", agent_id);
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

    // ADR-065: send project_dir (CWD) and generated session_id
    let project_dir = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let session_id = format!(
        "connect-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );

    let body = json!({
        "host": hostname,
        "project_dir": project_dir,
        "session_id": session_id,
        "capabilities": {
            "models": [],
            "maxConcurrent": 4,
        },
    });

    // ADR-058: Use unified agent registry, not legacy orchestration endpoint
    let resp = nexus.post("/api/hex-agents/connect", &body).await?;

    let agent_id = resp["agentId"].as_str().unwrap_or("-");
    let project_id = resp["projectId"].as_str().unwrap_or("");
    let project_name = if !project_dir.is_empty() {
        std::path::Path::new(&project_dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };

    // ADR-065 P1: Write session file so subsequent CLI commands can resolve agent ID
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".hex/sessions");
    std::fs::create_dir_all(&sessions_dir)?;

    let now = chrono::Utc::now().to_rfc3339();
    let session_data = json!({
        "agentId": agent_id,
        "name": format!("claude-{}", hostname),
        "project": if project_id.is_empty() { &project_name } else { project_id },
        "project_dir": project_dir,
        "registered_at": now,
        "last_heartbeat": now,
        "session_id": session_id,
        "nexus_url": nexus_url,
    });

    let session_file = sessions_dir.join(format!("agent-{}.json", session_id));
    let tmp_file = sessions_dir.join(format!(".agent-{}.json.tmp", session_id));
    std::fs::write(&tmp_file, serde_json::to_string_pretty(&session_data)?)?;
    std::fs::rename(&tmp_file, &session_file)?;

    println!("{} Connected to nexus", "\u{2b21}".green());
    println!("  Nexus URL:     {}", nexus_url);
    println!("  Agent ID:      {}", agent_id);
    println!("  Host:          {}", hostname);
    println!("  Project:       {}", if project_name.is_empty() { "-" } else { &project_name });
    println!("  Session file:  {}", session_file.display());

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

    // ADR-058: use unified agent registry endpoint
    let path = format!("/api/hex-agents/{}", agent_id);
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

/// Audit recent commits against HexFlo task completions (ADR-2603221939 P5).
///
/// Cross-references `git log` with completed HexFlo tasks to find "dark agents" —
/// commits produced by AI agents that were not tracked in any swarm.
async fn audit() -> anyhow::Result<()> {
    use std::process::Command;

    println!("{} Agent Audit — tracking compliance", "\u{2b21}".cyan());
    println!();

    // 1. Get recent Co-Authored-By commits (AI-produced)
    let git_output = Command::new("git")
        .args(["log", "--oneline", "-20", "--grep=Co-Authored-By"])
        .output()?;
    let git_log = String::from_utf8_lossy(&git_output.stdout);

    let commits: Vec<(&str, &str)> = git_log
        .lines()
        .filter_map(|line| {
            let (hash, msg) = line.split_once(' ')?;
            Some((hash, msg))
        })
        .collect();

    if commits.is_empty() {
        println!("  {} No AI-authored commits found in last 20 commits", "\u{25cb}".dimmed());
        return Ok(());
    }

    // 2. Get completed tasks from HexFlo
    let nexus = NexusClient::from_env();
    let task_results: Vec<String> = if nexus.ensure_running().await.is_ok() {
        match nexus.get("/api/hexflo/tasks").await {
            Ok(data) => {
                data["tasks"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter(|t| t["status"].as_str() == Some("completed"))
                    .filter_map(|t| t["result"].as_str().map(String::from))
                    .collect()
            }
            Err(_) => Vec::new(),
        }
    } else {
        println!("  {} nexus unreachable — comparing against local task data only", "\u{26a0}".yellow());
        Vec::new()
    };

    // 3. Cross-reference: is each commit's hash or message mentioned in any task result?
    let mut tracked = 0u32;
    let mut untracked = 0u32;

    for (hash, msg) in &commits {
        let is_tracked = task_results.iter().any(|result| {
            result.contains(hash) || msg.split_whitespace().take(5).any(|word| {
                word.len() > 4 && result.to_lowercase().contains(&word.to_lowercase())
            })
        });

        if is_tracked {
            println!("  {} {} {}", "\u{2713}".green(), hash.yellow(), msg);
            tracked += 1;
        } else {
            println!("  {} {} {} {}", "\u{2717}".red(), hash.yellow(), msg, "(untracked)".red());
            untracked += 1;
        }
    }

    println!();
    println!(
        "  {} tracked, {} untracked (of {} AI commits)",
        tracked.to_string().green(),
        if untracked > 0 { untracked.to_string().red().to_string() } else { "0".to_string() },
        commits.len()
    );

    if untracked > 0 {
        println!();
        println!(
            "  {} Untracked commits indicate agents that bypassed HexFlo swarm tracking.",
            "\u{26a0}".yellow()
        );
        println!("    Ensure all background agents include HEXFLO_TASK:{{uuid}} in their prompt.");
    }

    Ok(())
}

/// Run as a persistent agent worker that polls for and executes tasks.
///
/// The worker registers with nexus, sends heartbeats every 30s, polls for
/// assigned tasks, executes them based on its role, and writes results back.
/// Runs until SIGTERM.
async fn worker(
    role: &str,
    swarm_id: Option<String>,
    poll_interval: u64,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Register as a role-specific agent
    let hostname = gethostname::gethostname().to_string_lossy().to_string();
    let agent_name = format!("{}-{}", role, &hostname);
    let project_dir = std::env::current_dir()?.to_string_lossy().to_string();
    let session_id = std::env::var("CLAUDE_SESSION_ID")
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());

    let reg_body = json!({
        "name": agent_name,
        "host": hostname,
        "project_dir": project_dir,
        "session_id": session_id,
        "capabilities": [role],
    });
    let resp = nexus.post("/api/hex-agents/connect", &reg_body).await?;
    let agent_id = resp["agentId"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if agent_id.is_empty() {
        anyhow::bail!("Failed to register agent — no agentId returned");
    }

    let short_id = &agent_id[..8.min(agent_id.len())];
    println!(
        "{} Worker started: {} (agent: {})",
        "\u{2b21}".green(),
        agent_name,
        short_id
    );
    println!("  Role:     {}", role);
    println!(
        "  Swarm:    {}",
        swarm_id.as_deref().unwrap_or("any")
    );
    println!("  Poll:     {}s", poll_interval);

    // Set up heartbeat interval (every 30s)
    let heartbeat_nexus = NexusClient::from_env();
    let heartbeat_id = agent_id.clone();
    let _heartbeat_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let _ = heartbeat_nexus
                .post(
                    "/api/hex-agents/heartbeat",
                    &json!({ "agent_id": heartbeat_id }),
                )
                .await;
        }
    });

    // Main task poll loop
    let poll_duration = std::time::Duration::from_secs(poll_interval);
    loop {
        let query = if let Some(ref sid) = swarm_id {
            format!(
                "/api/swarms/tasks?agent_id={}&status=in_progress&swarm_id={}",
                agent_id, sid
            )
        } else {
            format!("/api/swarms/tasks?agent_id={}&status=in_progress", agent_id)
        };

        match nexus.get(&query).await {
            Ok(resp) => {
                if let Some(tasks) =
                    resp.as_array().or_else(|| resp["tasks"].as_array())
                {
                    for task in tasks {
                        let task_id = task["id"].as_str().unwrap_or("");
                        let title = task["title"].as_str().unwrap_or("");
                        if task_id.is_empty() {
                            continue;
                        }

                        let tid_short = &task_id[..8.min(task_id.len())];
                        println!(
                            "  {} Executing task: {} — {}",
                            "\u{2192}".cyan(),
                            tid_short,
                            title
                        );

                        // Execute based on role
                        let result =
                            execute_worker_task(role, task, &project_dir).await;

                        // Write result back
                        match result {
                            Ok(summary) => {
                                let _ = nexus
                                    .patch(
                                        &format!("/api/swarms/tasks/{}", task_id),
                                        &json!({
                                            "status": "completed",
                                            "result": summary,
                                        }),
                                    )
                                    .await;
                                println!(
                                    "  {} Task completed: {}",
                                    "\u{2713}".green(),
                                    tid_short
                                );
                            }
                            Err(e) => {
                                let _ = nexus
                                    .patch(
                                        &format!("/api/swarms/tasks/{}", task_id),
                                        &json!({
                                            "status": "failed",
                                            "result": format!("Error: {}", e),
                                        }),
                                    )
                                    .await;
                                println!(
                                    "  {} Task failed: {} — {}",
                                    "\u{2717}".red(),
                                    tid_short,
                                    e
                                );
                            }
                        }
                    }
                }
            }
            Err(_) => {
                // Nexus unreachable — wait and retry
            }
        }

        tokio::time::sleep(poll_duration).await;
    }

    // Cleanup (unreachable in normal flow, but for completeness)
    #[allow(unreachable_code)]
    {
        _heartbeat_handle.abort();
        let _ = nexus.delete(&format!("/api/hex-agents/{}", agent_id)).await;
        Ok(())
    }
}

/// Execute a single task based on the worker's role.
///
/// This is a stub implementation — returns placeholder results for now.
/// Real agent dispatch (via inference + prompt templates) will be wired in later.
async fn execute_worker_task(
    role: &str,
    task: &serde_json::Value,
    _project_dir: &str,
) -> anyhow::Result<String> {
    let title = task["title"].as_str().unwrap_or("");

    match role {
        "hex-coder" => Ok(format!("hex-coder: processed '{}'", title)),
        "hex-tester" => Ok(format!("hex-tester: processed '{}'", title)),
        "hex-reviewer" => Ok(format!("hex-reviewer: processed '{}'", title)),
        "hex-documenter" => Ok(format!("hex-documenter: processed '{}'", title)),
        "hex-ux" => Ok(format!("hex-ux: processed '{}'", title)),
        "hex-fixer" => Ok(format!("hex-fixer: processed '{}'", title)),
        _ => anyhow::bail!("Unknown worker role: {}", role),
    }
}
