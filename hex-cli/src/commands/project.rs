//! Project management commands.
//!
//! `hex project register <path>` — register a project with hex-nexus
//! `hex project unregister <id>` — unregister a project
//! `hex project list`            — list registered projects

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

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

    let projects = resp["projects"].as_array();
    match projects {
        Some(list) if !list.is_empty() => {
            println!("{} Registered projects ({})", "\u{2b21}".cyan(), list.len());
            println!();
            for p in list {
                let id = p["id"].as_str().unwrap_or("?");
                let name = p["name"].as_str().unwrap_or("?");
                let root = p["rootPath"].as_str().unwrap_or("?");
                println!("  {} {}", id.dimmed(), name.bold());
                println!("    {}", root);
            }
        }
        _ => {
            println!("{} No projects registered", "\u{2b21}".cyan());
            println!("  Register one with: hex project register /path/to/project");
        }
    }

    Ok(())
}
