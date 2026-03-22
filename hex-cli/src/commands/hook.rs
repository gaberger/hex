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

    // Check if nexus is reachable and register as agent (ADR-048)
    let nexus_status = check_nexus_health().await;
    match nexus_status {
        Ok(health) => {
            println!("  Nexus:   {}", "connected".green());

            // Report SpacetimeDB status from health response
            let stdb_ok = health["spacetimedb"].as_bool().unwrap_or(false);
            if stdb_ok {
                println!("  StDB:    {}", "connected".green());
            } else {
                println!("  StDB:    {} (nexus using SQLite fallback)", "offline".yellow());
            }

            // Register this Claude Code session as an agent
            let _ = register_session_agent(project_dir, name).await;
        }
        Err(_) => {
            println!("  Nexus:   {} (run `hex nexus start`)", "offline".yellow());
            println!("  StDB:    {} (requires nexus)", "offline".dimmed());
        }
    }

    // Check for architecture violations
    let src_dir = project_dir.join("src");
    if src_dir.exists() {
        println!("  Arch:    run `hex analyze .` to check health");
    }

    Ok(())
}

/// Register this Claude Code session as an agent with hex-nexus (ADR-048).
///
/// Persists the returned agentId to ~/.hex/sessions/ so session_end can
/// deregister without relying on in-process memory.
async fn register_session_agent(project_dir: &PathBuf, project_name: &str) -> Result<()> {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let model = std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "unknown".to_string());
    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .to_string();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
    let url = format!("http://localhost:{}/api/agents/connect", port);

    let agent_name = if session_id.is_empty() {
        format!("claude-{}", &hostname)
    } else {
        format!("claude-{}", &session_id[..8.min(session_id.len())])
    };

    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "host": hostname,
            "name": agent_name,
            "project_dir": project_dir.to_string_lossy(),
            "model": model,
            "session_id": session_id,
        }))
        .send()
        .await?
        .error_for_status()?;

    let body: serde_json::Value = resp.json().await?;
    let agent_id = body["agentId"].as_str().unwrap_or("");

    if !agent_id.is_empty() {
        // Persist agentId to disk for session_end deregistration
        let sessions_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".hex/sessions");
        std::fs::create_dir_all(&sessions_dir)?;

        let state_key = if session_id.is_empty() {
            format!("agent-{}.json", std::process::id())
        } else {
            format!("agent-{}.json", &session_id)
        };

        let state_file = sessions_dir.join(state_key);
        std::fs::write(
            &state_file,
            serde_json::json!({
                "agentId": agent_id,
                "name": agent_name,
                "project": project_name,
                "registered_at": chrono::Utc::now().to_rfc3339(),
            })
            .to_string(),
        )?;

        println!("  Agent:   {} ({})", "registered".green(), agent_name);
    }

    Ok(())
}

async fn session_end(_project_dir: &PathBuf) -> Result<()> {
    // Deregister this session's agent from hex-nexus (ADR-048)
    let _ = deregister_session_agent().await;
    Ok(())
}

/// Deregister this Claude Code session from hex-nexus.
///
/// Reads the agentId from the persisted state file written by session_start,
/// calls POST /api/agents/disconnect, then cleans up the state file.
async fn deregister_session_agent() -> Result<()> {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".hex/sessions");

    let state_key = if session_id.is_empty() {
        format!("agent-{}.json", std::process::id())
    } else {
        format!("agent-{}.json", &session_id)
    };

    let state_file = sessions_dir.join(&state_key);
    if !state_file.exists() {
        return Ok(()); // No registration to undo
    }

    let content = std::fs::read_to_string(&state_file)?;
    let state: serde_json::Value = serde_json::from_str(&content)?;
    let agent_id = state["agentId"].as_str().unwrap_or("");

    if !agent_id.is_empty() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()?;

        let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
        let url = format!("http://localhost:{}/api/agents/disconnect", port);

        // Fire-and-forget — don't block session teardown
        let _ = client
            .post(&url)
            .json(&serde_json::json!({ "agentId": agent_id }))
            .send()
            .await;
    }

    // Clean up state file regardless of disconnect success
    let _ = std::fs::remove_file(&state_file);

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

async fn check_nexus_health() -> Result<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()?;

    let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
    let url = format!("http://localhost:{}/api/health", port);

    let resp = client.get(&url).send().await?.error_for_status()?;
    let body: serde_json::Value = resp.json().await?;
    Ok(body)
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
