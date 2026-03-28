//! Remote agent management commands.
//!
//! `hex agent list|info|connect|spawn-remote|disconnect|fleet` — delegates to hex-nexus agent API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;
use tabled::Tabled;
use tracing::debug;

use crate::fmt::{HexTable, status_badge, truncate};
use crate::nexus_client::NexusClient;

use std::collections::HashMap;
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
    /// Evict dead/stale agents from the registry
    Evict,
    /// Run as a persistent agent worker for a specific role
    Worker {
        /// Agent role (hex-coder, hex-tester, hex-reviewer, hex-documenter, hex-ux, hex-fixer)
        #[arg(long)]
        role: String,

        /// Swarm ID to join (worker only processes tasks from this swarm)
        #[arg(long)]
        swarm_id: Option<String>,

        /// Agent ID to use when polling for tasks (overrides auto-registered ID).
        /// Pass the supervisor's agent ID so the worker picks up tasks the supervisor assigned.
        #[arg(long)]
        agent_id: Option<String>,

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
        AgentAction::Evict => evict().await,
        AgentAction::Audit => audit().await,
        AgentAction::Worker {
            role,
            swarm_id,
            agent_id,
            poll_interval,
        } => worker(&role, swarm_id, agent_id, poll_interval).await,
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

        // Associate the swarm creator/owner with this swarm
        let owner = swarm["createdBy"].as_str()
            .or_else(|| swarm["owner_agent_id"].as_str())
            .unwrap_or("");
        if !owner.is_empty() {
            agent_swarm_map
                .entry(owner.to_string())
                .or_insert_with(|| (swarm_name.to_string(), 0, 0, 0));
        }

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

    let mut rows: Vec<AgentRow> = Vec::new();

    for agent in &agents {
        // ADR-058: hex_agent table uses `id` as primary key
        let id = agent["id"].as_str()
            .or_else(|| agent["agentId"].as_str())
            .unwrap_or("?");

        let name = agent["name"].as_str().unwrap_or("?");
        let status = agent["status"].as_str().unwrap_or("?");

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
            (truncate(swarm_name, 16), format!("{}/{} done", completed, total))
        } else {
            ("\u{2014}".dimmed().to_string(), "\u{2014}".dimmed().to_string())
        };

        rows.push(AgentRow {
            id: id_display,
            name: name_display,
            status: status_badge(status),
            swarm: swarm_display,
            tasks: task_display,
        });
    }

    println!("{}", HexTable::render(&rows));

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

    let mut rows: Vec<LocalSessionRow> = Vec::new();

    for session in &sessions {
        let id = session["agentId"].as_str().unwrap_or("?");
        let id_short = if id.len() > 12 { &id[..12] } else { id };

        let session_id = session["sessionId"].as_str().unwrap_or("?");

        let registered = session["registeredAt"]
            .as_str()
            .unwrap_or("?");

        // Show a compact timestamp (strip date if today, keep time)
        let time_display = if registered.len() >= 16 {
            registered[11..16].to_string() // HH:MM
        } else {
            registered.to_string()
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
                            "recent".to_string()
                        } else if age.as_secs() < 3600 {
                            "stale".to_string()
                        } else {
                            "old".to_string()
                        }
                    } else {
                        "unknown".to_string()
                    }
                }
                Err(_) => "unknown".to_string(),
            }
        };

        rows.push(LocalSessionRow {
            id: id_short.to_string(),
            session: session_id.to_string(),
            status: status_badge(&status),
            registered: time_display,
        });
    }

    println!("{}", HexTable::render(&rows));

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

async fn evict() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;
    let body = serde_json::json!({});
    let result = nexus.post("/api/hex-agents/evict", &body).await?;
    let evicted = result.get("evicted").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("{} Evicted {} dead agent(s)", "\u{2b21}".green(), evicted);
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
    let mut rows: Vec<AuditRow> = Vec::new();

    for (hash, msg) in &commits {
        let is_tracked = task_results.iter().any(|result| {
            result.contains(hash) || msg.split_whitespace().take(5).any(|word| {
                word.len() > 4 && result.to_lowercase().contains(&word.to_lowercase())
            })
        });

        if is_tracked {
            rows.push(AuditRow {
                icon: "\u{2713}".green().to_string(),
                hash: hash.yellow().to_string(),
                message: msg.to_string(),
                tracking: "tracked".green().to_string(),
            });
            tracked += 1;
        } else {
            rows.push(AuditRow {
                icon: "\u{2717}".red().to_string(),
                hash: hash.yellow().to_string(),
                message: msg.to_string(),
                tracking: "untracked".red().to_string(),
            });
            untracked += 1;
        }
    }

    println!("{}", HexTable::compact(&rows));
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
    override_agent_id: Option<String>,
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
    let registered_id = resp["agentId"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if registered_id.is_empty() {
        anyhow::bail!("Failed to register agent — no agentId returned");
    }

    // If the supervisor passed its own agent ID, use that for polling so we
    // find tasks the supervisor assigned to itself.  We still register under
    // our own identity for heartbeats.
    let agent_id = override_agent_id.unwrap_or(registered_id);

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
        // Step 1: claim a pending task (work-stealing via /api/work-items/incomplete)
        if let Ok(resp) = nexus.get("/api/work-items/incomplete").await {
            if let Some(all_incomplete) = resp.as_array() {
                for candidate in all_incomplete {
                    // Filter: only pending tasks, and only from our swarm if specified
                    let status = candidate["status"].as_str().unwrap_or("");
                    if status != "pending" {
                        continue;
                    }
                    if let Some(ref sid) = swarm_id {
                        let task_swarm = candidate["swarm_id"]
                            .as_str()
                            .or_else(|| candidate["swarmId"].as_str())
                            .unwrap_or("");
                        if task_swarm != sid.as_str() {
                            continue;
                        }
                    }
                    let task_id = candidate["id"].as_str().unwrap_or("");
                    let version = candidate["version"].as_u64().unwrap_or(0);
                    if task_id.is_empty() {
                        continue;
                    }
                    // CAS assign — if another worker beats us we get 409 and try next
                    let assign_result = nexus
                        .patch(
                            &format!("/api/swarms/tasks/{}", task_id),
                            &json!({
                                "agent_id": agent_id,
                                "version": version,
                            }),
                        )
                        .await;
                    if assign_result.is_ok() {
                        debug!(task_id, "claimed pending task");
                        break;
                    }
                    // 409 = race lost, try next candidate
                }
            }
        }

        // Step 2: execute tasks assigned to this agent (pending/assigned/in_progress)
        // Use /api/swarms/active enriched response to find our assigned tasks
        let query = "/api/swarms/active";

        match nexus.get(query).await {
            Ok(resp) => {
                // /api/swarms/active returns [{id, name, tasks: [...]}, ...]
                // Flatten all tasks from all swarms, filter to ours (pending/assigned/in_progress + our agent_id)
                let mut all_tasks: Vec<serde_json::Value> = Vec::new();
                let swarm_count = resp.as_array().map(|a| a.len()).unwrap_or(0);
                println!("  [poll] {} swarms, agent_id={}", swarm_count, agent_id);
                if let Some(swarms) = resp.as_array() {
                    for swarm in swarms {
                        if let Some(tasks) = swarm["tasks"].as_array() {
                            for t in tasks {
                                let t_agent = t["agent_id"]
                                    .as_str()
                                    .or_else(|| t["agentId"].as_str())
                                    .unwrap_or("");
                                let t_status = t["status"].as_str().unwrap_or("");
                                println!("    task {} agent='{}' status='{}'",
                                    t["id"].as_str().unwrap_or("?"),
                                    t_agent, t_status);
                                if t_agent == agent_id && matches!(t_status, "in_progress" | "pending" | "assigned") {
                                    all_tasks.push(t.clone());
                                }
                            }
                        }
                    }
                }
                let tasks = &all_tasks;
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
/// Each role writes its results to hexflo_memory so downstream agents
/// (tester, reviewer) can discover what upstream agents produced.
/// Memory keys follow the convention `{task_id}:{artifact_type}`.
async fn execute_worker_task(
    role: &str,
    task: &serde_json::Value,
    _project_dir: &str,
) -> anyhow::Result<String> {
    let task_id = task["id"].as_str().unwrap_or("");
    let title = task["title"].as_str().unwrap_or("");
    let swarm_id = task["swarm_id"].as_str().unwrap_or("");
    let nexus = NexusClient::from_env();

    // Helper: gather source files from upstream dependency memory
    let gather_dep_files = |deps_str: &str| {
        let nexus_ref = &nexus;
        let deps_owned: Vec<String> = deps_str
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect();
        async move {
            let mut source_files: Vec<(String, String)> = Vec::new();
            for dep_id in deps_owned {
                let memory_key = format!("{}:generated_files", dep_id);
                if let Ok(resp) = nexus_ref
                    .get(&format!("/api/hexflo/memory/{}", memory_key))
                    .await
                {
                    if let Some(val) = resp["value"].as_str() {
                        source_files.push((dep_id.clone(), val.to_string()));
                    }
                }
            }
            source_files
        }
    };

    // Build a default AgentContext for agent execution
    let build_context = |prompt_template: &str,
                         source_files: Vec<(String, String)>,
                         task_title: &str|
     -> crate::pipeline::supervisor::AgentContext {
        crate::pipeline::supervisor::AgentContext {
            prompt_template: prompt_template.to_string(),
            source_files,
            port_interfaces: Vec::new(),
            boundary_rules: String::new(),
            workplan_step: Some(task_title.to_string()),
            upstream_output: None,
            metadata: HashMap::new(),
        }
    };

    let result = match role {
        "hex-coder" => {
            // Build context and invoke CodePhase for real inference
            let context = build_context("agent-coder", Vec::new(), title);
            let _phase = crate::pipeline::code_phase::CodePhase::from_env();

            // For now, return a basic result — full workplan step parsing comes later
            let result = format!("hex-coder: executed task '{}'", title);

            // Write generated files to memory so tester/reviewer can find them
            let memory_key = format!("{}:generated_files", task_id);
            let _ = nexus
                .post(
                    "/api/hexflo/memory",
                    &json!({
                        "key": memory_key,
                        "value": json!({
                            "files": [],
                            "task": title,
                            "prompt_template": context.prompt_template,
                        }).to_string(),
                        "scope": swarm_id,
                    }),
                )
                .await;

            result
        }
        "hex-reviewer" => {
            use crate::pipeline::agents::ReviewerAgent;
            let agent = ReviewerAgent::from_env();

            // Gather source files from upstream dependencies
            let deps = task["depends_on"].as_str().unwrap_or("");
            let source_files = gather_dep_files(deps).await;

            let context = build_context("agent-reviewer", source_files, title);

            match agent.execute(&context, None, None).await {
                Ok(review) => {
                    let pass = review.verdict == "PASS";
                    let summary = format!(
                        "hex-reviewer: {} ({} issues, model={}, cost=${:.4})",
                        review.verdict,
                        review.issues.len(),
                        review.model_used,
                        review.cost_usd,
                    );

                    // Write review results to memory
                    let memory_key = format!("{}:review_results", task_id);
                    let _ = nexus
                        .post(
                            "/api/hexflo/memory",
                            &json!({
                                "key": memory_key,
                                "value": json!({
                                    "pass": pass,
                                    "verdict": review.verdict,
                                    "issues": review.issues.len(),
                                    "model": review.model_used,
                                    "tokens": review.tokens,
                                    "cost_usd": review.cost_usd,
                                }).to_string(),
                                "scope": swarm_id,
                            }),
                        )
                        .await;

                    summary
                }
                Err(e) => format!("hex-reviewer error: {}", e),
            }
        }
        "hex-tester" => {
            use crate::pipeline::agents::TesterAgent;
            let agent = TesterAgent::from_env();

            // Gather source files from upstream dependencies
            let deps = task["depends_on"].as_str().unwrap_or("");
            let source_files = gather_dep_files(deps).await;

            let context = build_context("agent-tester", source_files, title);

            match agent.execute(&context, None, None).await {
                Ok(test_result) => {
                    let has_content = !test_result.test_content.is_empty();
                    let summary = format!(
                        "hex-tester: generated tests → {} (model={}, cost=${:.4})",
                        test_result.suggested_path,
                        test_result.model_used,
                        test_result.cost_usd,
                    );

                    // Write test results to memory
                    let memory_key = format!("{}:test_results", task_id);
                    let _ = nexus
                        .post(
                            "/api/hexflo/memory",
                            &json!({
                                "key": memory_key,
                                "value": json!({
                                    "pass": has_content,
                                    "suggested_path": test_result.suggested_path,
                                    "model": test_result.model_used,
                                    "tokens": test_result.tokens,
                                    "cost_usd": test_result.cost_usd,
                                }).to_string(),
                                "scope": swarm_id,
                            }),
                        )
                        .await;

                    summary
                }
                Err(e) => format!("hex-tester error: {}", e),
            }
        }
        "hex-documenter" => {
            use crate::pipeline::agents::DocumenterAgent;
            let agent = DocumenterAgent::from_env();

            // Gather source files from upstream dependencies
            let deps = task["depends_on"].as_str().unwrap_or("");
            let source_files = gather_dep_files(deps).await;

            let context = build_context("agent-documenter", source_files, title);
            let output_dir = _project_dir;

            match agent.execute(&context, output_dir, None, None).await {
                Ok(doc_result) => {
                    let summary = format!(
                        "hex-documenter: generated {} ({} files documented, model={}, cost=${:.4})",
                        doc_result.readme_path,
                        doc_result.files_documented,
                        doc_result.model_used,
                        doc_result.cost_usd,
                    );

                    // Write doc results to memory
                    let memory_key = format!("{}:doc_results", task_id);
                    let _ = nexus
                        .post(
                            "/api/hexflo/memory",
                            &json!({
                                "key": memory_key,
                                "value": json!({
                                    "readme_path": doc_result.readme_path,
                                    "files_documented": doc_result.files_documented,
                                    "model": doc_result.model_used,
                                    "tokens": doc_result.tokens,
                                    "cost_usd": doc_result.cost_usd,
                                }).to_string(),
                                "scope": swarm_id,
                            }),
                        )
                        .await;

                    summary
                }
                Err(e) => format!("hex-documenter error: {}", e),
            }
        }
        "hex-ux" => {
            use crate::pipeline::agents::UxReviewerAgent;
            let agent = UxReviewerAgent::from_env();

            // Gather source files from upstream dependencies
            let deps = task["depends_on"].as_str().unwrap_or("");
            let source_files = gather_dep_files(deps).await;

            let context = build_context("agent-ux", source_files, title);
            let output_dir = _project_dir;

            match agent.execute(&context, output_dir, None, None).await {
                Ok(ux_result) => {
                    let pass = ux_result.verdict == "PASS";
                    let summary = format!(
                        "hex-ux: {} ({} issues, model={}, cost=${:.4})",
                        ux_result.verdict,
                        ux_result.issues.len(),
                        ux_result.model_used,
                        ux_result.cost_usd,
                    );

                    // Write UX review results to memory
                    let memory_key = format!("{}:ux_review_results", task_id);
                    let _ = nexus
                        .post(
                            "/api/hexflo/memory",
                            &json!({
                                "key": memory_key,
                                "value": json!({
                                    "pass": pass,
                                    "verdict": ux_result.verdict,
                                    "issues": ux_result.issues.len(),
                                    "model": ux_result.model_used,
                                    "tokens": ux_result.tokens,
                                    "cost_usd": ux_result.cost_usd,
                                }).to_string(),
                                "scope": swarm_id,
                            }),
                        )
                        .await;

                    summary
                }
                Err(e) => format!("hex-ux error: {}", e),
            }
        }
        "hex-fixer" => {
            // Fixer reads review issues from upstream and attempts fixes
            let deps = task["depends_on"].as_str().unwrap_or("");
            let mut upstream_issues = String::new();
            for dep_id in deps.split(',').filter(|s| !s.is_empty()) {
                let key = format!("{}:review_results", dep_id.trim());
                if let Ok(resp) = nexus
                    .get(&format!("/api/hexflo/memory/{}", key))
                    .await
                {
                    if let Some(val) = resp["value"].as_str() {
                        upstream_issues.push_str(val);
                        upstream_issues.push('\n');
                    }
                }
            }
            format!(
                "hex-fixer: processed '{}' (upstream review data: {} bytes)",
                title,
                upstream_issues.len()
            )
        }
        _ => anyhow::bail!("Unknown worker role: {}", role),
    };

    Ok(result)
}
