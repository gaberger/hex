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

pub mod punch_list;

use anyhow::Result;
use clap::Subcommand;
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod checks;
use checks::{
    autofix_workplan, check_binary_freshness, check_stale_worktrees, check_workplan_status,
    FreshnessStatus,
};

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
    /// Active worktree path for current task (ADR-2026-03-23-1700)
    #[serde(default)]
    worktree_path: Option<String>,
    /// Allowed file paths for adapter boundary enforcement (ADR-2026-03-23-1700)
    #[serde(default)]
    allowed_paths: Vec<String>,
    /// Resolved worktree branch name from workplan step
    #[serde(default)]
    worktree_branch: Option<String>,
    /// RFC-3339 timestamp of last architecture fingerprint generation (ADR-2026-03-30-1200).
    /// Used to detect staleness when key project files change.
    #[serde(default)]
    fingerprint_generated_at: Option<String>,
    /// ADR-2026-04-11-0227: path to an in-flight workplan draft spawned by
    /// `hex plan draft` (auto-invoked when a T3 prompt is detected).
    /// Cleared on SessionEnd or when the user approves/clears the draft.
    #[serde(default)]
    pending_workplan_draft: Option<String>,
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

    fn set_worktree(&mut self, path: &str) {
        self.worktree_path = Some(path.to_string());
    }

    fn add_allowed_path(&mut self, path: &str) {
        if !self.allowed_paths.contains(&path.to_string()) {
            self.allowed_paths.push(path.to_string());
        }
    }

    /// Returns true if the given path is permitted for this session.
    ///
    /// Fail-open: when `allowed_paths` is empty all paths are allowed.
    /// Cross-cutting directories (`docs/`, `tests/`, `config/`, `.hex/`) are
    /// always allowed regardless of the allow-list.
    fn is_path_allowed(&self, path: &str) -> bool {
        const ALWAYS_ALLOWED: &[&str] = &["docs/", "tests/", "config/", ".hex/"];
        if self.allowed_paths.is_empty() {
            return true;
        }
        if ALWAYS_ALLOWED.iter().any(|prefix| path.contains(prefix)) {
            return true;
        }
        self.allowed_paths.iter().any(|allowed| path.starts_with(allowed.as_str()))
    }
}

/// Check lifecycle enforcement mode for this project.
/// Default is "mandatory" — all hex projects enforce the ADR → workplan → code pipeline.
/// Set "lifecycle_enforcement": "advisory" in .hex/project.json to downgrade to warnings only.
fn enforcement_mode(project_dir: &Path) -> &'static str {
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

/// ADR-2026-04-11-0227: Check whether auto-invoking the planner on T3
/// work-intent prompts is enabled for this project.
///
/// Precedence (highest to lowest):
/// 1. `HEX_AUTO_PLAN` env var — `0` disables, anything else enables
/// 2. `workplan.auto_invoke.enabled` in `.hex/project.json` — bool
/// 3. Default: `true` (opt-out, not opt-in)
///
/// The env var is checked first so individual shells can disable without
/// touching the project config file (useful for one-off debugging or for
/// CI environments that want deterministic behavior).
fn auto_plan_enabled(project_dir: &Path) -> bool {
    // Env var takes precedence
    if let Ok(val) = std::env::var("HEX_AUTO_PLAN") {
        return val != "0" && val.to_lowercase() != "false";
    }

    // Then project config
    let project_json = project_dir.join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(project) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(enabled) = project["workplan"]["auto_invoke"]["enabled"].as_bool() {
                return enabled;
            }
        }
    }

    // Default: enabled
    true
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
    /// Before a tool call — fire-and-forget POST to /api/events (ADR-2026-04-01-2137)
    ObservePre,
    /// After a tool call — fire-and-forget POST to /api/events (ADR-2026-04-01-2137)
    ObservePost,
    /// On Stop — fire-and-forget POST to /api/events with hook output (ADR-2026-04-01-2137)
    ObserveStop,
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
        HookEvent::ObservePre => observe("PreToolUse").await,
        HookEvent::ObservePost => observe("PostToolUse").await,
        HookEvent::ObserveStop => observe("Stop").await,
    }
}

// ── Event handlers ───────────────────────────────────────────────────

async fn session_start(project_dir: &Path) -> Result<()> {
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
    println!("  Project: {} ({})", name, id.get(..8).unwrap_or(id));

    // ADR-060: recover context from a previous session's checkpoint.
    let _ = recover_restart_checkpoint().await;
    // ADR-050: the active workplan, from local memory.
    let _ = load_workplan_context(id).await;

    // ADR-2026-03-30-1200: Inject architecture fingerprint into Claude Code
    // context. stdout is picked up as session context — never skip this.
    println!("\n{}", fingerprint_block(id, project_dir, name).await);

    // ── Workplan reconciliation ──────────────────────
    // Reconcile workplan task statuses against git history on every
    // session start so agents never act on stale "todo" states.
    match check_workplan_status() {
        Ok(summaries) if summaries.is_empty() => {}
        Ok(summaries) => {
            let total_stale: usize = summaries.iter().map(|s| s.stale_tasks.len()).sum();
            let mut total_fixed = 0usize;

            if total_stale > 0 {
                for wp in &summaries {
                    if let Ok(n) = autofix_workplan(wp) {
                        total_fixed += n;
                    }
                }
            }

            if total_stale == 0 {
                println!(
                    "  Workplans: {} {} active, all consistent",
                    "\u{2713}".green(),
                    summaries.len()
                );
            } else if total_fixed == total_stale {
                println!(
                    "  Workplans: {} {} active, reconciled {} stale {}",
                    "\u{2713}".green(),
                    summaries.len(),
                    total_fixed,
                    "[auto-fixed]".cyan()
                );
            } else {
                println!(
                    "  Workplans: {} {} active, {}/{} stale reconciled",
                    "\u{2717}".red(),
                    summaries.len(),
                    total_fixed,
                    total_stale
                );
            }
        }
        Err(_) => {}
    }

    // ── Brain validate: stale worktree detection ───────────────────
    match check_stale_worktrees() {
        Ok(stale) if stale.is_empty() => {}
        Ok(stale) => {
            let branches: Vec<&str> = stale.iter().map(|w| w.branch.as_str()).collect();
            println!(
                "  Worktrees: {} {} stale (>24h): {}",
                "\u{26a0}".yellow(),
                stale.len(),
                branches.join(", ")
            );
        }
        Err(_) => {}
    }

    // Check for architecture violations
    let src_dir = project_dir.join("src");
    if src_dir.exists() {
        println!("  Arch:    run `hex analyze .` to check health");
    }

    // ADR-2026-03-22-1939: Auto-upgrade settings — ensure Agent PreToolUse hook exists
    ensure_agent_hook(project_dir);

    Ok(())
}

/// Generate a minimal architecture fingerprint block from local project files.
///
/// Used as a fallback when nexus is offline or fingerprint generation failed.
/// Reads go.mod / Cargo.toml / package.json for language detection and any
/// active workplan for objective. Output matches the injection format from
/// ADR-2026-03-30-1200 §3, trimmed to the most essential fields.
fn minimal_fingerprint_block(project_dir: &Path, project_name: &str) -> String {
    minimal_fingerprint_block_inner(project_dir, project_name, false)
}

fn minimal_fingerprint_block_cached(project_dir: &Path, project_name: &str) -> String {
    minimal_fingerprint_block_inner(project_dir, project_name, true)
}

fn minimal_fingerprint_block_inner(project_dir: &Path, project_name: &str, nexus_online: bool) -> String {
    let mut language = "unknown".to_string();
    let mut framework = "unknown".to_string();
    let mut output_type = "unknown".to_string();
    let mut objective = String::new();

    // Detect language + framework
    if project_dir.join("Cargo.toml").exists() {
        language = "rust".to_string();
        // Peek at Cargo.toml for binary vs library
        if let Ok(ct) = std::fs::read_to_string(project_dir.join("Cargo.toml")) {
            if ct.contains("[[bin]]") || ct.contains("[package]") {
                output_type = "binary".to_string();
            }
            if ct.contains("axum") {
                framework = "axum".to_string();
                output_type = "web-api".to_string();
            } else if ct.contains("clap") {
                framework = "clap".to_string();
                output_type = "cli".to_string();
            }
        }
    } else if project_dir.join("go.mod").exists() {
        language = "go".to_string();
        if let Ok(gm) = std::fs::read_to_string(project_dir.join("go.mod")) {
            if gm.contains("gin-gonic") {
                framework = "gin".to_string();
                output_type = "web-api".to_string();
            } else {
                framework = "stdlib".to_string();
            }
        }
    } else if project_dir.join("package.json").exists() {
        language = "typescript".to_string();
        if let Ok(pj) = std::fs::read_to_string(project_dir.join("package.json")) {
            if pj.contains("\"react\"") { framework = "react".to_string(); output_type = "web-app".to_string(); }
            else if pj.contains("\"next\"") { framework = "next.js".to_string(); output_type = "web-app".to_string(); }
            else if pj.contains("\"axum\"") { framework = "axum".to_string(); }
        }
    }

    // Extract objective from most recent workplan
    let workplans_dir = project_dir.join("docs/workplans");
    if workplans_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&workplans_dir) {
            let mut files: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .collect();
            // Sort by modification time descending — most recent first
            files.sort_by_key(|e| std::cmp::Reverse(
                e.metadata().and_then(|m| m.modified()).ok()
            ));
            'outer: for entry in files.iter().take(3) {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(wp) = serde_json::from_str::<serde_json::Value>(&content) {
                        for field in &["objective", "description", "title"] {
                            if let Some(s) = wp[field].as_str().filter(|s| !s.is_empty()) {
                                objective = s.chars().take(120).collect();
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut block = format!(
        "## Project Architecture Context\n\
         Project: {} | Language: {} | Framework: {} | Output: {}\n",
        project_name, language, framework, output_type
    );
    if !objective.is_empty() {
        block.push_str(&format!("Objective: {}\n", objective));
    }
    let note = if nexus_online {
        "Note: fingerprint not cached — run `hex fingerprint generate` for full context."
    } else {
        "Note: nexus offline — run `hex nexus start` then `hex fingerprint generate` for full context."
    };
    block.push_str(&format!("{}\n---", note));
    block
}

/// Ensure `.claude/settings.json` has the Agent PreToolUse hook (ADR-2026-03-22-1939).
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
                println!("  Hooks:   {} Agent enforcement auto-installed (ADR-2026-03-22-1939)", "\u{2713}".green());
            }
        }
    }
}

/// SubagentStart — read stdin for HEXFLO_TASK:{uuid}, auto-assign the task.
/// ADR-2026-03-22-1939 P2: Hardened with heartbeat, lazy connect, and ownership validation.
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

    // The agent id came from the daemon's roster and the task assignment from
    // its HexFlo table. Neither exists; the session file is the state.
    if state.agent_id.is_empty() {
        state.agent_id = format!(
            "agent-{}",
            std::env::var("CLAUDE_SESSION_ID").unwrap_or_else(|_| "local".into())
        );
        let _ = state.save();
    }

    // P4: Capture HEXFLO_WORKPLAN:{id} if present in subagent prompt
    if let Some(wp_id) = extract_prefixed_value(&stdin, "HEXFLO_WORKPLAN:") {
        state.workplan_id = Some(wp_id);
    }

    // Extract swarm_id for tier gate enforcement
    let swarm_id = extract_prefixed_value(&stdin, "HEXFLO_SWARM:");

    // The worktree branch and the tier gate both came from the HexFlo task
    // table, which the daemon owned. A subagent runs in this process now, on
    // the branch the operator is on.
    let _ = &swarm_id;

    // Track the mapping so SubagentStop can complete it
    state.current_task_id = Some(task_id);
    state.save()?;

    Ok(())
}

/// Step 3: create a git worktree for `branch` (if it doesn't already exist) and
/// populate `state.allowed_paths` from the branch name's embedded layer segment.
/// Fail-open — logs warnings but never panics or exits non-zero.
fn ensure_worktree_exists(branch: &str, project_dir: &Path, state: &mut SessionState) {
    // 1. Determine repo root
    let repo_root = match std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_dir)
        .output()
    {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        _ => {
            eprintln!("[hex hook] step3: could not determine repo root, skipping worktree creation");
            // Still populate allowed_paths even without a worktree
            for p in derive_allowed_paths(branch) {
                state.add_allowed_path(&p);
            }
            return;
        }
    };

    // 2. Check concurrent worktree cap (max 8; `git worktree list` always includes
    //    the main worktree so threshold is 9 lines)
    let worktree_count = std::process::Command::new("git")
        .args(["worktree", "list"])
        .current_dir(&repo_root)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).lines().count())
        .unwrap_or(0);
    if worktree_count >= 9 {
        eprintln!(
            "[hex hook] step3: max concurrent worktrees reached ({}/8), skipping creation for branch '{}'",
            worktree_count - 1,
            branch
        );
        for p in derive_allowed_paths(branch) {
            state.add_allowed_path(&p);
        }
        return;
    }

    // 3. Check if the worktree already exists (porcelain output contains "branch refs/heads/<branch>")
    let porcelain_out = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repo_root)
        .output();
    let already_exists = if let Ok(out) = porcelain_out {
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        text.lines().any(|line| line == format!("branch refs/heads/{}", branch))
    } else {
        false
    };

    if !already_exists {
        // 4. Create the worktree: `git worktree add hex-worktrees-{branch} -b {branch}`
        let worktree_dir = format!("hex-worktrees-{}", branch);
        let add_out = std::process::Command::new("git")
            .args(["worktree", "add", &worktree_dir, "-b", branch])
            .current_dir(&repo_root)
            .output();
        match add_out {
            Ok(out) if out.status.success() => {
                eprintln!("[hex hook] step3: created worktree '{}' for branch '{}'", worktree_dir, branch);
                // Update worktree_path to the absolute path
                let abs_path = std::path::Path::new(&repo_root).join(&worktree_dir);
                state.worktree_path = Some(abs_path.to_string_lossy().to_string());
            }
            Ok(out) => {
                eprintln!(
                    "[hex hook] step3: git worktree add failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Err(e) => {
                eprintln!("[hex hook] step3: could not run git worktree add: {}", e);
            }
        }
    } else {
        eprintln!("[hex hook] step3: worktree for branch '{}' already exists, skipping", branch);
        // Still update worktree_path
        let worktree_dir = format!("hex-worktrees-{}", branch);
        let abs_path = std::path::Path::new(&repo_root).join(&worktree_dir);
        state.worktree_path = Some(abs_path.to_string_lossy().to_string());
    }

    // 5. Populate allowed_paths from layer derived from branch name
    for p in derive_allowed_paths(branch) {
        state.add_allowed_path(&p);
    }
}

/// Derive the set of allowed file paths for a worktree branch based on the
/// layer/adapter encoded in the branch name.
///
/// Branch naming conventions:
///   `feat-<feature>-domain`                  → domain layer
///   `feat-<feature>-ports`                   → ports layer
///   `feat-<feature>-secondary-<name>`        → secondary adapter
///   `feat-<feature>-primary-<name>`          → primary adapter
///   `feat-<feature>-usecases`                → usecases layer
///   anything else                            → no layer-specific paths (only universal)
///
/// Universal paths (always appended): `docs/`, `tests/`, `config/`, `.hex/`
fn derive_allowed_paths(branch: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();

    // Normalise: lower-case, replace underscores with hyphens
    let b = branch.to_lowercase().replace('_', "-");

    if b.ends_with("-domain") || b.contains("-domain-") {
        paths.push("src/core/domain/".to_string());
    } else if b.ends_with("-ports") || b.contains("-ports-") {
        paths.push("src/core/ports/".to_string());
    } else if b.ends_with("-usecases") || b.contains("-usecases-") {
        paths.push("src/usecases/".to_string());
    } else if let Some(idx) = b.find("-secondary-") {
        let adapter_name = &b[idx + "-secondary-".len()..];
        // Strip any trailing segments after the adapter name (e.g. worktree suffix)
        let name = adapter_name.split('-').next().unwrap_or(adapter_name);
        paths.push(format!("src/adapters/secondary/{}/", name));
        // Secondary adapters may also touch Rust crates
        paths.push("hex-nexus/src/".to_string());
        paths.push("hex-agent/src/".to_string());
        paths.push("hex-cli/src/".to_string());
    } else if let Some(idx) = b.find("-primary-") {
        let adapter_name = &b[idx + "-primary-".len()..];
        let name = adapter_name.split('-').next().unwrap_or(adapter_name);
        paths.push(format!("src/adapters/primary/{}/", name));
    }

    // Universal cross-cutting paths
    for p in &["docs/", "tests/", "config/", ".hex/"] {
        paths.push(p.to_string());
    }

    paths
}

/// Walk a workplan JSON for a step whose description contains `task_title`
/// (case-insensitive substring). Returns the `worktree_branch` field if found.
fn find_branch_in_workplan(wp: &serde_json::Value, task_title: &str) -> Option<(String, String)> {
    let check_step = |step: &serde_json::Value| -> Option<(String, String)> {
        let desc = step
            .get("description")
            .or_else(|| step.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if !desc.is_empty() && (desc.contains(task_title) || task_title.contains(desc.as_str())) {
            let branch = step.get("worktree_branch")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let step_id = step.get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if branch.is_empty() { None } else { Some((branch, step_id)) }
        } else {
            None
        }
    };

    // Flat steps array
    if let Some(steps) = wp.get("steps").and_then(|v| v.as_array()) {
        for step in steps {
            if let Some(b) = check_step(step) {
                return Some(b);
            }
        }
    }

    // Phases with nested steps
    if let Some(phases) = wp.get("phases").and_then(|v| v.as_array()) {
        for phase in phases {
            if let Some(steps) = phase.get("steps").and_then(|v| v.as_array()) {
                for step in steps {
                    if let Some(b) = check_step(step) {
                        return Some(b);
                    }
                }
            }
        }
    }

    None
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

    // The completion PATCH went to the daemon's HexFlo task table. What
    // actually matters here — clearing the task and merging the worktree —
    // is local and follows.
    let _ = (&task_id, &result);

    // Clear the current task from session state
    let mut state = state;
    state.current_task_id = None;

    // Auto-merge and cleanup worktree if one was set for this task (ADR-2026-03-23-1700)
    if let Some(ref branch) = state.worktree_branch.clone() {
        let branch = branch.clone();

        // Check if subagent result indicates failure — skip merge if so
        let looks_like_failure = result.to_lowercase().contains("error")
            || result.to_lowercase().contains("failed");

        // Find repo root (fail-open)
        let repo_root_opt = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()
            .and_then(|o| if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            });

        if let Some(repo_root) = repo_root_opt {
            // Check the branch exists
            let branch_exists = std::process::Command::new("git")
                .args(["branch", "--list", &branch])
                .current_dir(&repo_root)
                .output()
                .ok()
                .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
                .unwrap_or(false);

            let mut merge_ok = false;

            if !branch_exists {
                eprintln!("[hex hook] subagent_stop: branch '{}' not found, skipping merge", branch);
            } else if looks_like_failure {
                eprintln!("[hex hook] subagent_stop: result looks like failure, skipping merge of '{}'", branch);
            } else {
                // Merge the worktree branch into current branch
                let merge_msg = format!("feat(worktree): merge {}", branch);
                let merge_out = std::process::Command::new("git")
                    .args(["merge", "--no-ff", &branch, "-m", &merge_msg])
                    .current_dir(&repo_root)
                    .output();

                match merge_out {
                    Ok(o) if o.status.success() => {
                        eprintln!("[hex hook] subagent_stop: merged branch '{}'", branch);
                        merge_ok = true;
                    }
                    Ok(o) => {
                        eprintln!(
                            "[hex hook] subagent_stop: merge of '{}' failed: {}",
                            branch,
                            String::from_utf8_lossy(&o.stderr).trim()
                        );
                    }
                    Err(e) => {
                        eprintln!("[hex hook] subagent_stop: merge command error for '{}': {}", branch, e);
                    }
                }
            }

            // Remove worktree (regardless of merge outcome)
            let worktree_dir = format!("hex-worktrees-{}", branch);
            let rm_out = std::process::Command::new("git")
                .args(["worktree", "remove", "--force", &worktree_dir])
                .current_dir(&repo_root)
                .output();

            match rm_out {
                Ok(o) if o.status.success() => {
                    eprintln!("[hex hook] subagent_stop: removed worktree '{}'", worktree_dir);
                }
                Ok(o) => {
                    eprintln!(
                        "[hex hook] subagent_stop: worktree remove failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    );
                }
                Err(e) => {
                    eprintln!("[hex hook] subagent_stop: worktree remove error: {}", e);
                }
            }

            // Delete the branch (safe delete — only if merged)
            if merge_ok {
                let del_out = std::process::Command::new("git")
                    .args(["branch", "-d", &branch])
                    .current_dir(&repo_root)
                    .output();

                match del_out {
                    Ok(o) if o.status.success() => {
                        eprintln!("[hex hook] subagent_stop: deleted branch '{}'", branch);
                    }
                    Ok(o) => {
                        eprintln!(
                            "[hex hook] subagent_stop: branch delete failed: {}",
                            String::from_utf8_lossy(&o.stderr).trim()
                        );
                    }
                    Err(e) => {
                        eprintln!("[hex hook] subagent_stop: branch delete error: {}", e);
                    }
                }
            }
        } else {
            eprintln!("[hex hook] subagent_stop: could not determine repo root, skipping worktree cleanup");
        }

        // Clear worktree state
        state.worktree_branch = None;
        state.worktree_path = None;
    }

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
    // Progress used to be flushed to HexFlo memory and the agent deregistered
    // from the daemon's roster. There is no roster.
    //
    // The checkpoint is written here now. Its only trigger used to be a
    // "restart" notification in the daemon's inbox — so in practice it fired
    // almost never, and `recover_restart_checkpoint` had nothing to find.
    // Session end is when a session actually has something worth carrying.
    if let Some(state) = SessionState::load() {
        if !state.agent_id.is_empty() {
            let _ = save_restart_checkpoint(&state);
        }
    }
    Ok(())
}

async fn pre_edit(project_dir: &Path) -> Result<()> {
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

                // ADR-2026-03-23-1700: Enforce adapter boundary via allowed_paths.
                // Only active when allowed_paths is non-empty (set by step-2 worktree setup).
                // Mode: HEX_BOUNDARY_MODE=mandatory|advisory (default: advisory for safe rollout).
                if !state.allowed_paths.is_empty() && !state.is_path_allowed(file_path) {
                    let boundary_mode = std::env::var("HEX_BOUNDARY_MODE")
                        .unwrap_or_else(|_| "advisory".to_string());
                    let msg = format!(
                        "BOUNDARY VIOLATION: {} is outside allowed adapter boundary. Allowed: {:?}",
                        file_path, state.allowed_paths
                    );
                    if boundary_mode == "mandatory" {
                        println!("{}", msg);
                        std::process::exit(2);
                    } else {
                        println!("WARNING: {}", msg);
                    }
                }
            }
        }
    }

    Ok(())
}

async fn post_edit(project_dir: &PathBuf) -> Result<()> {
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();
    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if input["file_path"].as_str().is_some() {
            // The dashboard notification and the HexFlo edit event both went
            // to the daemon. The counter is local and stays.
            if let Some(mut state) = SessionState::load() {
                state.edits += 1;
                let _ = state.save();
            }
        }
    }
    Ok(())
}

/// PreAgent — enforce HEXFLO_TASK tracking for background agents (ADR-2026-03-22-1939).
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

    // ADR-2026-03-22-1939: Check workplan requirement for code-writing agents
    if is_background {
        let has_workplan = SessionState::load()
            .and_then(|s| s.workplan_id)
            .is_some();

        if !has_workplan {
            if mode == "mandatory" {
                println!(
                    "\u{26d4} Background agent blocked — no active workplan (ADR-2026-03-22-1939)"
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

    // ADR-2026-03-23-2000: Check active swarm exists for background agents
    if is_background {
        let has_swarm = SessionState::load()
            .and_then(|s| s.swarm_id)
            .is_some();

        if !has_swarm {
            if mode == "mandatory" {
                println!(
                    "\u{26d4} Background agent blocked — no active HexFlo swarm (ADR-2026-03-23-2000)"
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
            "\u{26d4} Background agent blocked — missing HEXFLO_TASK:{{uuid}} in prompt (ADR-2026-03-22-1939)"
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

    // Swarm membership used to be validated against the daemon's HexFlo task
    // table. There are no swarms and no table.

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
                        // ADR-2026-03-22-1939 P3: use println not eprintln so Claude sees the warning
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

/// ADR-2026-03-30-1200: Refresh the architecture fingerprint when key project files have changed.
///
/// Key files: docs/adrs/*.md, docs/workplans/*.json, go.mod, Cargo.toml, package.json.
/// The last generation timestamp is cached in session state — avoiding a nexus round-trip
/// on every prompt. When stale, regenerates silently (best-effort) and prints the updated
/// fingerprint block to stdout so Claude Code picks it up as fresh context.
async fn refresh_fingerprint_if_stale(project_dir: &Path) -> Result<()> {
    // Only run when nexus is available and project is registered
    let project_json = project_dir.join(".hex/project.json");
    if !project_json.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&project_json)?;
    let project: serde_json::Value = serde_json::from_str(&content)?;
    let project_id = match project["id"].as_str() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return Ok(()),
    };

    // Load the last known fingerprint generation time from session state
    let last_generated: Option<std::time::SystemTime> = SessionState::load()
        .and_then(|s| s.fingerprint_generated_at)
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
        .map(|dt| std::time::UNIX_EPOCH + std::time::Duration::from_secs(dt.timestamp() as u64));

    // Check modification times of key project files
    let key_globs: &[&str] = &["docs/adrs", "docs/workplans", "go.mod", "Cargo.toml", "package.json"];
    let mut latest_mtime: Option<std::time::SystemTime> = None;

    for rel in key_globs {
        let full = project_dir.join(rel);
        if full.is_file() {
            if let Ok(meta) = std::fs::metadata(&full) {
                if let Ok(mtime) = meta.modified() {
                    if latest_mtime.is_none_or(|prev| mtime > prev) {
                        latest_mtime = Some(mtime);
                    }
                }
            }
        } else if full.is_dir() {
            // Check all files one level deep in the directory
            if let Ok(entries) = std::fs::read_dir(&full) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(mtime) = meta.modified() {
                            if latest_mtime.is_none_or(|prev| mtime > prev) {
                                latest_mtime = Some(mtime);
                            }
                        }
                    }
                }
            }
        }
    }

    // Determine if regeneration is needed:
    // - No fingerprint generated yet in this session, OR
    // - A key file is newer than the last generation
    let needs_refresh = match (last_generated, latest_mtime) {
        (None, _) => true,
        (Some(gen), Some(mtime)) => mtime > gen,
        (Some(_), None) => false,
    };

    if !needs_refresh {
        return Ok(());
    }

    // Best-effort regeneration — never block or fail the hook.
    let name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    println!("\n{}", fingerprint_block(&project_id, project_dir, &name).await);
    if let Some(mut state) = SessionState::load() {
        state.fingerprint_generated_at = Some(chrono::Utc::now().to_rfc3339());
        let _ = state.save();
    }

    Ok(())
}

/// The architecture fingerprint for this project, as an injection block.
///
/// Falls back to the minimal in-process block if extraction fails, because
/// blank session context is worse than a thin one.
async fn fingerprint_block(project_id: &str, project_dir: &Path, name: &str) -> String {
    let workplan = SessionState::load()
        .and_then(|s| s.workplan_id)
        .map(|id| project_dir.join(format!("docs/workplans/{}.json", id)))
        .filter(|p| p.exists());
    let fp = hex_analysis::fingerprint_extractor::FingerprintExtractor::extract(
        project_id,
        project_dir,
        workplan.as_deref(),
    )
    .await;
    let block = fp.to_injection_block();
    if block.trim().is_empty() {
        return minimal_fingerprint_block(project_dir, name);
    }
    block
}

async fn route(project_dir: &Path) -> Result<()> {
    let tool_input = std::env::var("TOOL_INPUT").unwrap_or_default();

    // ADR-2026-03-30-1200: Refresh architecture fingerprint if key project files have changed
    let _ = refresh_fingerprint_if_stale(project_dir).await;

    if let Ok(input) = serde_json::from_str::<serde_json::Value>(&tool_input) {
        if let Some(content) = input["content"].as_str() {
            let lower = content.to_lowercase();

            // Detect hex-relevant intents and provide context hints
            let hints = classify_prompt(&lower);
            if !hints.is_empty() {
                println!("[HEX] {}", hints.join(", "));
            }

            // ADR-2026-04-11-0227: Three-tier work-intent classifier.
            //
            // Replaces the old passive-warning path with active tier dispatch:
            //   T1Todo      → silent, let Claude's TodoWrite handle it
            //   T2MiniPlan  → one-line suggestion, no auto-invocation
            //   T3Workplan  → auto-invoke `hex plan draft --background`
            //                 to create a draft stub + surface it in context
            if let Some(mut state) = SessionState::load() {
                if state.workplan_id.is_none() && state.pending_workplan_draft.is_none() {
                    let mode = enforcement_mode(project_dir);
                    let auto_plan_enabled = auto_plan_enabled(project_dir);
                    let tier = classify_work_intent(&lower);

                    // P2.2: Archive stale task.json when on main with a new T2/T3 task.
                    // Worktree branches have a valid task.json — only archive on main.
                    if matches!(tier, Tier::T2MiniPlan | Tier::T3Workplan) {
                        let task_json = project_dir.join(".hex/task.json");
                        if task_json.exists() {
                            let on_main = std::process::Command::new("git")
                                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                                .output()
                                .ok()
                                .and_then(|o| {
                                    if o.status.success() {
                                        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                                    } else {
                                        None
                                    }
                                })
                                .map(|branch| branch == "main")
                                .unwrap_or(false);

                            if on_main {
                                let history_dir = project_dir.join(".hex/task-history");
                                let _ = std::fs::create_dir_all(&history_dir);
                                let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
                                let dest = history_dir.join(format!("{}.json", ts));
                                if let Err(e) = std::fs::rename(&task_json, &dest) {
                                    eprintln!("hex: failed to archive task.json: {}", e);
                                } else {
                                    eprintln!("hex: archived stale task.json → .hex/task-history/{}.json", ts);
                                }
                            }
                        }
                    }

                    match tier {
                        Tier::T1Todo => {
                            // No action — let the host agent handle it.
                            // Confirmatory replies fall through silently.
                        }
                        Tier::T2MiniPlan => {
                            // One-line suggestion, no auto-invocation.
                            println!(
                                "[HEX] mini-plan scope detected — consider `hex plan create <name>` if this grows"
                            );
                        }
                        Tier::T3Workplan => {
                            // Feature-sized intent. In advisory+enabled mode, auto-invoke
                            // `hex plan draft` in the background. In mandatory mode,
                            // also auto-invoke but still require user approval before
                            // running coders (existing enforcement chain does this).
                            if auto_plan_enabled {
                                match spawn_plan_draft(content) {
                                    Ok(draft_path) => {
                                        println!(
                                            "[HEX] feature-sized task detected \u{2192} drafting workplan in background"
                                        );
                                        println!(
                                            "[HEX] draft: {} (run `hex plan drafts list` to see it, `/hex-feature-dev` to expand)",
                                            draft_path
                                        );
                                        state.pending_workplan_draft = Some(draft_path);
                                        let _ = state.save();
                                    }
                                    Err(e) => {
                                        // Fall back to the old warning/block behavior if spawn fails
                                        if mode == "mandatory" {
                                            println!(
                                                "BLOCKED: Cannot proceed without an active workplan. Run: hex plan create <name> (draft spawn failed: {})",
                                                e
                                            );
                                            std::process::exit(2);
                                        } else {
                                            println!(
                                                "WARNING: No active workplan. Consider: hex plan create <name> (draft spawn failed: {})",
                                                e
                                            );
                                        }
                                    }
                                }
                            } else if mode == "mandatory" {
                                // Auto-plan disabled but enforcement is mandatory — keep old behavior
                                println!(
                                    "BLOCKED: Cannot proceed without an active workplan. Run: hex plan create <name>"
                                );
                                std::process::exit(2);
                            } else {
                                // Auto-plan disabled, advisory mode — keep old warning
                                println!(
                                    "WARNING: No active workplan for this work. Consider: hex plan create <name>"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// ADR-2026-04-11-0227: Spawn `hex plan draft` in the background with the user prompt.
///
/// Returns the path of the created draft file on success. The draft is
/// a minimal stub quarantined to `docs/workplans/drafts/` — no worktrees,
/// no specs, no coder dispatch. The user (or Claude Code) picks it up
/// via `/hex-feature-dev` or `hex plan drafts approve`.
fn spawn_plan_draft(prompt: &str) -> Result<String> {
    // We run `hex plan draft --background <prompt>` synchronously here
    // (not via detached spawn) because we need the resulting draft path
    // to surface in the hook output. The draft command itself is fast:
    // it just writes a JSON stub. There's no LLM call on this path.
    let hex_bin = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("hex"));

    let output = std::process::Command::new(&hex_bin)
        .args(["plan", "draft", "--background", prompt])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!(
            "hex plan draft failed: {}",
            stderr.trim()
        ));
    }

    // The draft filename is deterministic (ts + slug) but we re-derive
    // it here by reading the newest file in docs/workplans/drafts/.
    let drafts_dir = Path::new("docs/workplans/drafts");
    let newest = std::fs::read_dir(drafts_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().and_then(|s| s.to_str()) == Some("json")
        })
        .max_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .unwrap_or_else(|| "docs/workplans/drafts/".to_string());

    Ok(newest)
}

// ── Boundary validation ──────────────────────────────────────────────

fn validate_boundary_edit(project_dir: &Path, file_path: &str) -> Result<()> {
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
fn validate_workplan_boundary(project_dir: &Path, file_path: &str, workplan_id: &str) -> Result<()> {
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

/// Save a restart checkpoint so the next session can pick up where this one
/// stopped (ADR-060 step 8).
///
/// Was a POST to the daemon's HexFlo memory table. It is a local memory entry
/// now — which also means a checkpoint survives when nothing is running.
fn save_restart_checkpoint(state: &SessionState) -> Result<()> {
    let checkpoint = serde_json::json!({
        "agent_id": state.agent_id,
        "agent_name": state.name,
        "project": state.project,
        "workplan_id": state.workplan_id,
        "current_task_id": state.current_task_id,
        "phase": state.phase,
        "edits": state.edits,
        "session_id": std::env::var("CLAUDE_SESSION_ID").unwrap_or_default(),
        "saved_at": chrono::Utc::now().to_rfc3339(),
    });
    let _ = hex_exec::local_store::memory_put(
        &format!("restart:checkpoint:{}", state.agent_id),
        &checkpoint.to_string(),
    );
    Ok(())
}

/// ── ADR-050: Lifecycle helpers ───────────────────────────────────────

/// Recover context from a checkpoint a previous session saved (ADR-060 step 8).
async fn recover_restart_checkpoint() -> Result<()> {
    let Some(mut state) = SessionState::load().filter(|s| !s.agent_id.is_empty()) else {
        return Ok(());
    };
    let key = format!("restart:checkpoint:{}", state.agent_id);
    let Some(raw) = hex_exec::local_store::memory_get(&key) else {
        return Ok(());
    };
    let Ok(cp) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };

    let field = |k: &str| cp.get(k).and_then(|v| v.as_str()).map(String::from);
    state.workplan_id = field("workplan_id").or(state.workplan_id);
    state.current_task_id = field("current_task_id").or(state.current_task_id);
    state.phase = field("phase").or(state.phase);
    let _ = state.save();

    println!("  {} recovered from checkpoint", "\u{21ba}".cyan());
    if let Some(ref wp) = state.workplan_id {
        println!("  Plan:    {}", wp.green());
    }
    // One-shot: a checkpoint consumed is a checkpoint spent, or every future
    // session recovers the same stale context.
    let _ = hex_exec::local_store::memory_delete(&key);
    Ok(())
}

/// Load the active workplan from local memory into session state (ADR-050).
async fn load_workplan_context(project_id: &str) -> Result<()> {
    let Some(raw) = hex_exec::local_store::memory_get(&format!("workplan:active:{project_id}"))
    else {
        return Ok(());
    };
    let Ok(wp) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(());
    };
    let Some(wp_id) = wp.get("workplan_id").and_then(|v| v.as_str()) else {
        return Ok(());
    };
    if let Some(mut state) = SessionState::load() {
        state.phase = wp.get("phase").and_then(|v| v.as_str()).map(String::from);
        state.workplan_id = Some(wp_id.to_string());
        let _ = state.save();
    }
    println!("  Plan:    {} (active)", wp_id.green());
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

// ── ADR-2026-04-11-0227: Three-tier work-intent classifier ─────────────────
//
// The existing `classify_prompt()` emits context hints; this classifier
// decides whether a prompt is a:
//   T1Todo       — small/conversational, let the host agent handle it
//   T2MiniPlan   — work-sized but scoped within one adapter boundary
//   T3Workplan   — feature-sized, cross-adapter, warrants a full workplan
//
// The route() hook dispatches on Tier to decide whether to auto-invoke
// the planner (T3), suggest a mini-plan (T2), or stay silent (T1).

/// Task sizing tiers for intent classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Small change or conversational — no hex planning artifact needed.
    T1Todo,
    /// Work-sized but contained within one adapter boundary.
    T2MiniPlan,
    /// Feature-sized, cross-adapter — warrants a full workplan.
    T3Workplan,
}

impl Tier {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T1Todo => "T1",
            Tier::T2MiniPlan => "T2",
            Tier::T3Workplan => "T3",
        }
    }
}

/// Classify a user prompt into a work-sizing Tier.
///
/// Uses a weighted keyword scoring heuristic:
///   - Feature-sized verbs + noun subsystems score high (T3)
///   - Generic work verbs without scope signals score medium (T2)
///   - Trivial edits, questions, and conversational replies score T1
///
/// This is a deliberately conservative classifier: when in doubt, it
/// returns a lower tier. False negatives (missed T3 → T2) are cheap
/// (user manually invokes planner); false positives (T1 → T3) would
/// spawn unwanted background agents, so the threshold errs high.
///
/// Implemented as a precedence-ordered Rule table (ADR-2026-04-14-2243):
///   P0 — Escape hatches (always T1, evaluated first)
///   P1 — Constraint signals: questions, trivial edits, confirmatory replies
///   P2 — Scored classification: feature verbs, subsystem nouns, cross-cutting
#[allow(dead_code)]
pub struct ClassifierRule {
    pub label: &'static str,
    pub tier: Tier,
    pub precedence: u8,
    pub signals: &'static [&'static str],
    pub matches: fn(&str) -> bool,
}

fn match_escape_hatch(s: &str) -> bool {
    s.is_empty() || s.contains("hex skip plan") || s.contains("hex: skip plan")
}

fn match_question(s: &str) -> bool {
    s.ends_with('?')
        || s.starts_with("what ")
        || s.starts_with("why ")
        || s.starts_with("how ")
        || s.starts_with("when ")
        || s.starts_with("where ")
        || s.starts_with("who ")
        || s.starts_with("can you explain")
        || s.starts_with("explain ")
        || s.starts_with("show me ")
}

fn match_trivial_edit(s: &str) -> bool {
    const TRIVIAL: &[&str] = &[
        "fix typo", "fix a typo", "typo in",
        "rename ", "renaming ",
        "add a comment", "add comment",
        "update comment", "update the comment",
        "docstring", "doc comment",
        "one-line", "one line fix",
        "minor tweak", "small tweak", "tiny change",
        "format ", "reformat ", "run fmt", "run rustfmt",
    ];
    TRIVIAL.iter().any(|p| s.contains(p))
}

fn match_confirmatory(s: &str) -> bool {
    is_confirmatory_response(s)
}

fn score_prompt(s: &str) -> i32 {
    let mut score: i32 = 0;

    const FEATURE_VERBS: &[&str] = &[
        "implement", "build a", "build an", "build the", "build support",
        "add support for", "add a new ", "add an ", "design a", "design an",
        "ship ", "deliver ", "roll out",
    ];
    for v in FEATURE_VERBS {
        if s.contains(v) { score += 2; }
    }

    const WORK_VERBS: &[&str] = &[
        "create", "add ", "fix", "refactor", "update", "change",
        "modify", "write", "generate", "scaffold", "wire ", "connect ",
        "remove", "delete", "migrate", "upgrade", "port ",
    ];
    for v in WORK_VERBS {
        if s.contains(v) { score += 1; }
    }

    const SUBSYSTEM_NOUNS: &[&str] = &[
        "feature", "pipeline", "system", "module", "subsystem",
        "adapter", "port", "endpoint", "api", "service", "dashboard",
        "auth", "oauth", "jwt", "database", "schema", "reducer",
        "migration", "cli command", "mcp tool",
    ];
    for n in SUBSYSTEM_NOUNS {
        if s.contains(n) { score += 2; }
    }

    const CROSS_CUTTING: &[&str] = &[
        "end to end", "end-to-end", "integrate ", "integration ",
        "across ", "cross-cutting", "refactor across",
    ];
    for c in CROSS_CUTTING {
        if s.contains(c) { score += 2; }
    }

    if s.len() < 30 { score -= 1; }
    score
}

fn match_t3_score(s: &str) -> bool { score_prompt(s) >= 4 }
fn match_t2_score(s: &str) -> bool { score_prompt(s) >= 1 }

pub static WORK_INTENT_RULES: &[ClassifierRule] = &[
    // P0 — Escape hatches
    ClassifierRule {
        label: "escape_hatch",
        tier: Tier::T1Todo,
        precedence: 0,
        signals: &["empty input", "hex skip plan", "hex: skip plan"],
        matches: match_escape_hatch,
    },
    // P1 — Constraint signals (override scored classification)
    ClassifierRule {
        label: "question",
        tier: Tier::T1Todo,
        precedence: 1,
        signals: &["trailing ?", "what/why/how/when/where/who", "explain", "show me"],
        matches: match_question,
    },
    ClassifierRule {
        label: "trivial_edit",
        tier: Tier::T1Todo,
        precedence: 1,
        signals: &["fix typo", "rename", "add comment", "docstring", "minor tweak", "run fmt"],
        matches: match_trivial_edit,
    },
    ClassifierRule {
        label: "confirmatory",
        tier: Tier::T1Todo,
        precedence: 1,
        signals: &["yes", "ok", "go ahead", "ship it", "lgtm"],
        matches: match_confirmatory,
    },
    // P2 — Scored classification (feature-sized → T3, work-sized → T2)
    ClassifierRule {
        label: "feature_score",
        tier: Tier::T3Workplan,
        precedence: 2,
        signals: &["implement/build/design +2", "subsystem nouns +2", "cross-cutting +2", "score >= 4"],
        matches: match_t3_score,
    },
    ClassifierRule {
        label: "work_score",
        tier: Tier::T2MiniPlan,
        precedence: 2,
        signals: &["create/add/fix/refactor +1", "work verbs +1", "score >= 1"],
        matches: match_t2_score,
    },
];

pub fn classify_work_intent(prompt: &str) -> Tier {
    let lower = prompt.to_lowercase();
    let trimmed = lower.trim();

    WORK_INTENT_RULES
        .iter()
        .find(|r| (r.matches)(trimmed))
        .map(|r| r.tier)
        .unwrap_or(Tier::T1Todo)
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

// ── Observe (ADR-2026-04-01-2137) ─────────────────────────────────────────────────

/// Non-blocking tool-call observer: reads Claude Code hook JSON from stdin and
/// POSTs it to `/api/events` with a 100 ms timeout (fire-and-forget).
///
/// Invoked as a non-blocking hook:
/// ```json
/// { "PreToolUse":  [{ "type": "command", "command": "hex hook observe-pre",  "blocking": false }] }
/// { "PostToolUse": [{ "type": "command", "command": "hex hook observe-post", "blocking": false }] }
/// ```
async fn observe(event_type: &str) -> Result<()> {
    // Read Claude Code hook JSON from stdin (non-blocking on missing data).
    let stdin = std::io::read_to_string(std::io::stdin()).unwrap_or_default();
    if stdin.trim().is_empty() {
        return Ok(());
    }

    let hook: serde_json::Value = match serde_json::from_str(&stdin) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Malformed stdin — skip silently
    };

    let session_id = std::env::var("CLAUDE_SESSION_ID")
        .or_else(|_| {
            hook.get("session_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or(std::env::VarError::NotPresent)
        })
        .unwrap_or_default();

    if session_id.is_empty() {
        return Ok(()); // Cannot correlate without session_id
    }

    let tool_name = hook.get("tool_name").and_then(|v| v.as_str()).map(|s| s.to_string());

    // input_json: the tool_input field (PreToolUse) or same (PostToolUse)
    let input_json = hook.get("tool_input").map(|v| v.to_string());

    // result_json: tool_response present in PostToolUse
    let result_json = hook.get("tool_response").map(|v| v.to_string());

    // Resolve agent_id from session state file (best-effort)
    let agent_id = SessionState::load().map(|s| s.agent_id).filter(|s| !s.is_empty());

    // The event POST fed the daemon's live dashboard. What this hook still
    // does — extracting ★ Insight blocks — is local and follows.
    let _ = (&session_id, &agent_id, &tool_name, &input_json);

    // ── Post-commit brain validate (fast subset) ────────────────────
    // After a Bash tool call that looks like a git commit, run the two
    // fast self-consistency checks: binary freshness and MCP↔CLI parity.
    // These complete in <2s and catch drift immediately after a commit.
    // ── Insight extraction (ADR-2026-04-14-2345) ─────────────────────────
    // Scan text-carrying fields for `★ Insight` blocks and persist each
    // to ~/.hex/insights/<id>.yaml. Non-blocking: failures log to stderr
    // but never halt the hook or block the turn.
    if event_type == "PostToolUse" || event_type == "Stop" {
        if let Err(e) = (|| -> Result<()> {
            let mut candidate_texts: Vec<String> = Vec::new();
            // tool_response is the primary source on PostToolUse; Stop
            // events may carry assistant text under varied field names so
            // also sweep the whole hook payload as a last resort.
            if let Some(s) = result_json.as_deref() {
                candidate_texts.push(s.to_string());
            }
            if event_type == "Stop" {
                if let Some(s) = hook.get("stop_hook_active").and_then(|v| v.as_str()) {
                    candidate_texts.push(s.to_string());
                }
                // Best-effort: stringify the whole envelope so assistant
                // text carried in any field (e.g. `message`, `content`,
                // `text`) still gets scanned.
                candidate_texts.push(hook.to_string());
            }

            let turn: usize = hook
                .get("turn")
                .and_then(|v| v.as_u64())
                .map(|u| u as usize)
                .unwrap_or(0);

            let insights_dir = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".hex/insights");

            let mut wrote_dir = false;
            for text in &candidate_texts {
                let extracted = crate::commands::insight::extract_insights(
                    text,
                    &session_id,
                    turn,
                );
                for ins in extracted {
                    if !wrote_dir {
                        std::fs::create_dir_all(&insights_dir).ok();
                        wrote_dir = true;
                    }
                    let path = insights_dir.join(format!("{}.yaml", sanitize_id(&ins.id)));
                    match serde_yaml::to_string(&ins) {
                        Ok(yaml) => {
                            if let Err(e) = std::fs::write(&path, yaml) {
                                eprintln!("insight extractor: write {} failed: {}", path.display(), e);
                            } else {
                                eprintln!("insight: extracted {} -> {:?}", ins.id, ins.route_to);
                            }
                        }
                        Err(e) => eprintln!("insight extractor: yaml serialize failed: {}", e),
                    }
                }
            }
            Ok(())
        })() {
            eprintln!("insight extractor error: {}", e);
        }
    }

    if event_type == "PostToolUse" {
        let is_commit = tool_name.as_deref() == Some("Bash")
            && input_json
                .as_deref()
                .map(|s| s.contains("git commit") || s.contains("git merge"))
                .unwrap_or(false);

        if is_commit {
            // Binary freshness — triggers background rebuild if stale
            match check_binary_freshness() {
                FreshnessStatus::Stale { .. } => {
                    eprintln!(
                        "{}",
                        "⬡ brain: binary stale after commit — background rebuild spawned"
                            .yellow()
                    );
                }
                FreshnessStatus::Missing => {
                    eprintln!(
                        "{}",
                        "⬡ brain: release binary missing — run cargo build --release"
                            .yellow()
                    );
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Constrain insight ids to filesystem-safe characters so writing
/// `~/.hex/insights/<id>.yaml` can never escape the insights directory
/// or include directory separators.
fn sanitize_id(id: &str) -> String {
    let trimmed: String = id
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    if trimmed.is_empty() {
        "insight".to_string()
    } else {
        trimmed
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─ ADR-2026-04-11-0227: classify_work_intent tier classifier ─

    #[test]
    fn t1_question_prompts() {
        // Questions are T1 regardless of other signals
        assert_eq!(classify_work_intent("how does the planner work?"), Tier::T1Todo);
        assert_eq!(classify_work_intent("what is HexFlo?"), Tier::T1Todo);
        assert_eq!(classify_work_intent("can you explain the ADR process"), Tier::T1Todo);
        assert_eq!(classify_work_intent("why do workplans need specs first?"), Tier::T1Todo);
        assert_eq!(classify_work_intent("show me the hook router"), Tier::T1Todo);
    }

    #[test]
    fn t1_trivial_edits() {
        // Trivial-edit phrases → T1 regardless of verb weight
        assert_eq!(classify_work_intent("fix typo in README"), Tier::T1Todo);
        assert_eq!(classify_work_intent("rename getCwd to getCurrentWorkingDirectory"), Tier::T1Todo);
        assert_eq!(classify_work_intent("add a comment explaining the regex"), Tier::T1Todo);
        assert_eq!(classify_work_intent("update the docstring on fn foo"), Tier::T1Todo);
        assert_eq!(classify_work_intent("run rustfmt"), Tier::T1Todo);
    }

    #[test]
    fn t1_empty_and_confirmatory() {
        assert_eq!(classify_work_intent(""), Tier::T1Todo);
        assert_eq!(classify_work_intent("   "), Tier::T1Todo);
        assert_eq!(classify_work_intent("yes"), Tier::T1Todo);
        assert_eq!(classify_work_intent("go ahead"), Tier::T1Todo);
        assert_eq!(classify_work_intent("lgtm"), Tier::T1Todo);
    }

    #[test]
    fn t1_opt_out_phrase() {
        // "hex skip plan" escape hatch always downgrades to T1
        assert_eq!(
            classify_work_intent("implement oauth login hex skip plan"),
            Tier::T1Todo
        );
        assert_eq!(
            classify_work_intent("hex skip plan: build the whole pipeline"),
            Tier::T1Todo
        );
    }

    #[test]
    fn t2_small_adapter_work() {
        // Work-sized but scoped to one adapter — should be T2, not T3.
        assert_eq!(
            classify_work_intent("add a helper function to trim whitespace"),
            Tier::T2MiniPlan
        );
        assert_eq!(
            classify_work_intent("refactor the retry loop in fs_adapter"),
            Tier::T2MiniPlan
        );
        assert_eq!(
            classify_work_intent("update the timeout in the llm adapter"),
            Tier::T2MiniPlan
        );
    }

    #[test]
    fn t3_feature_sized_cross_adapter() {
        // Feature-sized verbs + subsystem nouns → T3
        assert_eq!(
            classify_work_intent("implement OAuth login with refresh tokens"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent("add support for a new mcp tool that dispatches swarms"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent("build an end-to-end audit pipeline across all adapters"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent("design a new authentication subsystem with JWT"),
            Tier::T3Workplan
        );
        assert_eq!(
            classify_work_intent(
                "implement a new feature for spacetimedb module deployment via the CLI"
            ),
            Tier::T3Workplan
        );
    }

    #[test]
    fn t1_regression_false_positives() {
        // P10: regression suite — these should NEVER return T3.
        // These are the kinds of prompts that LOOK like work but are actually
        // small changes, read-only questions, or conversational.
        let t1_prompts = [
            "what files does the route hook read?",
            "how does classify_prompt differ from classify_work_intent?",
            "show me the SessionState struct",
            "explain the three-tier sizing",
            "fix typo in the ADR header",
            "rename the Tier variants to Todo/MiniPlan/Workplan",
            "add a comment to the match arm",
            "reformat the tests module with rustfmt",
            "why is the threshold at score >= 4?",
            "can you explain how the scoring works?",
            "update the docstring on classify_work_intent",
            "yes",
            "lgtm",
            "go ahead",
            "hex skip plan: implement oauth",
            "?",
            "",
            "",
            "run rustfmt",
            "one-line fix to the regex",
        ];
        for prompt in t1_prompts {
            let tier = classify_work_intent(prompt);
            assert_ne!(
                tier,
                Tier::T3Workplan,
                "prompt '{}' should NOT be T3 (got {:?})",
                prompt,
                tier
            );
        }
    }

    #[test]
    fn cross_tier_regression_trivial_verb_with_t3_nouns() {
        // Cross-tier regression: prompts with T3-triggering nouns (feature,
        // subsystem, pipeline) must stay T1 when the verb is trivial.
        // These would score ≥4 if the TRIVIAL early-return didn't fire.

        // "rename" is trivial even when the target is a feature-level concept
        assert_eq!(
            classify_work_intent("rename the variable that implements the OAuth feature"),
            Tier::T1Todo
        );

        // "add a comment" is trivial even inside a cross-adapter subsystem
        assert_eq!(
            classify_work_intent("add a comment explaining the migration pipeline endpoint"),
            Tier::T1Todo
        );

        // "fix typo" is trivial even when the file lives in a subsystem module
        assert_eq!(
            classify_work_intent("fix typo in the authentication subsystem adapter module"),
            Tier::T1Todo
        );
    }

    #[test]
    fn test_rule_table_invariants() {
        // Structural invariant: the TRIVIAL early-return MUST fire before
        // score-based classification. If the ordering flips, prompts that
        // combine a trivial marker with subsystem nouns false-positive as T3,
        // spawning unwanted workplan drafts.
        //
        // Each tuple: (prompt with trivial marker + T3 nouns, which marker wins)
        let trivial_beats_feature = [
            ("fix typo in the feature pipeline", "fix typo"),
            ("rename the adapter port to something clearer", "rename "),
            ("add a comment explaining the auth subsystem", "add a comment"),
            ("update the docstring on the api endpoint handler", "docstring"),
            ("reformat the migration module", "reformat "),
            ("run rustfmt on the mcp tool adapter", "run rustfmt"),
            ("fix a typo in the database schema docs", "fix a typo"),
            ("minor tweak to the jwt service port", "minor tweak"),
        ];
        for (prompt, trivial_marker) in trivial_beats_feature {
            assert_eq!(
                classify_work_intent(prompt),
                Tier::T1Todo,
                "trivial marker '{}' must precede feature-scoring in: '{}'",
                trivial_marker,
                prompt
            );
        }

        // Positive control: without the trivial marker, the same nouns reach T3.
        assert_eq!(
            classify_work_intent("implement the auth subsystem with jwt"),
            Tier::T3Workplan,
            "positive control: subsystem nouns without trivial marker → T3"
        );
        assert_eq!(
            classify_work_intent("build a new feature for the database migration pipeline"),
            Tier::T3Workplan,
            "positive control: feature + pipeline + database without trivial → T3"
        );
    }

    // ─ Existing helpers (sanity checks) ─

    #[test]
    fn classify_prompt_hints() {
        // Smoke test for the existing hint function.
        assert!(!classify_prompt("please scaffold a new hex project").is_empty());
        assert!(classify_prompt("just a random question").is_empty());
    }

    #[test]
    fn confirmatory_response_matching() {
        assert!(is_confirmatory_response("yes"));
        assert!(is_confirmatory_response("go ahead"));
        assert!(is_confirmatory_response("lgtm"));
        assert!(!is_confirmatory_response("yes but make it async"));
        assert!(!is_confirmatory_response("this is a much longer response that is not a confirmation"));
    }
}
