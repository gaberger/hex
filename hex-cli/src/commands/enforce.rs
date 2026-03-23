//! `hex enforce` — manage enforcement rules (ADR-2603221959 P5).
//!
//! Rules define what hex enforces at the MCP, CLI, and API layers.
//! Rules are synced from `.hex/adr-rules.toml` to SpacetimeDB on startup,
//! and can be listed/toggled via this command.

use colored::Colorize;
use clap::Subcommand;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum EnforceAction {
    /// List all enforcement rules
    List,
    /// Sync rules from .hex/adr-rules.toml to SpacetimeDB
    Sync,
    /// Disable a rule by ID
    Disable {
        /// Rule ID to disable
        rule_id: String,
    },
    /// Enable a rule by ID
    Enable {
        /// Rule ID to enable
        rule_id: String,
    },
    /// Show current enforcement mode (mandatory/advisory/disabled)
    Mode,
}

pub async fn run(action: EnforceAction) -> anyhow::Result<()> {
    match action {
        EnforceAction::List => list().await,
        EnforceAction::Sync => sync().await,
        EnforceAction::Disable { rule_id } => toggle(&rule_id, false).await,
        EnforceAction::Enable { rule_id } => toggle(&rule_id, true).await,
        EnforceAction::Mode => show_mode().await,
    }
}

async fn list() -> anyhow::Result<()> {
    // Try nexus first for SpacetimeDB rules
    let nexus = NexusClient::from_env();
    let from_stdb = if nexus.ensure_running().await.is_ok() {
        nexus.get("/api/hexflo/enforcement-rules").await.ok()
    } else {
        None
    };

    // Also load from local .hex/adr-rules.toml
    let local_rules = load_local_rules();

    println!("{} Enforcement Rules (ADR-2603221959)", "\u{2b21}".cyan());
    println!();

    // Show enforcement mode
    let mode = resolve_mode();
    let mode_display = match mode.as_str() {
        "mandatory" => "mandatory".red().bold().to_string(),
        "advisory" => "advisory".yellow().to_string(),
        "disabled" => "disabled".dimmed().to_string(),
        _ => mode.clone(),
    };
    println!("  Mode: {}", mode_display);
    println!();

    // Display local rules
    if !local_rules.is_empty() {
        println!("  {}", "Local rules (.hex/adr-rules.toml):".bold());
        for rule in &local_rules {
            let severity_icon = match rule.severity.as_str() {
                "error" => "\u{2717}".red(),
                "warning" => "\u{26a0}".yellow(),
                _ => "\u{2139}".dimmed(),
            };
            println!(
                "    {} [{}] {} — {}",
                severity_icon,
                rule.adr.dimmed(),
                rule.id,
                rule.message
            );
        }
        println!("    {} rule(s)", local_rules.len());
    } else {
        println!("  {} No local rules found (.hex/adr-rules.toml)", "\u{25cb}".dimmed());
    }

    // Display SpacetimeDB rules if available
    if let Some(data) = from_stdb {
        if let Some(rules) = data["rules"].as_array() {
            if !rules.is_empty() {
                println!();
                println!("  {}", "SpacetimeDB rules:".bold());
                for rule in rules {
                    let enabled = rule["enabled"].as_u64().unwrap_or(1) == 1;
                    let icon = if enabled {
                        "\u{2713}".green()
                    } else {
                        "\u{25cb}".dimmed()
                    };
                    println!(
                        "    {} [{}] {} — {}",
                        icon,
                        rule["adr"].as_str().unwrap_or("?").dimmed(),
                        rule["id"].as_str().unwrap_or("?"),
                        rule["message"].as_str().unwrap_or("")
                    );
                }
                println!("    {} rule(s)", rules.len());
            }
        }
    }

    Ok(())
}

async fn sync() -> anyhow::Result<()> {
    let rules = load_local_rules();
    if rules.is_empty() {
        println!("  {} No rules in .hex/adr-rules.toml", "\u{25cb}".dimmed());
        return Ok(());
    }

    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let mut synced = 0u32;
    for rule in &rules {
        let body = serde_json::json!({
            "id": rule.id,
            "adr": rule.adr,
            "operation": "pattern_match",
            "condition": "pattern_match",
            "severity": rule.severity,
            "enabled": 1,
            "project_id": "",
            "message": rule.message,
            "file_patterns": rule.file_patterns.join(","),
            "violation_patterns": rule.violation_patterns.join(","),
        });
        match nexus.post("/api/hexflo/enforcement-rules", &body).await {
            Ok(_) => synced += 1,
            Err(e) => eprintln!("  {} Failed to sync {}: {}", "\u{2717}".red(), rule.id, e),
        }
    }

    println!(
        "  {} {}/{} rules synced to SpacetimeDB",
        "\u{2713}".green(),
        synced,
        rules.len()
    );
    Ok(())
}

async fn toggle(rule_id: &str, enabled: bool) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let body = serde_json::json!({
        "id": rule_id,
        "enabled": if enabled { 1 } else { 0 },
    });
    nexus.patch("/api/hexflo/enforcement-rules/toggle", &body).await?;

    let action = if enabled { "enabled" } else { "disabled" };
    println!("  {} Rule {} {}", "\u{2713}".green(), rule_id, action);
    Ok(())
}

async fn show_mode() -> anyhow::Result<()> {
    let mode = resolve_mode();
    let display = match mode.as_str() {
        "mandatory" => format!("{} — violations block operations", "mandatory".red().bold()),
        "advisory" => format!("{} — violations produce warnings", "advisory".yellow()),
        "disabled" => format!("{} — no enforcement", "disabled".dimmed()),
        _ => mode.clone(),
    };
    println!("  Enforcement mode: {}", display);
    println!("  Set in: .hex/project.json → lifecycle_enforcement");
    Ok(())
}

// ── Local rule loading ──────────────────────────────────

#[derive(Debug)]
struct LocalRule {
    id: String,
    adr: String,
    severity: String,
    message: String,
    file_patterns: Vec<String>,
    violation_patterns: Vec<String>,
}

fn load_local_rules() -> Vec<LocalRule> {
    let paths = [
        std::path::PathBuf::from(".hex/adr-rules.toml"),
        std::env::var("CLAUDE_PROJECT_DIR")
            .map(|d| std::path::PathBuf::from(d).join(".hex/adr-rules.toml"))
            .unwrap_or_default(),
    ];

    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(parsed) = content.parse::<toml::Table>() {
                if let Some(rules) = parsed.get("rules").and_then(|r| r.as_array()) {
                    return rules
                        .iter()
                        .filter_map(|r| {
                            Some(LocalRule {
                                id: r.get("id")?.as_str()?.to_string(),
                                adr: r.get("adr")?.as_str()?.to_string(),
                                severity: r.get("severity")?.as_str()?.to_string(),
                                message: r.get("message")?.as_str()?.to_string(),
                                file_patterns: r
                                    .get("file_patterns")?
                                    .as_array()?
                                    .iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect(),
                                violation_patterns: r
                                    .get("violation_patterns")?
                                    .as_array()?
                                    .iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect(),
                            })
                        })
                        .collect();
                }
            }
        }
    }

    Vec::new()
}

fn resolve_mode() -> String {
    let paths = [
        std::path::PathBuf::from(".hex/project.json"),
        std::env::var("CLAUDE_PROJECT_DIR")
            .map(|d| std::path::PathBuf::from(d).join(".hex/project.json"))
            .unwrap_or_default(),
    ];

    for path in &paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(mode) = project["lifecycle_enforcement"].as_str() {
                    return mode.to_string();
                }
            }
        }
    }

    "mandatory".to_string()
}
