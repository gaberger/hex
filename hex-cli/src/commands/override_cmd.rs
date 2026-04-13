//! Emergency override (ADR-2604131500 §1 Layer 4).
//! `hex override <project> "<instruction>"` — sends priority-2 notification to all agents.

use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

pub async fn run(project: &str, instruction: &str) -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    let body = json!({
        "project_id": project,
        "priority": 2,
        "kind": "override",
        "payload": instruction,
    });

    client.post("/api/hexflo/inbox/notify", &body).await?;

    println!(
        "{} Override sent to {}: {}",
        "\u{26a0}".yellow(),
        project.bold(),
        instruction
    );

    Ok(())
}
