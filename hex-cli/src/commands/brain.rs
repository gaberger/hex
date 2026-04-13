//! Brain commands (ADR-2604102200).
//!
//! `hex brain status|test|scores|models|selfcheck`
//!
//! status    - Show brain service status and configuration
//! test      - Run a manual test of a model
//! scores    - Show learned method scores
//! models    - List available models for brain selection
//! selfcheck - CLI wiring consistency check

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;
use std::path::Path;

use crate::fmt::{pretty_table, truncate};

#[derive(Subcommand)]
pub enum BrainAction {
    /// Show brain service status and configuration
    Status,
    /// Run a test with a specific model
    Test {
        /// Model name (e.g. nemotron-mini, qwen3:8b)
        #[arg(default_value = "nemotron-mini")]
        model: String,
    },
    /// Show learned method scores from RL engine
    Scores,
    /// List models available for brain selection
    Models,
    /// Check CLI wiring consistency (mod.rs ↔ main.rs ↔ *.rs files)
    #[command(name = "selfcheck")]
    SelfCheck,
}

pub async fn run(action: BrainAction) -> anyhow::Result<()> {
    match action {
        BrainAction::Status => status().await,
        BrainAction::Test { model } => test(&model).await,
        BrainAction::Scores => scores().await,
        BrainAction::Models => models().await,
        BrainAction::SelfCheck => selfcheck().await,
    }
}

async fn status() -> anyhow::Result<()> {
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    
    let url = format!("{}/api/brain/status", base_url);
    let resp = client.get(&url).send().await?;
    
    if resp.status() == 404 {
        println!("{}", "Brain service not configured. Run hex-nexus with brain service enabled.".yellow());
        return Ok(());
    }
    
    if !resp.status().is_success() {
        eprintln!("Error: {}", resp.status());
        return Ok(());
    }
    
    let body: serde_json::Value = resp.json().await?;
    println!("{}", "Brain Service Status".green().bold());
    println!("  Service: {}", body.get("service_enabled").unwrap_or(&json!(false)));
    println!("  Test Model: {}", body.get("test_model").unwrap_or(&json!("nemotron-mini")));
    println!("  Interval: {} seconds", body.get("interval_secs").unwrap_or(&json!(600)));
    println!("  Last Test: {}", body.get("last_test").unwrap_or(&json!("never")));
    
    Ok(())
}

async fn test(model: &str) -> anyhow::Result<()> {
    println!("Testing model: {}", model.green());
    
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    
    let url = format!("{}/api/brain/test", base_url);
    let body = json!({ "model": model });
    let resp = client.post(&url).json(&body).send().await?;
    
    if !resp.status().is_success() {
        eprintln!("Test failed: {}", resp.status());
        let err: serde_json::Value = resp.json().await.unwrap_or_default();
        eprintln!("{}", err);
        return Ok(());
    }
    
    let result: serde_json::Value = resp.json().await?;
    println!("{}", "Test Result".green().bold());
    println!("  Outcome: {}", result.get("outcome").unwrap_or(&json!("unknown")));
    println!("  Reward: {}", result.get("reward").unwrap_or(&json!(0.0)));
    println!("  Response: {}", truncate(&result.get("response").unwrap_or(&json!("")).to_string(), 200));
    
    Ok(())
}

async fn scores() -> anyhow::Result<()> {
    let port = std::env::var("HEX_NEXUS_PORT")
        .unwrap_or_else(|_| "5555".to_string())
        .parse::<u16>()
        .unwrap_or(5555);
    let base_url = format!("http://127.0.0.1:{}", port);
    let client = reqwest::Client::new();
    
    let url = format!("{}/api/brain/scores", base_url);
    let resp = client.get(&url).send().await?;
    
    if resp.status() == 404 {
        println!("{}", "No scores yet. Brain service is learning.".yellow());
        return Ok(());
    }
    
    if !resp.status().is_success() {
        eprintln!("Error: {}", resp.status());
        return Ok(());
    }
    
    let scores: Vec<serde_json::Value> = resp.json().await?;
    
    if scores.is_empty() {
        println!("{}", "No scores recorded yet.".yellow());
        return Ok(());
    }
    
    println!("{}", "Method Scores".green().bold());
    let rows: Vec<Vec<String>> = scores
        .iter()
        .map(|score| {
            vec![
                score.get("method").unwrap_or(&json!("")).to_string(),
                format!("{:.3}", score.get("q_value").unwrap_or(&json!(0.0))),
                score.get("visit_count").unwrap_or(&json!(0)).to_string(),
            ]
        })
        .collect();
    println!("{}", pretty_table(&["Method", "Score", "Visits"], &rows));
    
    Ok(())
}

async fn models() -> anyhow::Result<()> {
    let models = vec![
        ("nemotron-mini", "Fast local inference", "0.3"),
        ("qwen3:4b", "Small local model", "0.25"),
        ("qwen3:8b", "Medium local model", "0.35"),
        ("qwen3.5:9b", "Large local model", "0.40"),
        ("qwen2.5-coder:32b", "Coding dedicated", "0.50"),
        ("sonnet", "Cloud fallback", "0.50"),
    ];

    println!("{}", "Available Models".green().bold());
    println!("{:<20}  {:<25}  Base Score", "Model", "Description");
    println!("{}", "-".repeat(60));

    for (model, desc, score) in models {
        println!("{:<20}  {:<25}  {}", model, desc, score);
    }

    Ok(())
}

// ── CLI wiring consistency check ──────────────────────────────────────────

/// An unwired module detected by `check_cli_wiring`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnwiredModule {
    /// Module stem (e.g. "decide", "taste")
    pub name: String,
    /// true when the `.rs` file exists but `pub mod <name>` is missing from mod.rs
    pub missing_from_mod: bool,
    /// true when `pub mod` exists but no Commands enum variant references the module
    pub missing_from_commands: bool,
}

/// Pure function: given the contents of the commands directory, mod.rs, and
/// main.rs, return every module that is not fully wired.
///
/// A module is "fully wired" when:
///   1. A `.rs` file exists in `commands/`
///   2. `mod.rs` contains `pub mod <stem>;`
///   3. `main.rs` references the module (via `commands::<stem>::` or as an import)
///
/// `commands_dir` is the path to `hex-cli/src/commands/`.
/// `mod_rs_content` and `main_rs_content` are the file contents as strings.
pub fn check_cli_wiring(
    rs_file_stems: &[String],
    mod_rs_content: &str,
    main_rs_content: &str,
) -> Vec<UnwiredModule> {
    // Parse `pub mod <name>;` entries from mod.rs
    let mod_entries: Vec<String> = mod_rs_content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("pub mod ") {
                // "pub mod foo;" → "foo"
                trimmed
                    .strip_prefix("pub mod ")
                    .and_then(|rest| rest.strip_suffix(';'))
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
        .collect();

    // Build a set of module names referenced in main.rs.
    // We look for `commands::<name>::` patterns — this catches both
    // `use commands::foo::*` imports and `commands::foo::run(...)` calls.
    let mut main_refs: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in main_rs_content.lines() {
        let trimmed = line.trim();
        // Match patterns like `commands::foo::` or `commands::foo,`
        let mut search = trimmed;
        while let Some(idx) = search.find("commands::") {
            let after = &search[idx + "commands::".len()..];
            let end = after
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            if end > 0 {
                main_refs.insert(after[..end].to_string());
            }
            search = &search[idx + "commands::".len() + end..];
        }
    }

    let mut unwired = Vec::new();

    for stem in rs_file_stems {
        // Skip mod.rs itself — it's not a command module
        if stem == "mod" {
            continue;
        }

        let in_mod = mod_entries.contains(stem);
        let in_main = main_refs.contains(stem);

        if !in_mod || !in_main {
            unwired.push(UnwiredModule {
                name: stem.clone(),
                missing_from_mod: !in_mod,
                missing_from_commands: !in_main,
            });
        }
    }

    unwired.sort_by(|a, b| a.name.cmp(&b.name));
    unwired
}

async fn selfcheck() -> anyhow::Result<()> {
    let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");

    // Collect .rs file stems
    let rs_stems: Vec<String> = std::fs::read_dir(&commands_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();

    let mod_rs = std::fs::read_to_string(commands_dir.join("mod.rs"))?;
    let main_rs = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"),
    )?;

    let unwired = check_cli_wiring(&rs_stems, &mod_rs, &main_rs);

    if unwired.is_empty() {
        println!("{}", "All command modules are fully wired.".green());
    } else {
        println!(
            "{} {} unwired module(s):",
            "⚠".yellow(),
            unwired.len()
        );
        for m in &unwired {
            let reason = match (m.missing_from_mod, m.missing_from_commands) {
                (true, true) => "not in mod.rs, not in Commands",
                (true, false) => "not in mod.rs",
                (false, true) => "not in Commands enum (main.rs)",
                (false, false) => "ok", // shouldn't happen
            };
            println!("  {} — {}", m.name.red(), reason);
        }
    }

    Ok(())
}