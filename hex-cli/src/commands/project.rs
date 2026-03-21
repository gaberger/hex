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
    /// Unregister a project from hex-nexus
    Unregister {
        /// Project ID (from `hex project list`)
        id: String,
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
