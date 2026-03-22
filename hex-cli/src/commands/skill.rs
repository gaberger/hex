//! `hex skill` — Manage skills registered in SpacetimeDB (ADR-042).
//!
//! Skills are synced from filesystem to SpacetimeDB on nexus startup.
//! These commands query and manage the SpacetimeDB skill registry.

use clap::Subcommand;
use colored::Colorize;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum SkillAction {
    /// List all registered skills
    List,
    /// Re-sync skills from filesystem to SpacetimeDB
    Sync,
    /// Show a specific skill's content
    Show {
        /// Skill name (e.g. "hex-generate", "project-output")
        name: String,
    },
}

pub async fn run(action: SkillAction) -> anyhow::Result<()> {
    match action {
        SkillAction::List => list_skills().await,
        SkillAction::Sync => sync_skills().await,
        SkillAction::Show { name } => show_skill(&name).await,
    }
}

async fn list_skills() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let data = nexus.get("/api/skills").await?;

    let skills = data
        .as_array()
        .unwrap_or(&Vec::new())
        .to_vec();

    if skills.is_empty() {
        println!("{} No skills registered (run `hex skill sync` or start nexus)", "\u{2717}".yellow());
        return Ok(());
    }

    println!("{} {} registered skills\n", "\u{2b21}".cyan(), skills.len());

    // Group by source path prefix
    let mut global: Vec<&serde_json::Value> = Vec::new();
    let mut project: Vec<&serde_json::Value> = Vec::new();

    for skill in &skills {
        let source = skill["source_path"].as_str().unwrap_or("");
        if source.starts_with(".claude/") {
            project.push(skill);
        } else {
            global.push(skill);
        }
    }

    if !global.is_empty() {
        println!("  {} Global skills (skills/)", "●".bold());
        for skill in &global {
            let name = skill["name"].as_str().unwrap_or("?");
            let trigger = skill["trigger"].as_str().unwrap_or("");
            if trigger.is_empty() {
                println!("    {}", name);
            } else {
                println!("    {} {}", name, trigger.dimmed());
            }
        }
        println!();
    }

    if !project.is_empty() {
        println!("  {} Project skills (.claude/skills/)", "●".bold());
        for skill in &project {
            let name = skill["name"].as_str().unwrap_or("?");
            let trigger = skill["trigger"].as_str().unwrap_or("");
            if trigger.is_empty() {
                println!("    {}", name);
            } else {
                println!("    {} {}", name, trigger.dimmed());
            }
        }
        println!();
    }

    Ok(())
}

async fn sync_skills() -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    println!("{} Syncing skills to SpacetimeDB...", "\u{2b21}".cyan());

    let result = nexus.post("/api/skills/sync", &serde_json::json!({})).await?;

    let count = result["synced"].as_u64().unwrap_or(0);
    println!("  {} {} skills synced", "\u{2713}".green(), count);

    Ok(())
}

async fn show_skill(name: &str) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let data = nexus.get(&format!("/api/skills/{}", name)).await?;

    if data.is_null() || data.get("error").is_some() {
        println!("{} Skill '{}' not found", "\u{2717}".red(), name);
        return Ok(());
    }

    let skill_name = data["name"].as_str().unwrap_or(name);
    let trigger = data["trigger"].as_str().unwrap_or("");
    let description = data["description"].as_str().unwrap_or("");
    let source = data["source_path"].as_str().unwrap_or("");

    println!("{} Skill: {}", "\u{2b21}".cyan(), skill_name.bold());
    if !trigger.is_empty() {
        println!("  Trigger: {}", trigger);
    }
    if !description.is_empty() {
        println!("  Description: {}", description);
    }
    if !source.is_empty() {
        println!("  Source: {}", source.dimmed());
    }

    Ok(())
}
