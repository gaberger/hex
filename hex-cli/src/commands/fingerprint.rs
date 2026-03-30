//! Architecture fingerprint commands.
//!
//! `hex fingerprint generate <project-id> [--root <path>]` — generate fingerprint
//! `hex fingerprint show <project-id>`                      — display fingerprint

use clap::Subcommand;
use colored::Colorize;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum FingerprintAction {
    /// Generate and store the architecture fingerprint for a project
    Generate {
        /// Project ID (from `hex project list`)
        project_id: String,

        /// Project root directory (defaults to current directory)
        #[arg(long, default_value = ".")]
        root: String,

        /// Path to workplan JSON (optional — enriches the fingerprint)
        #[arg(long)]
        workplan: Option<String>,
    },
    /// Show the current architecture fingerprint for a project
    Show {
        /// Project ID (from `hex project list`)
        project_id: String,

        /// Output raw injection block text (default: formatted JSON)
        #[arg(long)]
        text: bool,
    },
}

pub async fn run(action: FingerprintAction) -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    match action {
        FingerprintAction::Generate { project_id, root, workplan } => {
            generate(&client, &project_id, &root, workplan.as_deref()).await
        }
        FingerprintAction::Show { project_id, text } => {
            show(&client, &project_id, text).await
        }
    }
}

async fn generate(
    client: &NexusClient,
    project_id: &str,
    root: &str,
    workplan: Option<&str>,
) -> anyhow::Result<()> {
    let abs_root = std::path::Path::new(root)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(root));

    let body = serde_json::json!({
        "project_root": abs_root.display().to_string(),
        "workplan_path": workplan.unwrap_or(""),
    });

    let url = format!("/api/projects/{}/fingerprint", project_id);
    let resp = client.post_long(&url, &body).await?;

    println!("{} Architecture fingerprint generated", "✓".green());
    if let Some(lang) = resp["language"].as_str() {
        println!("  Language:    {}", lang);
    }
    if let Some(arch) = resp["architecture_style"].as_str() {
        println!("  Style:       {}", arch);
    }
    if let Some(output) = resp["output_type"].as_str() {
        println!("  Output:      {}", output);
    }
    if let Some(tokens) = resp["estimated_tokens"].as_u64() {
        println!("  Est. tokens: {}", tokens);
    }
    Ok(())
}

async fn show(
    client: &NexusClient,
    project_id: &str,
    text: bool,
) -> anyhow::Result<()> {
    if text {
        match client.fetch_fingerprint_text(project_id).await {
            Some(raw) => println!("{}", raw),
            None => anyhow::bail!("No fingerprint found for project {}. Run `hex fingerprint generate {}` first.", project_id, project_id),
        }
    } else {
        let url = format!("/api/projects/{}/fingerprint", project_id);
        let resp = client.get(&url).await?;
        println!("{}", serde_json::to_string_pretty(&resp)?);
    }
    Ok(())
}
