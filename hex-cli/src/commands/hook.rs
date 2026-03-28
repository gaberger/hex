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
    /// PID of the parent `claude` process — used by the statusline to match
    /// this session file to the correct Claude instance.
    #[serde(default)]
    claude_pid: Option<u32>,
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
    /// Active worktree path for current task (ADR-2603231700)
    #[serde(default)]
    worktree_path: Option<String>,
    /// Allowed file paths for adapter boundary enforcement (ADR-2603231700)
    #[serde(default)]
    allowed_paths: Vec<String>,
    /// Resolved worktree branch name from workplan step
    #[serde(default)]
    worktree_branch: Option<String>,
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

/// Check lifecycle enforcement mode for this project.
/// Default is "mandatory" — all hex projects enforce the ADR → workplan → code pipeline.
/// Set "lifecycle_enforcement": "advisory" in .hex/project.json to downgrade to warnings only.
fn enforcement_mode(project_dir: &PathBuf) -> &'static str {
    let project_json = project_dir.join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
            if project["lifecycle_enforcement"].as_str() == Some("advisory") {
                return "advisory";
            }
        }
    }
    "mandatory"
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
    /// Before an Agent tool call — enforce HEXFLO_TASK for background agents
    PreAgent,
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
        HookEvent::PreAgent => pre_agent().await,
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

            // Register this Claude Code session as an agent (ADR-048, ADR-058)
            let _ = register_session_agent(project_dir, name).await;

            // Evict dead agents from previous sessions (ADR-058)
            if let Ok(evict_client) = nexus_client(3) {
                let _ = evict_client.post(nexus_url("/api/hex-agents/evict")).send().await;
            }

            // ADR-060: Check for restart checkpoint from a previous session
            let _ = recover_restart_checkpoint().await;

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

    // ADR-2603221939: Auto-upgrade settings — ensure Agent PreToolUse hook exists
    ensure_agent_hook(project_dir);

    Ok(())
}

/// Ensure `.claude/settings.json` has the Agent PreToolUse hook (ADR-2603221939).
/// If the Agent matcher is missing, inject it automatically on session start.
/// This upgrades existing projects without requiring `hex init --force`.
fn ensure_agent_hook(project_dir: &std::path::Path) {
    let settings_path = project_dir.join(".claude/settings.json");
    if !settings_path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Quick check: if "pre-agent" is already in the file, nothing to do
    if content.contains("pre-agent") {
        return;
    }

    let mut settings: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Inject the Agent matcher into PreToolUse array
    if let Some(pre_tool_use) = settings
        .get_mut("hooks")
        .and_then(|h| h.get_mut("PreToolUse"))
        .and_then(|p| p.as_array_mut())
    {
        pre_tool_use.push(serde_json::json!({
            "matcher": "Agent",
            "hooks": [{
                "type": "command",
                "command": "hex hook pre-agent",
                "timeout": 3000
            }]
        }));

        if let Ok(updated) = serde_json::to_string_pretty(&settings) {
            if std::fs::write(&settings_path, &updated).is_ok() {
                println!("  Hooks:   {} Agent enforcement auto-installed (ADR-2603221939)", "\u{2713}".green());
            }
        }
    }
}

/// Register this Claude Code session as an agent with hex-nexus (ADR-048).
/// Extended with lifecycle state tracking (ADR-050).
/// Register this Claude session as an agent with hex-nexus.
/// Writes `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json` with the agent_id.
/// Called by session-start hook and by `hex dev start` (Phase 0).
pub async fn register_session_agent(project_dir: &PathBuf, project_name: &str) -> Result<()> {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let model = std::env::var("CLAUDE_MODEL").unwrap_or_else(|_| "unknown".to_string());
    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .to_string();

    let client = nexus_client(3)?;
    let url = nexus_url("/api/hex-agents/connect");

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
        // Walk PPID chain to find the `claude` process — our immediate parent
        // may be a transient shell spawned by Claude Code to run hooks.
        let claude_pid = find_ancestor_claude_pid();
        let state = SessionState {
            agent_id: agent_id.to_string(),
            name: agent_name.clone(),
            project: project_name.to_string(),
            registered_at: now.clone(),
            claude_pid,
            last_heartbeat: Some(now),
            edits: 0,
            workplan_id: None,
            swarm_id: None,
            current_task_id: None,
            phase: None,
            worktree_path: None,
            allowed_paths: Vec::new(),
            worktree_branch: None,
        };
        state.save()?;

        println!("  Agent:   {} ({})", "registered".green(), agent_name);
    }

    Ok(())
}

/// SubagentStart — read stdin for HEXFLO_TASK:{uuid}, auto-assign the task.
/// ADR-2603221939 P2: Hardened with heartbeat, lazy connect, and ownership validation.
async fn subagent_start() -> Result<()> {
    let stdin = std::io::read_to_string(std::io::stdin()).unwrap_or_default();

    // Spec S08: block non-worktree execution when HEXFLO_TASK is present in the prompt
    if stdin.contains("HEXFLO_TASK:") {
        let cwd = std::env::current_dir().unwrap_or_default();
        // Git worktrees have a .git FILE (not directory); the project root has a .git DIR
        let in_worktree = cwd.join(".git").is_file();
        if !in_worktree {
            eprintln!("worktree_required: swarm agents must run in an isolated worktree, not the project root");
            eprintln!("  cwd: {}", cwd.to_string_lossy());
            eprintln!("  hint: use 'hex swarm' to spawn agents in isolated worktrees");
            std::process::exit(1);
        }
    }

    // P2.3: Send heartbeat so parent agent doesn't go stale during subagent work
    let _ = send_heartbeat().await;

    // Look for HEXFLO_TASK:{uuid} pattern in the subagent prompt
    let task_id = extract_hexflo_task(&stdin);
    if task_id.is_none() {
        return Ok(()); // No task reference — nothing to sync
    }
    let task_id = task_id.unwrap();

    // Resolve agent_id from session state
    let mut state = match SessionState::load() {
        Some(s) => s,
        None => return Ok(()),
    };

    // P2.2: Lazy agent connect if session has no agent_id yet
    if state.agent_id.is_empty() {
        if let Ok(client) = nexus_client(2) {
            let project_dir = std::env::var("CLAUDE_PROJECT_DIR").unwrap_or_default();
            let body = serde_json::json!({
                "name": format!("agent-{}", std::env::var("CLAUDE_SESSION_ID").unwrap_or_default()),
                "project_dir": project_dir,
            });
            if let Ok(resp) = client.post(nexus_url("/api/hex-agents/connect")).json(&body).send().await {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    if let Some(id) = data["agent_id"].as_str() {
                        state.agent_id = id.to_string();
                        let _ = state.save();
                    }
                }
            }
        }
    }

    // Assign the task via nexus REST API
    let client = nexus_client(2)?;
    let url = nexus_url(&format!("/api/hexflo/tasks/{}", task_id));
    let _ = client
        .patch(&url)
        .json(&serde_json::json!({ "agent_id": state.agent_id }))
        .send()
        .await;

    // P4: Capture HEXFLO_WORKPLAN:{id} if present in subagent prompt
    if let Some(wp_id) = extract_prefixed_value(&stdin, "HEXFLO_WORKPLAN:") {
        state.workplan_id = Some(wp_id);
    }

    // Track the mapping so SubagentStop can complete it
    state.current_task_id = Some(task_id);
    state.save()?;

    Ok(())
}

/// Extract a value after a PREFIX: marker (e.g. HEXFLO_WORKPLAN:wp-foo → "wp-foo").
fn extract_prefixed_value(text: &str, prefix: &str) -> Option<String> {
    let start = text.find(prefix)?;
    let after = &text[start + prefix.len()..];
    // Take chars until whitespace or newline
    let value: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
    if value.is_empty() { None } else { Some(value) }
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
        .post(nexus_url("/api/hexflo/memory"))
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
            .patch(nexus_url(&format!("/api/swarms/tasks/{}", task_id)))
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
            .post(nexus_url("/api/agents/disconnect"))
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

/// PreAgent — enforce HEXFLO_TASK tracking for background agents (ADR-2603221939).
///
/// Background agents (`run_in_background: true`) MUST include `HEXFLO_TASK:{uuid}`
/// in their prompt. Without it, the agent is invisible to HexFlo tracking, the
/// dashboard, and session continuity.
///
/// Exempt agent types (read-only, no code changes): Explore, Plan, claude-code-guide.
///
/// Exit codes:
///   0 = allow (foreground agent, or exempt type, or has task)
///   2 = block (background agent without HEXFLO_TASK)
async fn pre_agent() -> Result<()> {
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();

    let input: serde_json::Value = match serde_json::from_str(&tool_input) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Can't parse — allow (fail-open)
    };

    let prompt = input["prompt"].as_str().unwrap_or("");
    let subagent_type = input["subagent_type"].as_str().unwrap_or("");
    let is_background = input["run_in_background"].as_bool().unwrap_or(false);

    // Exempt agent types — read-only, no code changes
    let exempt_types = ["Explore", "Plan", "claude-code-guide", "code-explorer"];
    if exempt_types.iter().any(|t| subagent_type.eq_ignore_ascii_case(t)) {
        return Ok(());
    }

    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
    let mode = enforcement_mode(&project_dir);

    // ADR-2603221939: Check workplan requirement for code-writing agents
    if is_background {
        let has_workplan = SessionState::load()
            .and_then(|s| s.workplan_id)
            .is_some();

        if !has_workplan {
            if mode == "mandatory" {
                println!(
                    "\u{26d4} Background agent blocked — no active workplan (ADR-2603221939)"
                );
                println!("  Pipeline: ADR → Workplan → Swarm → Agent");
                println!("  Create a workplan first: hex plan create <requirements> --adr <ADR-ID>");
                std::process::exit(2);
            } else {
                println!(
                    "\u{26a0}\u{fe0f} Agent spawned without active workplan — work may not be tracked"
                );
            }
        }
    }

    // ADR-2603232000: Check active swarm exists for background agents
    if is_background {
        let has_swarm = SessionState::load()
            .and_then(|s| s.swarm_id)
            .is_some();

        if !has_swarm {
            if mode == "mandatory" {
                println!(
                    "\u{26d4} Background agent blocked — no active HexFlo swarm (ADR-2603232000)"
                );
                println!("  Pipeline: ADR → Workplan → Swarm → Task → Agent");
                println!("  Create a swarm first: hex swarm init <name>");
                std::process::exit(2);
            } else {
                println!(
                    "\u{26a0}\u{fe0f} Agent spawned without active swarm — coordination disabled"
                );
            }
        }
    }

    // Check for HEXFLO_TASK:{uuid} in prompt
    let has_task = extract_hexflo_task(prompt).is_some();

    if is_background && !has_task {
        // BLOCK: background agent without task tracking
        println!(
            "\u{26d4} Background agent blocked — missing HEXFLO_TASK:{{uuid}} in prompt (ADR-2603221939)"
        );
        println!("  Create a swarm and task first:");
        println!("    hex swarm init <name>");
        println!("    hex task create <swarm_id> <title>");
        println!("  Then include HEXFLO_TASK:{{task_id}} as the first line of the agent prompt.");
        std::process::exit(2);
    }

    if !is_background && !has_task {
        // ADVISORY: foreground agent without tracking — warn but allow
        println!(
            "\u{26a0}\u{fe0f} Agent spawned without HEXFLO_TASK — work won't be tracked in HexFlo"
        );
    }

    // P4: Propagate workplan context — output HEXFLO_WORKPLAN so subagent inherits it
    if let Some(state) = SessionState::load() {
        if let Some(ref wp_id) = state.workplan_id {
            println!("HEXFLO_WORKPLAN:{}", wp_id);
        }
    }

    // If task present, validate it exists in an active swarm (best-effort)
    if let Some(task_id) = extract_hexflo_task(prompt) {
        if let Ok(client) = nexus_client(2) {
            let url = nexus_url(&format!("/api/hexflo/tasks/{}", task_id));
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    // ADR-2603232000: Validate parent swarm is active
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let swarm_status = body["swarmStatus"].as_str().unwrap_or("unknown");
                        if swarm_status != "active" {
                            if mode == "mandatory" {
                                println!(
                                    "\u{26d4} HEXFLO_TASK:{} belongs to {} swarm — cannot proceed (ADR-2603232000)",
                                    &task_id[..8.min(task_id.len())],
                                    swarm_status
                                );
                                std::process::exit(2);
                            } else {
                                println!(
                                    "\u{26a0}\u{fe0f} HEXFLO_TASK:{} belongs to {} swarm — proceeding in advisory mode",
                                    &task_id[..8.min(task_id.len())],
                                    swarm_status
                                );
                            }
                        }
                    }
                }
                Ok(resp) if resp.status().as_u16() == 404 => {
                    println!(
                        "\u{26d4} HEXFLO_TASK:{} not found — create the task first", &task_id[..8.min(task_id.len())]
                    );
                    std::process::exit(2);
                }
                _ => {
                    // Nexus unreachable — degrade to advisory (don't block offline work)
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
                        // ADR-2603221939 P3: use println not eprintln so Claude sees the warning
                        println!(
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

    // ADR-060: Check agent inbox for critical notifications
    let _ = check_inbox().await;

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
        .strip_prefix(project_dir.to_string_lossy().as_ref())
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
        .strip_prefix(project_dir.to_string_lossy().as_ref())
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

// ── Agent Notification Inbox (ADR-060) ───────────────────────────────

/// Check for unacknowledged critical notifications in the agent's inbox.
/// Priority-2 messages are always shown. Priority 0-1 are shown once
/// (tracked via session state `last_inbox_check` timestamp).
///
/// For `restart` notifications: automatically saves session state to HexFlo
/// memory before prompting the user, so the next session can recover context.
async fn check_inbox() -> Result<()> {
    let state = match SessionState::load() {
        Some(s) if !s.agent_id.is_empty() => s,
        _ => return Ok(()),
    };

    let client = match nexus_client(2) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    // Only check critical (priority 2) — always re-delivered until acked
    let url = nexus_url(&format!(
        "/api/hexflo/inbox/{}?min_priority=2&unacked_only=true",
        state.agent_id
    ));

    let resp = match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return Ok(()),
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(_) => return Ok(()),
    };

    let notifications = body["notifications"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    if notifications.is_empty() {
        return Ok(());
    }

    // ADR-060 step 8: For restart notifications, save state BEFORE prompting
    let has_restart = notifications.iter().any(|n| n["kind"].as_str() == Some("restart"));
    if has_restart {
        let _ = save_restart_checkpoint(&state, &client).await;
    }

    // Print to stdout — this gets injected into Claude's context
    println!();
    println!("\u{26a0} CRITICAL NOTIFICATION(S) \u{2014} action required:");
    for n in &notifications {
        let kind = n["kind"].as_str().unwrap_or("unknown");
        let payload = n["payload"].as_str().unwrap_or("{}");
        let id = n["id"].as_u64().unwrap_or(0);
        println!("  [{}] #{}: {}", kind, id, payload);
    }
    println!();

    if has_restart {
        println!("Session state has been saved automatically.");
        println!("To acknowledge and restart: hex inbox ack <id>, then restart your session.");
        println!("The next session will recover your workplan/task/swarm context.");
    } else {
        println!("To acknowledge: hex inbox ack <id>");
    }
    println!();

    Ok(())
}

/// Save a restart checkpoint to HexFlo memory (ADR-060 step 8).
/// Stores current session state so the next session can recover context.
async fn save_restart_checkpoint(state: &SessionState, client: &reqwest::Client) -> Result<()> {
    let session_id = std::env::var("CLAUDE_SESSION_ID").unwrap_or_default();
    let checkpoint = serde_json::json!({
        "agent_id": state.agent_id,
        "agent_name": state.name,
        "project": state.project,
        "workplan_id": state.workplan_id,
        "swarm_id": state.swarm_id,
        "current_task_id": state.current_task_id,
        "phase": state.phase,
        "edits": state.edits,
        "session_id": session_id,
        "saved_at": chrono::Utc::now().to_rfc3339(),
    });

    // Store under a well-known key so session_start can find it
    let memory_key = format!("restart:checkpoint:{}", state.agent_id);
    let _ = client
        .post(nexus_url("/api/hexflo/memory"))
        .json(&serde_json::json!({
            "key": memory_key,
            "value": checkpoint.to_string(),
            "scope": "project",
        }))
        .send()
        .await;

    Ok(())
}

// ── Nexus communication ──────────────────────────────────────────────

async fn check_nexus_health() -> Result<serde_json::Value> {
    let client = nexus_client(2)?;
    let resp = client.get(nexus_url("/api/health")).send().await?.error_for_status()?;
    let body: serde_json::Value = resp.json().await?;
    Ok(body)
}

async fn notify_nexus_edit(_project_dir: &PathBuf, file_path: &str) -> Result<()> {
    let client = nexus_client(1)?;
    let _ = client
        .post(nexus_url("/api/events"))
        .json(&serde_json::json!({
            "type": "file_edit",
            "path": file_path,
        }))
        .send()
        .await;
    Ok(())
}

// ── ADR-050: Lifecycle helpers ───────────────────────────────────────

/// Recover context from a restart checkpoint saved by a previous session (ADR-060 step 8).
/// If a checkpoint exists for this agent, inject the workplan/task/swarm context
/// into the current session state and print a recovery banner.
async fn recover_restart_checkpoint() -> Result<()> {
    let state = match SessionState::load() {
        Some(s) if !s.agent_id.is_empty() => s,
        _ => return Ok(()),
    };

    let client = match nexus_client(2) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    let memory_key = format!("restart:checkpoint:{}", state.agent_id);
    let encoded_key: String = memory_key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect();
    let url = nexus_url(&format!("/api/hexflo/memory/{}", encoded_key));

    let resp = match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return Ok(()),
    };

    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(_) => return Ok(()),
    };

    let checkpoint: serde_json::Value = match body["value"]
        .as_str()
        .and_then(|v| serde_json::from_str(v).ok())
    {
        Some(cp) => cp,
        None => return Ok(()),
    };

    // Restore session state from checkpoint
    let mut state = state;
    if let Some(wp) = checkpoint["workplan_id"].as_str() {
        state.workplan_id = Some(wp.to_string());
    }
    if let Some(sw) = checkpoint["swarm_id"].as_str() {
        state.swarm_id = Some(sw.to_string());
    }
    if let Some(ph) = checkpoint["phase"].as_str() {
        state.phase = Some(ph.to_string());
    }
    // Don't restore current_task_id — the task may have been reclaimed
    let _ = state.save();

    // Print recovery banner
    let prev_session = checkpoint["session_id"].as_str().unwrap_or("unknown");
    let saved_at = checkpoint["saved_at"].as_str().unwrap_or("unknown");
    let prev_edits = checkpoint["edits"].as_u64().unwrap_or(0);

    println!(
        "  {} Recovered from restart checkpoint (prev session: {}, {} edits, saved {})",
        "\u{21ba}".green(),
        &prev_session[..8.min(prev_session.len())],
        prev_edits,
        saved_at,
    );

    if let Some(wp) = &state.workplan_id {
        println!("  Restored: workplan={}", wp);
    }
    if let Some(sw) = &state.swarm_id {
        println!("  Restored: swarm={}", sw);
    }

    // Clean up the checkpoint so it's not replayed on future sessions
    let _ = client
        .delete(nexus_url(&format!("/api/hexflo/memory/{}", encoded_key)))
        .send()
        .await;

    Ok(())
}

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
                    println!("  Plan:    {} (active)", wp_id.green());
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
        state.claude_pid = find_ancestor_claude_pid();
        state.last_heartbeat = Some(now);
        state.save()?;

        // Notify Claude that registration happened (appears in hook output)
        eprintln!("  Agent:   registered (late registration)");
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
        .post(nexus_url("/api/hexflo/memory"))
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
    confirmations.contains(&trimmed)
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

/// Walk the PPID chain from this process to find the ancestor `claude` process PID.
/// Returns None if no `claude` process is found (e.g., running outside Claude Code).
fn find_ancestor_claude_pid() -> Option<u32> {
    use std::process::Command;
    let output = Command::new("ps")
        .args(["-o", "pid=,ppid=,comm=", "-ax"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);

    // Build pid → (ppid, comm) map
    let mut proc_map: std::collections::HashMap<u32, (u32, String)> =
        std::collections::HashMap::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            if let (Ok(pid), Ok(ppid)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                let comm = parts[2..].join(" ");
                proc_map.insert(pid, (ppid, comm));
            }
        }
    }

    // Walk up from our PID looking for a process named "claude"
    let mut cur = std::process::id();
    for _ in 0..10 {
        if cur <= 1 {
            break;
        }
        if let Some((ppid, comm)) = proc_map.get(&cur) {
            // Match "claude" binary (may appear as "claude" or full path ending in /claude)
            let base = comm.rsplit('/').next().unwrap_or(comm);
            if base == "claude" {
                return Some(cur);
            }
            cur = *ppid;
        } else {
            break;
        }
    }

    // Fallback: immediate parent (best effort)
    Some(std::os::unix::process::parent_id())
}
