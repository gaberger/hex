//! Task management commands.
//!
//! `hex task create|list|complete` — delegates to hex-nexus swarm/task API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::fmt::{extract_task_title, pretty_table, status_badge, truncate};
use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum TaskAction {
    /// Create a new task in a swarm
    Create {
        /// Swarm ID
        swarm_id: String,
        /// Task title
        title: String,
        /// Comma-separated task IDs this task depends on
        #[arg(long, default_value = "")]
        depends_on: String,
        /// Agent to assign immediately (auto-resolved from session state if omitted)
        #[arg(long)]
        agent: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// List all tasks across active swarms
    List,
    /// Mark a task as complete
    Complete {
        /// Task ID
        id: String,
        /// Completion result/summary
        result: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Assign a task to an agent
    Assign {
        /// Task ID
        task_id: String,
        /// Agent ID (auto-resolved from session state if omitted)
        agent_id: Option<String>,
    },
}

pub async fn run(action: TaskAction) -> anyhow::Result<()> {
    match action {
        TaskAction::Create { swarm_id, title, depends_on, agent, json } => create(&swarm_id, &title, &depends_on, agent, json).await,
        TaskAction::List => list().await,
        TaskAction::Complete { id, result, json } => complete(&id, result.as_deref(), json).await,
        TaskAction::Assign { task_id, agent_id } => assign(&task_id, agent_id).await,
    }
}

async fn create(swarm_id: &str, title: &str, depends_on: &str, agent: Option<String>, json_output: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // Only assign if --agent is explicitly provided. Auto-resolving from session
    // state would pre-assign tasks to the supervisor, preventing Docker workers
    // from self-claiming via the pull model (ADR-2603282000).
    let agent_id = agent;

    // POST to /api/swarms/{swarm_id}/tasks
    let path = format!("/api/swarms/{}/tasks", swarm_id);
    let mut body = json!({ "title": title });
    if !depends_on.is_empty() {
        body["dependsOn"] = json!(depends_on);
    }
    if let Some(ref aid) = agent_id {
        body["agentId"] = json!(aid);
    }

    let resp = nexus.post(&path, &body).await;

    match resp {
        Ok(data) => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                let task_id = data["id"].as_str().unwrap_or("-");
                println!("{} Task created", "\u{2b21}".green());
                println!("  ID:    {}", task_id);
                println!("  Swarm: {}", swarm_id);
                println!("  Title: {}", title.bold());
                if !depends_on.is_empty() {
                    println!("  Deps:  {}", depends_on);
                }
            }
        }
        Err(e) => {
            // Fallback: try with swarmId in body (older API)
            let mut alt_body = json!({ "title": title, "swarmId": swarm_id });
            if !depends_on.is_empty() {
                alt_body["dependsOn"] = json!(depends_on);
            }
            let alt_resp = nexus
                .post("/api/swarms", &alt_body)
                .await;

            match alt_resp {
                Ok(data) => {
                    if json_output {
                        println!("{}", serde_json::to_string_pretty(&data)?);
                    } else {
                        let task_id = data["id"].as_str().or(data["taskId"].as_str()).unwrap_or("-");
                        println!("{} Task created", "\u{2b21}".green());
                        println!("  ID:    {}", task_id);
                        println!("  Swarm: {}", swarm_id);
                        println!("  Title: {}", title.bold());
                        if !depends_on.is_empty() {
                            println!("  Deps:  {}", depends_on);
                        }
                    }
                }
                Err(_) => return Err(e),
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

    let mut all_tasks = Vec::new();
    for swarm in &swarms {
        let swarm_name = swarm["name"].as_str().unwrap_or("-");
        let swarm_id = swarm["id"].as_str().unwrap_or("-");
        if let Some(tasks) = swarm["tasks"].as_array() {
            for task in tasks {
                all_tasks.push((swarm_name.to_string(), swarm_id.to_string(), task.clone()));
            }
        }
    }

    if all_tasks.is_empty() {
        println!("{} No tasks found", "\u{2b21}".dimmed());
        return Ok(());
    }

    // Sort by swarm_name so tasks from the same swarm are contiguous
    all_tasks.sort_by(|a, b| a.0.cmp(&b.0));

    // Count stats
    let completed = all_tasks.iter().filter(|(_, _, t)| t["status"].as_str() == Some("completed")).count();
    let total = all_tasks.len();

    println!(
        "{} Tasks ({}/{} completed)",
        "\u{2b21}".cyan(),
        completed,
        total
    );
    println!();
    let rows: Vec<Vec<String>> = all_tasks
        .iter()
        .map(|(swarm_name, swarm_id, task)| {
            let _ = swarm_id; // available for swarm boundary detection if needed
            let tid = task["id"].as_str().unwrap_or("-");
            let raw_title = task["title"].as_str().unwrap_or("-");
            let title = extract_task_title(raw_title);
            let status = task["status"].as_str().unwrap_or("pending");
            // SwarmTaskInfo (hex-nexus/src/ports/state.rs) uses #[serde(rename_all = "camelCase")],
            // so `agent_id: String` serializes as "agentId" in JSON. The field is non-optional:
            //   - Assigned tasks:   agentId = the assigning agent's UUID (set by STDB task_assign reducer)
            //   - Unassigned tasks: agentId = "" (empty string) — IF the STDB row stores "" not null
            // The snake_case fallback guards against any future adapter that omits the camelCase rename.
            //
            // P1 NOTE: spacetime_state.rs swarm_task_list uses `r.get("agent_id")?.as_str()?.to_string()`
            // which silently drops tasks where agent_id is null in STDB (filter_map returns None).
            // If unassigned tasks are missing from `hex task list`, fix spacetime_state.rs:974 to:
            //   agent_id: r.get("agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            let agent_id = task["agentId"]
                .as_str()
                .or_else(|| task["agent_id"].as_str())
                .unwrap_or("");

            vec![
                swarm_name.to_string(),
                status_badge(status),
                truncate(agent_id, 14),
                truncate(tid, 12),
                truncate(&title, 50),
            ]
        })
        .collect();

    println!(
        "{}",
        pretty_table(
            &["Swarm", "Status", "Agent", "Task ID", "Title"],
            &rows,
        )
    );

    Ok(())
}

async fn complete(id: &str, result: Option<&str>, json_output: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // We need the swarm ID to construct the PATCH URL.
    // Search active swarms for this task.
    let swarms_resp = nexus.get("/api/swarms/active").await?;
    let swarms = swarms_resp
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut swarm_id: Option<String> = None;
    for swarm in &swarms {
        if let Some(tasks) = swarm["tasks"].as_array() {
            if tasks.iter().any(|t| t["id"].as_str() == Some(id)) {
                swarm_id = swarm["id"].as_str().map(String::from);
                break;
            }
        }
    }

    let swarm_id = swarm_id.ok_or_else(|| {
        anyhow::anyhow!("Task '{}' not found in any active swarm", id)
    })?;

    let path = format!("/api/swarms/{}/tasks/{}", swarm_id, id);
    let body = json!({
        "status": "completed",
        "result": result.unwrap_or(""),
    });

    let resp = nexus.patch(&path, &body).await?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        println!("{} Task completed", "\u{2b21}".green());
        println!("  ID:    {}", id);
        println!("  Swarm: {}", swarm_id);
        if let Some(r) = result {
            println!("  Result: {}", r);
        }
    }

    Ok(())
}

async fn assign(task_id: &str, agent_id: Option<String>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let resolved_agent_id = match agent_id {
        Some(id) => id,
        None => crate::nexus_client::read_session_agent_id()
            .ok_or_else(|| anyhow::anyhow!(
                "No agent_id provided and could not auto-resolve from session state.\n\
                 Provide an explicit agent_id or ensure a session is active."
            ))?,
    };

    let path = format!("/api/hexflo/tasks/{}", task_id);
    nexus.patch(&path, &json!({
        "agentId": resolved_agent_id,
    })).await?;

    println!("{} Task assigned", "\u{2b21}".green());
    println!("  Task:  {}", task_id);
    println!("  Agent: {}", resolved_agent_id.bold());

    Ok(())
}
