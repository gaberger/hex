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
//!
//! ADR-050: Hook-Enforced Agent Lifecycle Pipeline
//! Every hook validates participation in: ADR → WorkPlan → HexFlo Memory → Swarm

use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Extended session state file (ADR-050).
/// Persisted to ~/.hex/sessions/agent-{sessionId}.json
#[derive(Serialize, Deserialize, Default)]
struct SessionState {
    #[serde(rename = "agentId")]
    agent_id: String,
    name: String,
    project: String,
    registered_at: String,
    #[serde(default)]
    workplan_id: Option<String>,
    #[serde(default)]
    swarm_id: Option<String>,
    #[serde(default)]
    current_task_id: Option<String>,
    #[serde(default)]
    last_heartbeat: Option<String>,
    #[serde(default)]
    edits: u64,
    #[serde(default)]
    phase: Option<String>,
}

impl SessionState {
    fn state_file_path() -> PathBuf {
        let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
        let sessions_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".hex/sessions");
        let key = if session_id.is_empty() {
            format!("agent-{}.json", std::process::id())
        } else {
            format!("agent-{}.json", &session_id)
        };
        sessions_dir.join(key)
    }

    fn load() -> Option<Self> {
        let path = Self::state_file_path();
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    fn save(&self) -> Result<()> {
        let path = Self::state_file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn nexus_url(path: &str) -> String {
    let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
    format!("http://localhost:{}{}", port, path)
}

fn nexus_client(timeout_secs: u64) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    // Inject agent ID for agent-guarded endpoints (hexflo, swarms)
    if let Some(state) = SessionState::load() {
        if !state.agent_id.is_empty() {
            if let Ok(val) = reqwest::header::HeaderValue::from_str(&state.agent_id) {
                headers.insert("x-hex-agent-id", val);
            }
        }
    }
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .default_headers(headers)
        .build()?)
}

/// Check if lifecycle enforcement is mandatory for this project.
fn enforcement_mode(project_dir: &PathBuf) -> &'static str {
    let project_json = project_dir.join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
            if project["lifecycle_enforcement"].as_str() == Some("mandatory") {
                return "mandatory";
            }
        }
    }
    "advisory"
}

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
    /// Subagent spawned — auto-assign task if HEXFLO_TASK in prompt
    SubagentStart,
    /// Subagent completed — auto-complete task
    SubagentStop,
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
        HookEvent::SubagentStart => subagent_start().await,
        HookEvent::SubagentStop => subagent_stop().await,
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

            // Register this Claude Code session as an agent (ADR-048)
            let _ = register_session_agent(project_dir, name).await;

            // ADR-050: Load active workplan from HexFlo memory
            let _ = load_workplan_context(id).await;
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
/// Extended with lifecycle state tracking (ADR-050).
async fn register_session_agent(project_dir: &PathBuf, project_name: &str) -> Result<()> {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let model = std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "unknown".to_string());
    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .to_string();

    let client = nexus_client(3)?;
    let url = nexus_url("/api/agents/connect");

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
        let now = chrono::Utc::now().to_rfc3339();
        let state = SessionState {
            agent_id: agent_id.to_string(),
            name: agent_name.clone(),
            project: project_name.to_string(),
            registered_at: now.clone(),
            last_heartbeat: Some(now),
            edits: 0,
            workplan_id: None,
            swarm_id: None,
            current_task_id: None,
            phase: None,
        };
        state.save()?;

        println!("  Agent:   {} ({})", "registered".green(), agent_name);
    }

    Ok(())
}

/// SubagentStart — read stdin for HEXFLO_TASK:{uuid}, auto-assign the task.
async fn subagent_start() -> Result<()> {
    let stdin = std::io::read_to_string(std::io::stdin()).unwrap_or_default();

    // Look for HEXFLO_TASK:{uuid} pattern in the subagent prompt
    let task_id = extract_hexflo_task(&stdin);
    if task_id.is_none() {
        return Ok(()); // No task reference — nothing to sync
    }
    let task_id = task_id.unwrap();

    // Resolve agent_id from session state
    let state = match SessionState::load() {
        Some(s) => s,
        None => return Ok(()),
    };

    // Assign the task via nexus REST API
    let client = nexus_client(2)?;
    let url = nexus_url(&format!("/api/hexflo/tasks/{}", task_id));
    let _ = client
        .patch(&url)
        .json(&serde_json::json!({ "agent_id": state.agent_id }))
        .send()
        .await;

    // Track the mapping so SubagentStop can complete it
    let mut state = state;
    state.current_task_id = Some(task_id);
    state.save()?;

    Ok(())
}

/// SubagentStop — auto-complete the task if one was assigned on start.
async fn subagent_stop() -> Result<()> {
    let stdin = std::io::read_to_string(std::io::stdin()).unwrap_or_default();

    let state = match SessionState::load() {
        Some(s) => s,
        None => return Ok(()),
    };

    let task_id = match &state.current_task_id {
        Some(id) => id.clone(),
        None => return Ok(()), // No task was assigned — nothing to complete
    };

    // Use the first 200 chars of subagent output as the result summary
    let result = if stdin.len() > 200 {
        format!("{}...", &stdin[..200])
    } else if stdin.is_empty() {
        "completed".to_string()
    } else {
        stdin.trim().to_string()
    };

    // Complete the task via nexus REST API
    let client = nexus_client(2)?;
    let url = nexus_url(&format!("/api/hexflo/tasks/{}", task_id));
    let _ = client
        .patch(&url)
        .json(&serde_json::json!({
            "status": "completed",
            "result": result,
        }))
        .send()
        .await;

    // Clear the current task from session state
    let mut state = state;
    state.current_task_id = None;
    state.save()?;

    Ok(())
}

/// Extract HEXFLO_TASK:{uuid} from text. Returns the UUID if found.
fn extract_hexflo_task(text: &str) -> Option<String> {
    let prefix = "HEXFLO_TASK:";
    let start = text.find(prefix)?;
    let after = &text[start + prefix.len()..];
    // UUID is 36 chars (8-4-4-4-12)
    if after.len() >= 36 {
        let candidate = &after[..36];
        // Basic validation: contains hyphens at right positions
        if candidate.chars().nth(8) == Some('-')
            && candidate.chars().nth(13) == Some('-')
        {
            return Some(candidate.to_string());
        }
    }
    None
}

async fn session_end(_project_dir: &PathBuf) -> Result<()> {
    // ADR-050: Flush progress to HexFlo memory, then deregister (ADR-048)
    let _ = flush_session_progress().await;
    let _ = deregister_session_agent().await;
    Ok(())
}

/// Flush session progress to HexFlo memory before disconnecting (ADR-050).
async fn flush_session_progress() -> Result<()> {
    let state = match SessionState::load() {
        Some(s) => s,
        None => return Ok(()),
    };

    // Only flush if there was meaningful activity
    if state.edits == 0 && state.workplan_id.is_none() {
        return Ok(());
    }

    let client = nexus_client(2)?;

    // Store session summary in HexFlo memory
    let memory_key = format!("session:{}:summary", state.name);
    let summary = serde_json::json!({
        "agent": state.name,
        "workplan": state.workplan_id,
        "swarm": state.swarm_id,
        "task": state.current_task_id,
        "phase": state.phase,
        "edits": state.edits,
        "ended_at": chrono::Utc::now().to_rfc3339(),
    });

    let _ = client
        .post(&nexus_url("/api/hexflo/memory"))
        .json(&serde_json::json!({
            "key": memory_key,
            "value": summary.to_string(),
            "scope": "project",
        }))
        .send()
        .await;

    // If there's an active swarm task, update its status
    if let Some(task_id) = &state.current_task_id {
        let _ = client
            .patch(&nexus_url(&format!("/api/swarms/tasks/{}", task_id)))
            .json(&serde_json::json!({
                "status": "paused",
                "result": format!("Session ended after {} edits", state.edits),
            }))
            .send()
            .await;
    }

    Ok(())
}

/// Deregister this Claude Code session from hex-nexus (ADR-048).
async fn deregister_session_agent() -> Result<()> {
    let state = match SessionState::load() {
        Some(s) => s,
        None => return Ok(()),
    };

    if !state.agent_id.is_empty() {
        let client = nexus_client(2)?;

        // Fire-and-forget — don't block session teardown
        let _ = client
            .post(&nexus_url("/api/agents/disconnect"))
            .json(&serde_json::json!({ "agentId": state.agent_id }))
            .send()
            .await;
    }

    // Clean up state file regardless of disconnect success
    let _ = std::fs::remove_file(SessionState::state_file_path());

    Ok(())
}

async fn pre_edit(project_dir: &PathBuf) -> Result<()> {
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(file_path) = input["file_path"].as_str() {
            // Existing hex boundary check
            validate_boundary_edit(project_dir, file_path)?;

            let mode = enforcement_mode(project_dir);
            let state = SessionState::load();

            // ADR-050: Enforce workplan + swarm registration before edits
            if let Some(ref state) = state {
                let has_workplan = state.workplan_id.is_some();
                let has_swarm = state.swarm_id.is_some();

                if !has_workplan {
                    if mode == "mandatory" {
                        // stdout so Claude sees it; exit non-zero to block
                        println!(
                            "BLOCKED: No active workplan. Create one first: hex plan create <name>"
                        );
                        std::process::exit(2);
                    } else {
                        // Advisory: stdout warning so it enters Claude's context
                        println!(
                            "WARNING: Editing without an active workplan. Consider: hex plan create <name>"
                        );
                    }
                } else if !has_swarm {
                    if mode == "mandatory" {
                        println!(
                            "BLOCKED: Workplan active but no HexFlo swarm registered. Run: hex swarm init <name>"
                        );
                        std::process::exit(2);
                    } else {
                        println!(
                            "WARNING: Editing without a HexFlo swarm. Consider: hex swarm init <name>"
                        );
                    }
                }

                // ADR-050: Validate file falls within workplan adapter boundary
                if let Some(ref workplan_id) = state.workplan_id {
                    validate_workplan_boundary(project_dir, file_path, workplan_id)?;
                }
            }
        }
    }

    Ok(())
}

async fn post_edit(project_dir: &PathBuf) -> Result<()> {
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();
    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(file_path) = input["file_path"].as_str() {
            // Notify nexus for live dashboard updates
            let _ = notify_nexus_edit(project_dir, file_path).await;

            // ADR-050: Increment edit counter and update HexFlo memory
            if let Some(mut state) = SessionState::load() {
                state.edits += 1;
                let _ = state.save();

                // Push edit event to HexFlo memory (best-effort)
                let _ = record_edit_event(&state, file_path).await;
            }

            // ADR-055: Auto-sync README ADR table when an ADR file is edited
            if file_path.contains("docs/adrs/") && file_path.ends_with(".md") {
                let readme_path = project_dir.join("README.md");
                let adr_dir = project_dir.join("docs/adrs");
                if readme_path.exists() {
                    if let Ok(true) = super::readme::sync_adr_section(&readme_path, &adr_dir) {
                        tracing::debug!("ADR-055: README.md ADR summary auto-synced");
                    }
                }
            }
        }
    }
    Ok(())
}

async fn pre_bash() -> Result<()> {
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(command) = input["command"].as_str() {
            // ADR-050: Detect destructive operations
            let destructive = is_destructive_command(command);
            if destructive {
                if let Some(state) = SessionState::load() {
                    let in_ship_phase = state.phase.as_deref() == Some("SHIP");
                    if !in_ship_phase && state.workplan_id.is_some() {
                        eprintln!(
                            "{} Destructive command outside SHIP phase: `{}`",
                            "\u{26a0}".yellow(),
                            truncate_cmd(command, 60)
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

async fn route(project_dir: &PathBuf) -> Result<()> {
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();

    // ADR-050: Send heartbeat on every user interaction
    let _ = send_heartbeat().await;

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(content) = input["content"].as_str() {
            let lower = content.to_lowercase();

            // Detect hex-relevant intents and provide context hints
            let hints = classify_prompt(&lower);
            if !hints.is_empty() {
                println!("[HEX] {}", hints.join(", "));
            }

            // ADR-050: Warn if no active workplan (advisory mode)
            if let Some(state) = SessionState::load() {
                if state.workplan_id.is_none() {
                    let mode = enforcement_mode(project_dir);
                    // Only warn on work-like prompts, not queries
                    let is_work = lower.contains("implement")
                        || lower.contains("create")
                        || lower.contains("add")
                        || lower.contains("fix")
                        || lower.contains("refactor")
                        || lower.contains("build")
                        || lower.contains("update")
                        || lower.contains("change")
                        || lower.contains("modify")
                        || lower.contains("write")
                        || lower.contains("generate")
                        || lower.contains("scaffold")
                        || lower.contains("wire")
                        || lower.contains("connect")
                        || lower.contains("remove")
                        || lower.contains("delete")
                        || lower.contains("migrate")
                        || lower.contains("upgrade")
                        // Short confirmations after a work proposal
                        || is_confirmatory_response(&lower);
                    if is_work && mode == "mandatory" {
                        println!(
                            "BLOCKED: Cannot proceed without an active workplan. Run: hex plan create <name>"
                        );
                        std::process::exit(2);
                    } else if is_work {
                        // stdout so Claude sees the warning in its context
                        println!(
                            "WARNING: No active workplan for this work. Consider: hex plan create <name>"
                        );
                    }
                }
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

/// ADR-050: Check if file being edited falls within the workplan's declared adapter boundary.
/// Loads the workplan JSON, extracts declared `files` from all tasks, and warns/blocks
/// if the edit target isn't in any task's file list.
fn validate_workplan_boundary(project_dir: &PathBuf, file_path: &str, workplan_id: &str) -> Result<()> {
    let rel = file_path
        .strip_prefix(&project_dir.to_string_lossy().as_ref())
        .unwrap_or(file_path)
        .trim_start_matches('/');

    // Files outside hex structure — no enforcement needed
    if detect_hex_layer(rel).is_none() {
        return Ok(());
    }

    // Try to load the workplan JSON to check declared file boundaries
    let workplan_path = project_dir.join("docs/workplans").join(workplan_id);
    let content = match std::fs::read_to_string(&workplan_path) {
        Ok(c) => c,
        Err(_) => return Ok(()), // Can't load workplan — skip boundary check
    };

    let workplan: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    // Collect all declared files from all tiers/steps
    let mut declared_files: Vec<String> = Vec::new();
    if let Some(tiers) = workplan["tiers"].as_object() {
        for (_tier_name, tier) in tiers {
            if let Some(steps) = tier["steps"].as_array() {
                for step in steps {
                    // Single file field
                    if let Some(f) = step["file"].as_str() {
                        declared_files.push(f.to_string());
                    }
                    // Array of files
                    if let Some(files) = step["files"].as_array() {
                        for f in files {
                            if let Some(s) = f.as_str() {
                                declared_files.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // If no files declared in workplan, skip boundary check
    if declared_files.is_empty() {
        return Ok(());
    }

    // Check if the file being edited matches any declared file (prefix match for directories)
    let in_boundary = declared_files.iter().any(|declared| {
        rel == declared || rel.starts_with(declared) || declared.starts_with(rel)
    });

    if !in_boundary {
        let mode = enforcement_mode(project_dir);
        if mode == "mandatory" {
            println!(
                "BLOCKED: File '{}' is outside workplan boundary. Declared files: {:?}",
                rel,
                &declared_files[..declared_files.len().min(5)]
            );
            std::process::exit(2);
        } else {
            println!(
                "WARNING: File '{}' is outside the active workplan's declared boundary.",
                rel
            );
        }
    }

    Ok(())
}

/// Detect which hex layer a file belongs to.
fn detect_hex_layer(rel_path: &str) -> Option<&'static str> {
    if rel_path.contains("core/domain/") || rel_path.contains("src/domain/") {
        Some("domain")
    } else if rel_path.contains("core/ports/") || rel_path.contains("src/ports/") {
        Some("ports")
    } else if rel_path.contains("core/usecases/") || rel_path.contains("src/usecases/") {
        Some("usecases")
    } else if rel_path.contains("adapters/primary/") {
        Some("primary")
    } else if rel_path.contains("adapters/secondary/") {
        Some("secondary")
    } else if rel_path.contains("composition-root") {
        Some("composition-root")
    } else {
        None
    }
}

// ── Nexus communication ──────────────────────────────────────────────

async fn check_nexus_health() -> Result<serde_json::Value> {
    let client = nexus_client(2)?;
    let resp = client.get(&nexus_url("/api/health")).send().await?.error_for_status()?;
    let body: serde_json::Value = resp.json().await?;
    Ok(body)
}

async fn notify_nexus_edit(_project_dir: &PathBuf, file_path: &str) -> Result<()> {
    let client = nexus_client(1)?;
    let _ = client
        .post(&nexus_url("/api/events"))
        .json(&serde_json::json!({
            "type": "file_edit",
            "path": file_path,
        }))
        .send()
        .await;
    Ok(())
}

// ── ADR-050: Lifecycle helpers ───────────────────────────────────────

/// Load active workplan context from HexFlo memory into session state.
async fn load_workplan_context(project_id: &str) -> Result<()> {
    let client = nexus_client(2)?;
    let key = format!("workplan:active:{}", project_id);
    let encoded_key: String = key.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            c.to_string()
        } else {
            format!("%{:02X}", c as u32)
        }
    }).collect();
    let url = nexus_url(&format!("/api/hexflo/memory/{}", encoded_key));

    let resp = client.get(&url).send().await;
    if let Ok(resp) = resp {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                let workplan_id = body["value"].as_str().and_then(|v| {
                    serde_json::from_str::<serde_json::Value>(v)
                        .ok()
                        .and_then(|wp| wp["workplan_id"].as_str().map(String::from))
                });

                if let Some(wp_id) = workplan_id {
                    if let Some(mut state) = SessionState::load() {
                        state.phase = body["value"].as_str().and_then(|v| {
                            serde_json::from_str::<serde_json::Value>(v)
                                .ok()
                                .and_then(|wp| wp["phase"].as_str().map(String::from))
                        });
                        state.swarm_id = body["value"].as_str().and_then(|v| {
                            serde_json::from_str::<serde_json::Value>(v)
                                .ok()
                                .and_then(|wp| wp["swarm_id"].as_str().map(String::from))
                        });
                        state.workplan_id = Some(wp_id.clone());
                        let _ = state.save();
                    }
                    println!("  Plan:    {} ({})", wp_id.green(), "active");
                }
            }
        }
    }

    Ok(())
}

/// Send heartbeat to hex-nexus (ADR-050).
async fn send_heartbeat() -> Result<()> {
    let mut state = match SessionState::load() {
        Some(s) => s,
        None => return Ok(()),
    };

    // Lazy registration: if this session was never registered with nexus
    // (e.g. nexus was offline at session start), try now.
    if state.agent_id.is_empty() {
        let _ = try_lazy_register(&mut state).await;
        if state.agent_id.is_empty() {
            return Ok(()); // Still can't register — nexus likely still offline
        }
    }

    let client = nexus_client(2)?;
    let url = nexus_url(&format!("/api/agents/{}/heartbeat", state.agent_id));

    let now = chrono::Utc::now().to_rfc3339();
    let _ = client
        .post(&url)
        .json(&serde_json::json!({
            "timestamp": &now,
            "phase": state.phase,
            "edits": state.edits,
        }))
        .send()
        .await;

    state.last_heartbeat = Some(now);
    let _ = state.save();

    Ok(())
}

/// Record an edit event in HexFlo memory (ADR-050).
/// Attempt to register this session with nexus if it wasn't registered at startup.
/// This handles the case where nexus was offline when the Claude Code session started
/// but came online later. Runs silently — errors are swallowed.
async fn try_lazy_register(state: &mut SessionState) -> Result<()> {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let model = std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "unknown".to_string());
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR").unwrap_or_default();
    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .to_string();

    let agent_name = if session_id.is_empty() {
        format!("claude-{}", &hostname)
    } else {
        format!("claude-{}", &session_id[..8.min(session_id.len())])
    };

    // Derive project name from dir
    let project_name = std::path::Path::new(&project_dir)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let client = nexus_client(2)?;
    let url = nexus_url("/api/agents/connect");

    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "host": hostname,
            "name": agent_name,
            "project_dir": project_dir,
            "model": model,
            "session_id": session_id,
        }))
        .send()
        .await?
        .error_for_status()?;

    let body: serde_json::Value = resp.json().await?;
    let agent_id = body["agentId"].as_str().unwrap_or("");

    if !agent_id.is_empty() {
        let now = chrono::Utc::now().to_rfc3339();
        state.agent_id = agent_id.to_string();
        state.name = agent_name;
        state.project = project_name;
        state.registered_at = now.clone();
        state.last_heartbeat = Some(now);
        state.save()?;

        // Notify Claude that registration happened (appears in hook output)
        eprintln!("  Agent:   {} (late registration)", "registered".to_string());
    }

    Ok(())
}

async fn record_edit_event(state: &SessionState, file_path: &str) -> Result<()> {
    if state.agent_id.is_empty() {
        return Ok(());
    }

    let client = nexus_client(1)?;
    let memory_key = format!("agent:{}:last_edit", state.agent_id);

    let _ = client
        .post(&nexus_url("/api/hexflo/memory"))
        .json(&serde_json::json!({
            "key": memory_key,
            "value": serde_json::json!({
                "file": file_path,
                "edit_number": state.edits,
                "phase": state.phase,
                "workplan": state.workplan_id,
                "at": chrono::Utc::now().to_rfc3339(),
            }).to_string(),
            "scope": "agent",
        }))
        .send()
        .await;

    Ok(())
}

/// Detect destructive bash commands (ADR-050).
fn is_destructive_command(cmd: &str) -> bool {
    let patterns = [
        "git push --force",
        "git push -f",
        "git reset --hard",
        "git clean -f",
        "rm -rf",
        "rm -r ",
        "drop table",
        "DROP TABLE",
        "git branch -D",
        "git checkout -- .",
        "git restore .",
    ];
    patterns.iter().any(|p| cmd.contains(p))
}

/// Truncate a command string for display.
fn truncate_cmd(cmd: &str, max: usize) -> String {
    if cmd.len() <= max {
        cmd.to_string()
    } else {
        format!("{}...", &cmd[..max])
    }
}

/// Detect short confirmatory responses that likely approve a proposed code change.
/// When Claude proposes work and the user says "yes" / "do it" / "go ahead",
/// that inherits the work classification of the prior exchange.
fn is_confirmatory_response(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    // Only match very short responses — longer prompts are queries, not confirmations
    if trimmed.len() > 30 {
        return false;
    }
    let confirmations = [
        "yes", "yep", "yeah", "yea", "y", "sure", "ok", "okay", "go",
        "go ahead", "do it", "proceed", "continue", "ship it", "lgtm",
        "approved", "let's go", "sounds good", "go for it", "make it so",
        "do that", "yes please", "please do", "please", "correct",
    ];
    confirmations.iter().any(|c| trimmed == *c)
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
