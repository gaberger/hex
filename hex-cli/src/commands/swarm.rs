//! Swarm coordination commands.
//!
//! `hex swarm init|status|list` — delegates to hex-nexus HexFlo API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::fmt::{pretty_table, status_badge, truncate};
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
    /// Mark a swarm as completed
    Complete {
        /// Swarm ID
        id: String,
    },
    /// Mark a swarm as failed
    Fail {
        /// Swarm ID
        id: String,
        /// Reason for failure
        #[arg(default_value = "manually failed")]
        reason: String,
    },
    /// Clean up stale/completed swarms (dry-run by default)
    Cleanup {
        /// Swarms older than N hours are considered stale (default 24)
        #[arg(long, default_value_t = 24)]
        stale_hours: u64,
        /// Actually execute the transitions (default is dry-run)
        #[arg(long)]
        apply: bool,
    },
}

pub async fn run(action: SwarmAction) -> anyhow::Result<()> {
    match action {
        SwarmAction::Init { name, topology, json } => init(&name, &topology, json).await,
        SwarmAction::Status => status().await,
        SwarmAction::List => list().await,
        SwarmAction::Complete { id } => complete(&id).await,
        SwarmAction::Fail { id, reason } => fail(&id, &reason).await,
        SwarmAction::Cleanup { stale_hours, apply } => cleanup(stale_hours, apply).await,
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
            let rows: Vec<Vec<String>> = tasks.iter().map(|task| {
                let tid = task["id"].as_str().unwrap_or("-");
                let title = task["title"].as_str().unwrap_or("-");
                let status = task["status"].as_str().unwrap_or("unknown");
                let agent_id = task["agentId"].as_str()
                    .or_else(|| task["agent_id"].as_str())
                    .unwrap_or("");
                vec![
                    status_badge(status),
                    truncate(agent_id, 16),
                    truncate(tid, 36),
                    truncate(title, 50),
                ]
            }).collect();
            println!("{}", pretty_table(&["Status", "Agent", "Task ID", "Title"], &rows));
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

    let mut rows: Vec<Vec<String>> = Vec::new();
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

        rows.push(vec![
            truncate(id, 36),
            name.to_string(),
            topology.to_string(),
            status_colored,
            task_summary,
        ]);
    }

    println!("{}", pretty_table(&["ID", "Name", "Topology", "Status", "Tasks"], &rows));

    Ok(())
}

async fn complete(id: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    nexus
        .patch(&format!("/api/swarms/{}", id), &json!({}))
        .await?;

    println!("{} Swarm {} marked as completed", "\u{2b21}".green(), &id[..8]);
    Ok(())
}

async fn fail(id: &str, reason: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    nexus
        .post(
            &format!("/api/swarms/{}/fail", id),
            &json!({ "reason": reason }),
        )
        .await?;

    println!("{} Swarm {} marked as failed: {}", "\u{2b21}".red(), &id[..8], reason);
    Ok(())
}

async fn cleanup(stale_hours: u64, apply: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resp = nexus.get("/api/swarms/active").await?;
    let swarms = resp.as_array().cloned().unwrap_or_default();

    if swarms.is_empty() {
        println!("{} No active swarms to clean up", "\u{2b21}".dimmed());
        return Ok(());
    }

    let cutoff = chrono::Utc::now() - chrono::Duration::hours(stale_hours as i64);

    // Classify each swarm
    struct Action {
        id: String,
        name: String,
        transition: &'static str, // "complete" or "fail"
        reason: String,
    }
    let mut actions: Vec<Action> = Vec::new();

    for swarm in &swarms {
        let id = swarm["id"].as_str().unwrap_or("").to_string();
        let name = swarm["name"].as_str().unwrap_or("-").to_string();
        let tasks = swarm["tasks"].as_array();
        let total = tasks.map(|t| t.len()).unwrap_or(0);
        let completed = tasks
            .map(|t| {
                t.iter()
                    .filter(|tk| tk["status"].as_str() == Some("completed"))
                    .count()
            })
            .unwrap_or(0);

        // All tasks completed → mark swarm completed
        if total > 0 && completed == total {
            actions.push(Action {
                id,
                name,
                transition: "complete",
                reason: format!("all {}/{} tasks done", completed, total),
            });
            continue;
        }

        // All tasks pending + older than cutoff → mark swarm failed (stale)
        let created_at = swarm["createdAt"]
            .as_str()
            .or_else(|| swarm["created_at"].as_str())
            .unwrap_or("");
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(created_at) {
            if dt < cutoff && completed == 0 {
                actions.push(Action {
                    id,
                    name,
                    transition: "fail",
                    reason: format!("stale — 0/{} tasks started, older than {}h", total, stale_hours),
                });
                continue;
            }
        }

        // Empty swarms (0 tasks) older than cutoff → fail
        if total == 0 {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(created_at) {
                if dt < cutoff {
                    actions.push(Action {
                        id,
                        name,
                        transition: "fail",
                        reason: "stale — no tasks, older than cutoff".to_string(),
                    });
                }
            }
        }
    }

    if actions.is_empty() {
        println!("{} No swarms need cleanup", "\u{2b21}".dimmed());
        return Ok(());
    }

    // Print table
    let rows: Vec<Vec<String>> = actions
        .iter()
        .map(|a| {
            vec![
                truncate(&a.id, 36),
                a.name.clone(),
                a.transition.to_string(),
                a.reason.clone(),
            ]
        })
        .collect();

    println!(
        "{} Swarm cleanup — {} action(s){}",
        "\u{2b21}".cyan(),
        actions.len(),
        if apply { "" } else { " (dry-run)" }
    );
    println!();
    println!("{}", pretty_table(&["ID", "Name", "Action", "Reason"], &rows));

    if !apply {
        println!();
        println!(
            "Run with {} to execute these transitions",
            "--apply".bold()
        );
        return Ok(());
    }

    // Execute transitions
    let mut ok = 0u32;
    let mut err = 0u32;
    for action in &actions {
        let result = match action.transition {
            "complete" => {
                nexus
                    .patch(&format!("/api/swarms/{}", action.id), &json!({}))
                    .await
            }
            "fail" => {
                nexus
                    .post(
                        &format!("/api/swarms/{}/fail", action.id),
                        &json!({ "reason": action.reason }),
                    )
                    .await
            }
            _ => unreachable!(),
        };
        match result {
            Ok(_) => ok += 1,
            Err(e) => {
                eprintln!("  {} {} — {}", "✗".red(), &action.id[..8], e);
                err += 1;
            }
        }
    }

    println!();
    println!(
        "{} Done: {} succeeded, {} failed",
        "\u{2b21}".green(),
        ok,
        err
    );

    Ok(())
}

