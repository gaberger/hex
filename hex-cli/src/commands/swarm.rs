//! Swarm coordination commands.
//!
//! `hex swarm init|status|list` — delegates to hex-nexus HexFlo API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::fmt::{extract_task_title, pretty_table, status_badge, truncate};
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
        /// Explicitly set the project ID (overrides CWD-based resolution)
        #[arg(long)]
        project_id: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show current swarm status
    Status,
    /// List all swarms
    List {
        /// Show all swarms including completed history (default: active + failed only)
        #[arg(long)]
        all: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Mark a swarm as completed
    Complete {
        /// Swarm ID
        id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Mark a swarm as failed
    Fail {
        /// Swarm ID
        id: String,
        /// Reason for failure
        #[arg(default_value = "manually failed")]
        reason: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Clean up stale/completed swarms (dry-run by default)
    Cleanup {
        /// Swarms older than N hours are considered stale (default 24)
        #[arg(long, default_value_t = 24)]
        stale_hours: u64,
        /// Actually execute the transitions (default is dry-run)
        #[arg(long)]
        apply: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// Auto-complete any active swarms where all tasks are done (public for use by agent commands).
/// Returns the number of swarms transitioned.
pub async fn auto_complete_done_swarms(nexus: &NexusClient, swarms: &[serde_json::Value]) -> u32 {
    let mut count = 0u32;
    for swarm in swarms {
        let id = swarm["id"].as_str().unwrap_or("");
        if id.is_empty() { continue; }
        if swarm["status"].as_str() != Some("active") { continue; }
        let tasks = swarm["tasks"].as_array();
        let total = tasks.map(|t| t.len()).unwrap_or(0);
        if total == 0 { continue; }
        let completed = tasks
            .map(|t| t.iter().filter(|tk| tk["status"].as_str() == Some("completed")).count())
            .unwrap_or(0);
        if completed == total
            && nexus.patch(&format!("/api/swarms/{}", id), &json!({})).await.is_ok() {
                count += 1;
            }
    }
    count
}

pub async fn run(action: SwarmAction) -> anyhow::Result<()> {
    match action {
        SwarmAction::Init { name, topology, project_id, json } => init(&name, &topology, project_id.as_deref(), json).await,
        SwarmAction::Status => status().await,
        SwarmAction::List { json, all } => list(json, all).await,
        SwarmAction::Complete { id, .. } => complete(&id).await,
        SwarmAction::Fail { id, reason, .. } => fail(&id, &reason).await,
        SwarmAction::Cleanup { stale_hours, apply, .. } => cleanup(stale_hours, apply).await,
    }
}

async fn init(name: &str, topology: &str, explicit_project_id: Option<&str>, json_output: bool) -> anyhow::Result<()> {
    match topology {
        "hierarchical" | "mesh" | "pipeline" | "mixed" | "hex-pipeline" => {}
        other => {
            anyhow::bail!(
                "Unknown topology '{}'. Supported: hierarchical, mesh, pipeline, mixed, hex-pipeline",
                other
            );
        }
    }

    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Use explicit project_id if provided; otherwise resolve from CWD.
    let project_id: String = if let Some(pid) = explicit_project_id {
        pid.to_string()
    } else {
        let cwd = std::env::current_dir()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        if let Ok(projects_resp) = nexus.get("/api/projects").await {
            projects_resp["projects"]
                .as_array()
                .and_then(|list| {
                    list.iter().find(|p| {
                        p["rootPath"].as_str().map(|rp| rp == cwd_str).unwrap_or(false)
                    })
                })
                .and_then(|p| p["id"].as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    cwd.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string()
                })
        } else {
            cwd.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        }
    };

    let resp = nexus
        .post(
            "/api/swarms",
            &json!({
                "projectId": project_id,
                "name": name,
                "topology": topology,
            }),
        )
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("already owns an active swarm") {
                anyhow::anyhow!(
                    "{}\n\nTo fix: run `hex swarm cleanup --apply` to clear stale swarms, or `hex swarm complete <id>` to close a specific one.",
                    msg
                )
            } else {
                e
            }
        })?;

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
    let swarms = resp.as_array().cloned().unwrap_or_default();

    // Auto-complete swarms where all tasks are done
    auto_complete_done_swarms(&nexus, &swarms).await;

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
                let raw_title = task["title"].as_str().unwrap_or("-");
                let title = extract_task_title(raw_title);
                let status = task["status"].as_str().unwrap_or("unknown");
                let agent_id = task["agentId"].as_str()
                    .or_else(|| task["agent_id"].as_str())
                    .unwrap_or("");
                vec![
                    status_badge(status),
                    truncate(agent_id, 16),
                    truncate(tid, 36),
                    truncate(&title, 50),
                ]
            }).collect();
            println!("{}", pretty_table(&["Status", "Agent", "Task ID", "Title"], &rows));
        }
    }

    Ok(())
}

async fn list(json_output: bool, show_all: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Auto-detect current project from cwd basename (same heuristic as `hex project report`)
    let project_hint = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));

    let (resp, project_scoped) = if let Some(ref hint) = project_hint {
        match nexus.get(&format!("/api/projects/{}/swarms", hint)).await {
            Ok(r) => (r, true),
            Err(_) => (nexus.get("/api/swarms/all?limit=50").await?, false),
        }
    } else {
        (nexus.get("/api/swarms/all?limit=50").await?, false)
    };
    let all_swarms = resp.as_array().cloned().unwrap_or_default();

    if json_output {
        println!("{}", resp);
        return Ok(());
    }

    // Default: only active + failed (things needing attention). --all shows full history.
    let swarms: Vec<_> = if show_all {
        all_swarms.clone()
    } else {
        all_swarms.iter().filter(|s| matches!(s["status"].as_str(), Some("active") | Some("failed"))).cloned().collect()
    };

    let active_count = all_swarms.iter().filter(|s| s["status"].as_str() == Some("active")).count();
    let completed_count = all_swarms.iter().filter(|s| s["status"].as_str() == Some("completed")).count();

    if swarms.is_empty() {
        let hint = if completed_count > 0 {
            format!("  ({} completed — use --all to show)", completed_count)
        } else {
            String::new()
        };
        println!("{} No active or failed swarms{}", "\u{2b21}".dimmed(), hint.dimmed());
        return Ok(());
    }

    let scope_label = if project_scoped {
        format!(" · {}", project_hint.as_deref().unwrap_or("?").dimmed())
    } else {
        " · all projects".dimmed().to_string()
    };
    let header = if active_count > 0 {
        format!("Swarms ({} active / {} total{})", active_count, all_swarms.len(), scope_label)
    } else {
        format!("Swarms ({} total, none active{})", all_swarms.len(), scope_label)
    };
    println!("{} {}", "\u{2b21}".cyan(), header);
    println!();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for swarm in &swarms {
        let id       = swarm["id"].as_str().unwrap_or("-");
        let name     = swarm["name"].as_str().unwrap_or("-");
        let topology = swarm["topology"].as_str().unwrap_or("-");
        let status   = swarm["status"].as_str().unwrap_or("?");

        // taskSummary is from /api/swarms/all; fall back to tasks array for other endpoints
        let (total, completed, in_progress) = if let Some(ts) = swarm.get("taskSummary") {
            (
                ts["total"].as_u64().unwrap_or(0) as usize,
                ts["completed"].as_u64().unwrap_or(0) as usize,
                ts["inProgress"].as_u64().unwrap_or(0) as usize,
            )
        } else {
            let tasks = swarm["tasks"].as_array();
            let t = tasks.map(|t| t.len()).unwrap_or(0);
            let c = tasks.map(|t| t.iter().filter(|tk| tk["status"].as_str() == Some("completed")).count()).unwrap_or(0);
            let p = tasks.map(|t| t.iter().filter(|tk| matches!(tk["status"].as_str(), Some("in_progress") | Some("running"))).count()).unwrap_or(0);
            (t, c, p)
        };
        let pending = total.saturating_sub(completed).saturating_sub(in_progress);

        let status_colored = match status {
            "active"    => status.green().to_string(),
            "completed" => status.dimmed().to_string(),
            "failed"    => status.red().to_string(),
            _           => status.yellow().to_string(),
        };

        // For completed/failed swarms don't show in_progress counts — tasks may be stuck
        // in DB from purged zombie swarms (misleading noise).
        let task_summary = if total == 0 {
            "—".dimmed().to_string()
        } else if status == "completed" {
            format!("{}/{} done", completed, total).green().to_string()
        } else if in_progress > 0 {
            format!("{}/{} done  {} active  {} pending", completed, total, in_progress, pending)
        } else {
            format!("{}/{} done  {} pending", completed, total, pending)
        };

        rows.push(vec![
            truncate(id, 36),
            truncate(name, 36),
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

    // Fetch active swarms + failed swarms (zombies: agent-death with all tasks in_progress)
    let active_resp = nexus.get("/api/swarms/active").await?;
    let mut swarms = active_resp.as_array().cloned().unwrap_or_default();

    // Also load failed swarms to catch zombies (agent-death: all tasks stuck in_progress)
    if let Ok(failed_resp) = nexus.get("/api/swarms/failed").await {
        for s in failed_resp.as_array().cloned().unwrap_or_default() {
            let tasks = s["tasks"].as_array();
            let total = tasks.map(|t| t.len()).unwrap_or(0);
            let in_progress = tasks.map(|t| {
                t.iter().filter(|tk| tk["status"].as_str() == Some("in_progress")).count()
            }).unwrap_or(0);
            let completed = tasks.map(|t| {
                t.iter().filter(|tk| tk["status"].as_str() == Some("completed")).count()
            }).unwrap_or(0);
            // Zombie: all tasks stuck in_progress OR empty swarm (0 tasks ever created)
            if total == 0 || (in_progress == total && completed == 0) {
                swarms.push(s);
            }
        }
    }

    if swarms.is_empty() {
        println!("{} No swarms to clean up", "\u{2b21}".dimmed());
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
        let status = swarm["status"].as_str().unwrap_or("active");
        let tasks = swarm["tasks"].as_array();
        let total = tasks.map(|t| t.len()).unwrap_or(0);
        let completed = tasks
            .map(|t| {
                t.iter()
                    .filter(|tk| tk["status"].as_str() == Some("completed"))
                    .count()
            })
            .unwrap_or(0);

        // Zombie failed swarm: already in failed state with tasks all stuck in_progress
        // OR empty (no tasks ever created). Purge = transition to completed so they
        // collapse into history and stop appearing as anomalies.
        if status == "failed" {
            let reason = if total == 0 {
                "empty — no tasks ever created".to_string()
            } else {
                format!("zombie — {}/{} tasks stuck in_progress (agent died)", total, total)
            };
            actions.push(Action { id, name, transition: "purge", reason });
            continue;
        }

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
                        id: id.clone(),
                        name: name.clone(),
                        transition: "fail",
                        reason: "stale — no tasks, older than cutoff".to_string(),
                    });
                }
            }
        }

        // Partially-done swarms older than cutoff → complete (pipeline abandoned mid-run)
        if total > 0 && completed > 0 && completed < total {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(created_at) {
                if dt < cutoff {
                    actions.push(Action {
                        id,
                        name,
                        transition: "complete",
                        reason: format!("abandoned — {}/{} tasks done, older than {}h", completed, total, stale_hours),
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
            "complete" | "purge" => {
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

