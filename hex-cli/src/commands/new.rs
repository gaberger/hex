//! Structured project intake (ADR-2604131500 §2).
//! `hex new <path>` — register a project, seed trust, optionally copy taste.

use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

pub async fn run(path: &str, name: Option<String>, description: Option<String>) -> anyhow::Result<()> {
    // Resolve / create the target directory
    let target = std::path::Path::new(path);
    if !target.exists() {
        std::fs::create_dir_all(target)?;
    }
    let abs_path = target
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));

    let client = NexusClient::from_env();
    client.ensure_running().await?;

    let dir_name = abs_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());

    let mut body = json!({ "rootPath": abs_path.display().to_string() });
    if let Some(n) = &name {
        body["name"] = json!(n);
    } else {
        body["name"] = json!(&dir_name);
    }
    if let Some(d) = &description {
        body["description"] = json!(d);
    }

    let resp = client.post("/api/projects/register", &body).await?;

    let proj_name = resp["name"]
        .as_str()
        .unwrap_or(name.as_deref().unwrap_or(&dir_name));

    println!(
        "{} Project {} registered. Trust: all suggest. Run `hex brief` to check status.",
        "\u{2713}".green(),
        proj_name.bold()
    );

    Ok(())
}
