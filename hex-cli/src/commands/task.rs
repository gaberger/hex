//! Task management commands.
//!
//! `hex task create|list|complete` — delegates to hex-nexus swarm/task API.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

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
        TaskAction::Create { swarm_id, title, depends_on, json } => create(&swarm_id, &title, &depends_on, json).await,
        TaskAction::List => list().await,
        TaskAction::Complete { id, result, json } => complete(&id, result.as_deref(), json).await,
        TaskAction::Assign { task_id, agent_id } => assign(&task_id, agent_id).await,
    }
}

async fn create(swarm_id: &str, title: &str, depends_on: &str, json_output: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // The swarm endpoint expects tasks to be created via the swarm
    let path = format!("/api/swarms/{}/tasks", swarm_id);

    // Build the request body — include depends_on if non-empty
    let mut body = json!({
        "swarmId": swarm_id,
        "title": title,
        "action": "create_task",
    });
    if !depends_on.is_empty() {
        body["dependsOn"] = json!(depends_on);
    }

    // The endpoint is POST to create swarm with tasks, but individual task creation
    // uses the swarm tasks sub-resource. We'll use the HexFlo task_create pattern.
    let resp = nexus
        .post("/api/swarms", &body)
        .await;

    // If the swarm endpoint doesn't support direct task creation,
    // fall back to the task-specific pattern
    match resp {
        Ok(data) => {
            if json_output {
                println!("{}", serde_json::to_string_pretty(&data)?);
            } else {
                let task_id = data["taskId"].as_str().unwrap_or("-");
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
            // Try alternative: the path-based endpoint
            let mut alt_body = json!({ "title": title });
            if !depends_on.is_empty() {
                alt_body["dependsOn"] = json!(depends_on);
            }
            let alt_resp = nexus
                .post(&path, &alt_body)
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
    println!(
        "  {:<15} {:<12} {:<14} {:<12} {}",
        "SWARM".bold(),
        "STATUS".bold(),
        "AGENT".bold(),
        "TASK ID".bold(),
        "TITLE".bold()
    );
    println!("  {}", "\u{2500}".repeat(90).dimmed());

    for (swarm_name, _swarm_id, task) in &all_tasks {
        let tid = task["id"].as_str().unwrap_or("-");
        let title = task["title"].as_str().unwrap_or("-");
        let status = task["status"].as_str().unwrap_or("pending");
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
        } else if agent_id.len() > 12 {
            agent_id[..12].to_string()
        } else {
            agent_id.to_string()
        };

        let tid_short = if tid.len() > 10 { &tid[..10] } else { tid };
        println!(
            "  {:<15} {:<21} {:<14} {:<12} {}",
            swarm_name, status_colored, agent_display, tid_short, title
        );
    }

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
        "agent_id": resolved_agent_id,
    })).await?;

    println!("{} Task assigned", "\u{2b21}".green());
    println!("  Task:  {}", task_id);
    println!("  Agent: {}", resolved_agent_id.bold());

    Ok(())
}
