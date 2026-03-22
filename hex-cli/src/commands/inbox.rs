//! Agent notification inbox commands (ADR-060).
//!
//! `hex inbox list|notify|ack|expire` — delegates to hex-nexus inbox API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum InboxAction {
    /// List notifications for the current agent
    List {
        /// Minimum priority (0=info, 1=warning, 2=critical)
        #[arg(long, default_value = "0")]
        min_priority: u8,
        /// Show all (including acknowledged)
        #[arg(long)]
        all: bool,
    },
    /// Send a notification to an agent or project
    Notify {
        /// Target agent ID (use --project for broadcast)
        #[arg(long)]
        agent: Option<String>,
        /// Target project ID (broadcasts to all agents)
        #[arg(long)]
        project: Option<String>,
        /// Priority: 0=info, 1=warning, 2=critical
        #[arg(short, long, default_value = "1")]
        priority: u8,
        /// Notification kind (restart, update, shutdown, config_change, info)
        kind: String,
        /// JSON payload
        #[arg(default_value = "{}")]
        payload: String,
    },
    /// Acknowledge a notification
    Ack {
        /// Notification ID
        id: u64,
    },
    /// Expire stale notifications (older than 24h)
    Expire,
}

pub async fn run(action: InboxAction) -> anyhow::Result<()> {
    match action {
        InboxAction::List { min_priority, all } => list(min_priority, all).await,
        InboxAction::Notify { agent, project, priority, kind, payload } => {
            notify(agent, project, priority, &kind, &payload).await
        }
        InboxAction::Ack { id } => ack(id).await,
        InboxAction::Expire => expire().await,
    }
}

async fn list(min_priority: u8, all: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let agent_id = resolve_agent_id()?;
    let unacked = if all { "false" } else { "true" };
    let path = format!(
        "/api/hexflo/inbox/{}?min_priority={}&unacked_only={}",
        agent_id, min_priority, unacked
    );

    let resp = nexus.get(&path).await?;
    let notifications = resp["notifications"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    if notifications.is_empty() {
        println!("{} Inbox empty", "\u{2b21}".dimmed());
        return Ok(());
    }

    println!(
        "{} Inbox: {} notification(s)",
        "\u{2b21}".cyan(),
        notifications.len()
    );
    println!();

    for n in &notifications {
        let id = n["id"].as_u64().unwrap_or(0);
        let priority = n["priority"].as_u64().unwrap_or(0);
        let kind = n["kind"].as_str().unwrap_or("-");
        let created = n["createdAt"].as_str().unwrap_or("");
        let acked = n["acknowledgedAt"].as_str();

        let priority_label = match priority {
            0 => "info".dimmed(),
            1 => "warn".yellow(),
            _ => "CRIT".red().bold(),
        };

        let status = if acked.is_some() { " [acked]".dimmed() } else { "".normal() };

        println!("  #{:<4} [{}] {}{}", id, priority_label, kind.bold(), status);
        if !created.is_empty() {
            println!("        {}", created.dimmed());
        }

        // Show payload preview
        if let Some(payload) = n["payload"].as_str() {
            if payload != "{}" && !payload.is_empty() {
                let preview = if payload.len() > 80 {
                    format!("{}...", &payload[..77])
                } else {
                    payload.to_string()
                };
                println!("        {}", preview.dimmed());
            }
        }
    }

    Ok(())
}

async fn notify(
    agent: Option<String>,
    project: Option<String>,
    priority: u8,
    kind: &str,
    payload: &str,
) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let mut body = json!({
        "priority": priority,
        "kind": kind,
        "payload": payload,
    });

    if let Some(aid) = &agent {
        body["agent_id"] = json!(aid);
    } else if let Some(pid) = &project {
        body["project_id"] = json!(pid);
    } else {
        anyhow::bail!("Either --agent or --project is required");
    }

    nexus.post("/api/hexflo/inbox/notify", &body).await?;

    let target = agent.as_deref().unwrap_or(project.as_deref().unwrap_or("?"));
    println!(
        "{} Notification sent: [{}] {} → {}",
        "\u{2b21}".green(),
        priority,
        kind.bold(),
        target
    );

    Ok(())
}

async fn ack(id: u64) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let agent_id = resolve_agent_id()?;
    let path = format!("/api/hexflo/inbox/{}/ack", id);
    nexus.patch(&path, &json!({ "agent_id": agent_id })).await?;

    println!("{} Notification #{} acknowledged", "\u{2b21}".green(), id);

    Ok(())
}

async fn expire() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.post("/api/hexflo/inbox/expire", &json!({})).await?;
    let count = resp["expiredCount"].as_u64().unwrap_or(0);

    println!("{} Expired {} stale notifications", "\u{2b21}".green(), count);

    Ok(())
}

/// Resolve the current agent ID using the canonical 4-strategy priority chain (ADR-065).
/// Delegates to nexus_client::read_session_agent_id() — the single source of truth.
fn resolve_agent_id() -> anyhow::Result<String> {
    crate::nexus_client::read_session_agent_id().ok_or_else(|| {
        anyhow::anyhow!(
            "Could not resolve agent ID. Try: hex agent connect <nexus-url>, \
             or set HEX_AGENT_ID env var. See: hex agent id"
        )
    })
}
