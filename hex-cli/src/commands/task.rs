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
    },
    /// List all tasks across active swarms
    List,
    /// Mark a task as complete
    Complete {
        /// Task ID
        id: String,
        /// Completion result/summary
        result: Option<String>,
    },
}

pub async fn run(action: TaskAction) -> anyhow::Result<()> {
    match action {
        TaskAction::Create { swarm_id, title } => create(&swarm_id, &title).await,
        TaskAction::List => list().await,
        TaskAction::Complete { id, result } => complete(&id, result.as_deref()).await,
    }
}

async fn create(swarm_id: &str, title: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // The swarm endpoint expects tasks to be created via the swarm
    let path = format!("/api/swarms/{}/tasks", swarm_id);

    // The endpoint is POST to create swarm with tasks, but individual task creation
    // uses the swarm tasks sub-resource. We'll use the HexFlo task_create pattern.
    let resp = nexus
        .post(
            "/api/swarms",
            &json!({
                "swarmId": swarm_id,
                "title": title,
                "action": "create_task",
            }),
        )
        .await;

    // If the swarm endpoint doesn't support direct task creation,
    // fall back to the task-specific pattern
    match resp {
        Ok(data) => {
            let task_id = data["taskId"].as_str().unwrap_or("-");
            println!("{} Task created", "\u{2b21}".green());
            println!("  ID:    {}", task_id);
            println!("  Swarm: {}", swarm_id);
            println!("  Title: {}", title.bold());
        }
        Err(e) => {
            // Try alternative: the path-based endpoint
            let alt_resp = nexus
                .post(&path, &json!({ "title": title }))
                .await;

            match alt_resp {
                Ok(data) => {
                    let task_id = data["id"].as_str().or(data["taskId"].as_str()).unwrap_or("-");
                    println!("{} Task created", "\u{2b21}".green());
                    println!("  ID:    {}", task_id);
                    println!("  Swarm: {}", swarm_id);
                    println!("  Title: {}", title.bold());
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

    println!("{} Tasks ({})", "\u{2b21}".cyan(), all_tasks.len());
    println!();
    println!(
        "  {:<15} {:<12} {:<36} {}",
        "SWARM".bold(),
        "STATUS".bold(),
        "TASK ID".bold(),
        "TITLE".bold()
    );
    println!("  {}", "\u{2500}".repeat(80).dimmed());

    for (swarm_name, _swarm_id, task) in &all_tasks {
        let tid = task["id"].as_str().unwrap_or("-");
        let title = task["title"].as_str().unwrap_or("-");
        let status = task["status"].as_str().unwrap_or("pending");

        let status_colored = match status {
            "completed" => status.green().to_string(),
            "in_progress" | "running" => status.yellow().to_string(),
            "pending" => status.dimmed().to_string(),
            "failed" => status.red().to_string(),
            _ => status.to_string(),
        };

        let tid_short = if tid.len() > 34 { &tid[..34] } else { tid };
        println!(
            "  {:<15} {:<21} {:<36} {}",
            swarm_name, status_colored, tid_short, title
        );
    }

    Ok(())
}

async fn complete(id: &str, result: Option<&str>) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    // We need the swarm ID to construct the PATCH URL.
    // Search active swarms for this task.
    let swarms_resp = nexus.get("/api/swarms/active").await?;
    let swarms = swarms_resp
        .get("swarms")
        .and_then(|s| s.as_array())
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

    nexus.patch(&path, &body).await?;

    println!("{} Task completed", "\u{2b21}".green());
    println!("  ID:    {}", id);
    println!("  Swarm: {}", swarm_id);
    if let Some(r) = result {
        println!("  Result: {}", r);
    }

    Ok(())
}
