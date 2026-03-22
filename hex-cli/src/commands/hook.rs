//! `hex hook <event>` — Claude Code hook handler.
//!
//! When `hex init` installs hooks into a project, they call back to
//! `hex hook <event>` rather than running Node.js helper scripts.
//! This keeps hex self-contained — no need to copy JS files around.
//!
//! Hook events receive context via environment variables set by Claude Code:
//! - `CLAUDE_PROJECT_DIR` — project root
//! - `CLAUDE_SESSION_ID` — current session
//! - `TOOL_NAME` / `TOOL_INPUT` — for PreToolUse/PostToolUse hooks

use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum HookEvent {
    /// Session started — print project status
    SessionStart,
    /// Session ending — cleanup
    SessionEnd,
    /// Before a Write/Edit/MultiEdit — validate hex boundaries
    PreEdit,
    /// After a Write/Edit/MultiEdit — notify nexus
    PostEdit,
    /// Before a Bash command
    PreBash,
    /// User submitted a prompt — route/classify
    Route,
}

pub async fn run(event: HookEvent) -> Result<()> {
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());

    match event {
        HookEvent::SessionStart => session_start(&project_dir).await,
        HookEvent::SessionEnd => session_end(&project_dir).await,
        HookEvent::PreEdit => pre_edit(&project_dir).await,
        HookEvent::PostEdit => post_edit(&project_dir).await,
        HookEvent::PreBash => pre_bash().await,
        HookEvent::Route => route(&project_dir).await,
    }
}

// ── Event handlers ───────────────────────────────────────────────────

async fn session_start(project_dir: &PathBuf) -> Result<()> {
    let project_json = project_dir.join(".hex/project.json");

    if !project_json.exists() {
        eprintln!(
            "{} Not a hex project (no .hex/project.json). Run `hex init`.",
            "\u{26a0}".yellow()
        );
        return Ok(());
    }

    let content = std::fs::read_to_string(&project_json)?;
    let project: serde_json::Value = serde_json::from_str(&content)?;

    let name = project["name"].as_str().unwrap_or("unknown");
    let id = project["id"].as_str().unwrap_or("?");

    // Print a compact status banner
    println!(
        "\u{2b21}  hex \u{2014} {}",
        name
    );
    println!("  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
    println!("  Project: {} ({})", name, &id[..8]);

    // Check if nexus is reachable
    let nexus_status = check_nexus_health().await;
    match nexus_status {
        Ok(_) => println!("  Nexus:   {}", "connected".green()),
        Err(_) => println!("  Nexus:   {} (run `hex nexus start`)", "offline".yellow()),
    }

    // Check for architecture violations
    let src_dir = project_dir.join("src");
    if src_dir.exists() {
        println!("  Arch:    run `hex analyze .` to check health");
    }

    Ok(())
}

async fn session_end(_project_dir: &PathBuf) -> Result<()> {
    // Lightweight cleanup — future: flush pending memory, deregister agent
    Ok(())
}

async fn pre_edit(project_dir: &PathBuf) -> Result<()> {
    // Read TOOL_INPUT to get the file being edited
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();

    // Quick boundary check: warn if editing across adapter boundaries
    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(file_path) = input["file_path"].as_str() {
            validate_boundary_edit(project_dir, file_path)?;
        }
    }

    Ok(())
}

async fn post_edit(project_dir: &PathBuf) -> Result<()> {
    // Notify nexus of the edit (if running) for live dashboard updates
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();
    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(file_path) = input["file_path"].as_str() {
            let _ = notify_nexus_edit(project_dir, file_path).await;
        }
    }
    Ok(())
}

async fn pre_bash() -> Result<()> {
    // Future: validate dangerous commands, enforce security policy
    Ok(())
}

async fn route(_project_dir: &PathBuf) -> Result<()> {
    // Lightweight prompt classification for intelligence routing
    // Read the user's prompt from TOOL_INPUT
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(content) = input["content"].as_str() {
            let lower = content.to_lowercase();

            // Detect hex-relevant intents and provide context hints
            let hints = classify_prompt(&lower);
            if !hints.is_empty() {
                println!("[HEX] {}", hints.join(", "));
            }
        }
    }

    Ok(())
}

// ── Boundary validation ──────────────────────────────────────────────

fn validate_boundary_edit(project_dir: &PathBuf, file_path: &str) -> Result<()> {
    let rel = file_path
        .strip_prefix(&project_dir.to_string_lossy().as_ref())
        .unwrap_or(file_path)
        .trim_start_matches('/');

    // Detect cross-adapter imports would need AST parsing (hex analyze does this).
    // Here we do a quick structural check: warn if editing composition-root
    // from a context that suggests adapter work.
    if rel.contains("adapters/primary/") || rel.contains("adapters/secondary/") {
        // Adapters are fine to edit — just can't import each other
    } else if rel.contains("domain/") {
        // Domain should have zero external deps — flag if importing node_modules
    }

    Ok(())
}

// ── Nexus communication ──────────────────────────────────────────────

async fn check_nexus_health() -> Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
    let url = format!("http://localhost:{}/api/health", port);

    client.get(&url).send().await?.error_for_status()?;
    Ok(())
}

async fn notify_nexus_edit(_project_dir: &PathBuf, _file_path: &str) -> Result<()> {
    // Best-effort push to nexus — don't block the hook on failure
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1))
        .build()?;

    let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
    let url = format!("http://localhost:{}/api/events", port);

    let _ = client
        .post(&url)
        .json(&serde_json::json!({
            "type": "file_edit",
            "path": _file_path,
        }))
        .send()
        .await;

    Ok(())
}

// ── Prompt classification ────────────────────────────────────────────

fn classify_prompt(prompt: &str) -> Vec<&'static str> {
    let mut hints = Vec::new();

    if prompt.contains("scaffold") || prompt.contains("new project") || prompt.contains("init") {
        hints.push("Relevant: hex scaffold, hex init");
    }
    if prompt.contains("architect") || prompt.contains("boundary") || prompt.contains("violation") {
        hints.push("Relevant: hex analyze");
    }
    if prompt.contains("adr") || prompt.contains("decision record") {
        hints.push("Relevant: hex adr list/search/status");
    }
    if prompt.contains("swarm") || prompt.contains("agent") || prompt.contains("coordinate") {
        hints.push("Relevant: hex swarm, hex task");
    }
    if prompt.contains("feature") && (prompt.contains("develop") || prompt.contains("implement") || prompt.contains("build")) {
        hints.push("Relevant: /hex-feature-dev");
    }

    hints
}
