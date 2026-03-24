//! Project management commands.
//!
//! `hex project register <path>` — register a project with hex-nexus
//! `hex project unregister <id>` — unregister a project
//! `hex project list`            — list registered projects

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::fmt::{pretty_table, truncate as fmt_truncate};
use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum ProjectAction {
    /// Register a project with hex-nexus
    Register {
        /// Path to the project root
        path: String,

        /// Optional project name (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Unregister a project from hex-nexus (keeps files)
    Unregister {
        /// Project ID (from `hex project list`)
        id: String,
    },
    /// Archive a project — unregisters and removes .hex/ config but keeps source files
    Archive {
        /// Project ID (from `hex project list`)
        id: String,

        /// Also remove .claude/ directory
        #[arg(long)]
        remove_claude: bool,
    },
    /// Delete a project — unregisters AND removes all project files from disk
    Delete {
        /// Project ID (from `hex project list`)
        id: String,

        /// Required confirmation flag (this is destructive!)
        #[arg(long)]
        confirm: bool,
    },
    /// List registered projects
    List,
    /// Full project report: agents → swarms → tasks
    Report {
        /// Project ID, name, or prefix. Defaults to current directory name.
        #[arg(default_value = "")]
        id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: ProjectAction) -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    match action {
        ProjectAction::Register { path, name } => register(&client, &path, name).await,
        ProjectAction::Unregister { id } => unregister(&client, &id).await,
        ProjectAction::Archive { id, remove_claude } => {
            archive(&client, &id, remove_claude).await
        }
        ProjectAction::Delete { id, confirm } => delete(&client, &id, confirm).await,
        ProjectAction::List => list(&client).await,
        ProjectAction::Report { id, json } => report(&client, &id, json).await,
    }
}

async fn register(client: &NexusClient, path: &str, name: Option<String>) -> anyhow::Result<()> {
    let abs_path = std::path::Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));

    let mut body = json!({ "rootPath": abs_path.display().to_string() });
    if let Some(n) = &name {
        body["name"] = json!(n);
    }

    let resp = client.post("/api/projects/register", &body).await?;

    let id = resp["id"].as_str().unwrap_or("?");
    let proj_name = resp["name"].as_str().unwrap_or("?");

    println!("{} Project registered", "\u{2b21}".cyan());
    println!("  ID:   {}", id);
    println!("  Name: {}", proj_name);
    println!("  Path: {}", abs_path.display());

    Ok(())
}

async fn unregister(client: &NexusClient, id: &str) -> anyhow::Result<()> {
    client.delete(&format!("/api/projects/{}", id)).await?;
    println!("{} Project {} unregistered", "\u{2b21}".cyan(), id);
    Ok(())
}

async fn archive(
    client: &NexusClient,
    id: &str,
    remove_claude: bool,
) -> anyhow::Result<()> {
    // Get project details first so we know the root path
    let resp = client.get("/api/projects").await?;
    let projects = resp["projects"].as_array();
    let project = projects
        .and_then(|list| list.iter().find(|p| p["id"].as_str() == Some(id)));

    let root_path = match project {
        Some(p) => p["rootPath"].as_str().unwrap_or("").to_string(),
        None => anyhow::bail!("Project {} not found. Run `hex project list` to see IDs.", id),
    };
    let project_name = project
        .and_then(|p| p["name"].as_str())
        .unwrap_or("?");

    println!(
        "{} Archiving project {} ({})",
        "\u{2b21}".cyan(),
        project_name.bold(),
        root_path
    );

    // 1. Unregister from nexus
    client.delete(&format!("/api/projects/{}", id)).await?;
    println!("  {} Unregistered from nexus", "\u{2713}".green());

    // 2. Remove .hex/ config directory
    let hex_dir = std::path::Path::new(&root_path).join(".hex");
    if hex_dir.exists() {
        std::fs::remove_dir_all(&hex_dir)?;
        println!("  {} Removed .hex/", "\u{2713}".green());
    }

    // 3. Remove .mcp.json (hex-specific config)
    let mcp_json = std::path::Path::new(&root_path).join(".mcp.json");
    if mcp_json.exists() {
        std::fs::remove_file(&mcp_json)?;
        println!("  {} Removed .mcp.json", "\u{2713}".green());
    }

    // 4. Optionally remove .claude/ directory
    if remove_claude {
        let claude_dir = std::path::Path::new(&root_path).join(".claude");
        if claude_dir.exists() {
            std::fs::remove_dir_all(&claude_dir)?;
            println!("  {} Removed .claude/", "\u{2713}".green());
        }
    }

    println!();
    println!(
        "{} Project {} archived — source files preserved at {}",
        "\u{2b21}".green(),
        project_name.bold(),
        root_path
    );

    Ok(())
}

async fn delete(
    client: &NexusClient,
    id: &str,
    confirm: bool,
) -> anyhow::Result<()> {
    if !confirm {
        anyhow::bail!(
            "This will permanently delete all project files!\n  \
             Re-run with --confirm to proceed:\n  \
             hex project delete {} --confirm",
            id
        );
    }

    // Get project details
    let resp = client.get("/api/projects").await?;
    let projects = resp["projects"].as_array();
    let project = projects
        .and_then(|list| list.iter().find(|p| p["id"].as_str() == Some(id)));

    let root_path = match project {
        Some(p) => p["rootPath"].as_str().unwrap_or("").to_string(),
        None => anyhow::bail!("Project {} not found. Run `hex project list` to see IDs.", id),
    };
    let project_name = project
        .and_then(|p| p["name"].as_str())
        .unwrap_or("?");

    // Safety: refuse to delete system directories
    let path = std::path::Path::new(&root_path);
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = canon.to_string_lossy();

    if path_str == "/"
        || path_str.starts_with("/System")
        || path_str.starts_with("/usr")
        || path_str.starts_with("/bin")
        || path_str.starts_with("/sbin")
        || path_str.starts_with("/var")
        || path_str == std::env::var("HOME").unwrap_or_default()
    {
        anyhow::bail!(
            "Refusing to delete protected path: {}\n  \
             This looks like a system directory or home folder.",
            root_path
        );
    }

    println!(
        "{} {} Deleting project {} and ALL files at:",
        "\u{26a0}".yellow(),
        "WARNING:".red().bold(),
        project_name.bold()
    );
    println!("  {}", root_path.red());
    println!();

    // 1. Unregister from nexus
    client.delete(&format!("/api/projects/{}", id)).await?;
    println!("  {} Unregistered from nexus", "\u{2713}".green());

    // 2. Delete entire project directory
    if path.exists() {
        std::fs::remove_dir_all(path)?;
        println!("  {} Deleted {}", "\u{2713}".green(), root_path);
    } else {
        println!(
            "  {} Directory already gone: {}",
            "\u{2717}".yellow(),
            root_path
        );
    }

    println!();
    println!(
        "{} Project {} permanently deleted",
        "\u{2b21}".red(),
        project_name.bold()
    );

    Ok(())
}

async fn list(client: &NexusClient) -> anyhow::Result<()> {
    let resp = client.get("/api/projects").await?;

    let projects = match resp["projects"].as_array() {
        Some(list) if !list.is_empty() => list.clone(),
        _ => {
            println!("{} No projects registered", "\u{2b21}".cyan());
            println!("  Register one with: hex project register /path/to/project");
            return Ok(());
        }
    };

    let rows: Vec<Vec<String>> = projects
        .iter()
        .map(|p| {
            let id = p["id"].as_str().unwrap_or("?").to_string();
            let name = p["name"].as_str().unwrap_or("?").bold().to_string();
            let root = fmt_truncate(p["rootPath"].as_str().unwrap_or("?"), 60);
            vec![id, name, root]
        })
        .collect();

    println!("{}", pretty_table(&["ID", "Name", "Path"], &rows));
    println!("  {} project{}", projects.len(), if projects.len() == 1 { "" } else { "s" });

    Ok(())
}

async fn report(client: &NexusClient, id: &str, json_output: bool) -> anyhow::Result<()> {
    // Default to current directory name if no ID given
    let resolved_id = if id.is_empty() {
        std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default()
    } else {
        id.to_string()
    };

    let resp = client
        .get(&format!("/api/projects/{}/report", resolved_id))
        .await
        .map_err(|e| {
            // Enrich error: if 404, show available projects hint
            anyhow::anyhow!("{}\n  Tip: run `hex project list` to see registered project IDs", e)
        })?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    // ── Header ───────────────────────────────────────────
    let proj = &resp["project"];
    let summary = &resp["summary"];
    let name = proj["name"].as_str().unwrap_or("?");
    let root = proj["rootPath"].as_str().unwrap_or("?");
    let proj_id = proj["id"].as_str().unwrap_or("?");

    println!();
    println!("{}", "═".repeat(65).cyan());
    println!("  {} {}", "\u{2b21}".cyan(), name.bold());
    println!("{}", "═".repeat(65).cyan());
    println!("  {}  {}", "ID:  ".white().bold(), proj_id.dimmed());
    println!("  {}  {}", "Path:".white().bold(), root);

    // ── Summary counts ───────────────────────────────────
    let agent_count = summary["agentCount"].as_u64().unwrap_or(0);
    let swarm_count = summary["swarmCount"].as_u64().unwrap_or(0);
    let tasks_total = summary["tasksTotal"].as_u64().unwrap_or(0);
    let tasks_done = summary["tasksCompleted"].as_u64().unwrap_or(0);
    let tasks_pending = summary["tasksPending"].as_u64().unwrap_or(0);
    let tasks_failed = summary["tasksFailed"].as_u64().unwrap_or(0);

    println!();
    println!(
        "  {}  {}  {}  {}",
        format!("{} agents", agent_count).white().bold(),
        format!("{} swarms", swarm_count).white().bold(),
        format!("{}/{} tasks done", tasks_done, tasks_total).green(),
        if tasks_failed > 0 {
            format!("{} failed", tasks_failed).red().to_string()
        } else {
            format!("{} pending", tasks_pending).dimmed().to_string()
        }
    );

    // ── Agents ───────────────────────────────────────────
    let agents = resp["agents"].as_array();
    if let Some(agents) = agents.filter(|a| !a.is_empty()) {
        println!();
        println!("{}", "── Agents ────────────────────────────────────────────────────".dimmed());
        for a in agents {
            let aid = a["agentId"].as_str().or_else(|| a["id"].as_str()).unwrap_or("?");
            let aname = a["name"].as_str().unwrap_or("?");
            let status = a["status"].as_str().unwrap_or("?");
            let model = a["model"].as_str().unwrap_or("");
            let status_colored = colorize_agent_status(status);
            let model_str = if model.is_empty() { String::new() } else { format!("  {}", model.dimmed()) };
            println!("  {} {}  {}{}", "\u{25cf}".cyan(), aname.bold(), status_colored, model_str);
            println!("    {}", aid.dimmed());
        }
    }

    // ── Swarms + Tasks ───────────────────────────────────
    let swarms = resp["swarms"].as_array();
    if let Some(swarms) = swarms.filter(|s| !s.is_empty()) {
        println!();
        println!("{}", "── Swarms ────────────────────────────────────────────────────".dimmed());
        for swarm in swarms {
            let sid = swarm["id"].as_str().unwrap_or("?");
            let sname = swarm["name"].as_str().unwrap_or("?");
            let status = swarm["status"].as_str().unwrap_or("?");
            let topology = swarm["topology"].as_str().unwrap_or("?");
            let st = &swarm["tasks"];
            let s_total = st["total"].as_u64().unwrap_or(0);
            let s_done = st["completed"].as_u64().unwrap_or(0);
            let s_fail = st["failed"].as_u64().unwrap_or(0);
            let s_prog = st["inProgress"].as_u64().unwrap_or(0);

            let status_colored = colorize_swarm_status(status);
            println!(
                "  {} {}  {}  [{}]",
                "\u{2b21}".yellow(),
                sname.bold(),
                status_colored,
                topology.dimmed()
            );
            println!("    {}", sid.dimmed());
            println!(
                "    Tasks: {}/{} done  {} in-progress  {} failed",
                s_done, s_total,
                s_prog,
                s_fail
            );

            // Task list
            if let Some(task_list) = swarm["taskList"].as_array() {
                for task in task_list {
                    let title = task["title"].as_str().unwrap_or("?");
                    let tstatus = task["status"].as_str().unwrap_or("?");
                    let agent = task["agentId"].as_str().unwrap_or("");
                    let icon = task_status_icon(tstatus);
                    let agent_str = if agent.is_empty() {
                        String::new()
                    } else {
                        format!("  → {}", &agent[..agent.len().min(8)])
                    };
                    println!("      {} {}  {}{}", icon, fmt_truncate(title, 55), colorize_task_status(tstatus), agent_str.dimmed());
                }
            }
            println!();
        }
    } else {
        println!();
        println!("  {}", "No swarms created for this project yet.".dimmed());
    }

    println!("{}", "═".repeat(65).cyan());
    println!();
    Ok(())
}

fn task_status_icon(status: &str) -> &'static str {
    match status {
        "completed"   => "✓",
        "failed"      => "✗",
        "in_progress" => "⟳",
        _             => "○",
    }
}

fn colorize_agent_status(s: &str) -> String {
    match s {
        "online"   | "active"   => s.green().to_string(),
        "inactive" | "stale"    => s.yellow().to_string(),
        "dead"     | "offline"  => s.red().to_string(),
        _                       => s.dimmed().to_string(),
    }
}

fn colorize_swarm_status(s: &str) -> String {
    match s {
        "active"    => s.green().to_string(),
        "completed" => s.dimmed().to_string(),
        "failed"    => s.red().to_string(),
        _           => s.yellow().to_string(),
    }
}

fn colorize_task_status(s: &str) -> String {
    match s {
        "completed"   => s.green().to_string(),
        "failed"      => s.red().to_string(),
        "in_progress" => s.yellow().to_string(),
        _             => s.dimmed().to_string(),
    }
}
