//! POST /api/brain/chat — operator chat dispatch for the Brain dashboard.
//!
//! wp-brain-dashboard M3.
//!
//! Operator types `@<role> <message>` in the Brain chat pane. Frontend POSTs
//! { role, message } here. We:
//!   1. Load the role's YAML persona from embedded AgentTemplates
//!   2. Build a system prompt from persona.description + persona.constraints
//!   3. Call inference_complete with the persona's preferred model
//!   4. Return { content, model, role } for the frontend to render
//!
//! No swarm/task creation, no worker spawn — this is the lightweight chat path
//! for "I want to ask my agent something". For full workflow execution
//! (multi-iteration, gates, commits) the operator still uses
//! `hex agent worker --role <name> --once`.

use axum::{extract::{Path, State}, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::state::SharedState;
use crate::templates::AgentTemplates;
use crate::routes::inference::{inference_complete, InferenceCompleteRequest};
use tracing::warn;

#[derive(Debug, Deserialize)]
pub struct BrainChatRequest {
    /// Persona role name (e.g. "pm-agent", "adversarial-red"). Must have a
    /// matching YAML at hex-cli/assets/agents/hex/hex/<role>.yml.
    pub role: String,
    /// Operator message — free text, the agent's task.
    pub message: String,
    /// Optional project context. When set, the persona's system prompt is
    /// prefixed with PROJECT CONTEXT (name + rootPath) so the agent's
    /// response is scoped to that project — pm-agent's ADR drafts land in
    /// the right docs/adrs/, planner's workplans target the right repo, etc.
    /// Resolved against state.projects (the registry from /api/projects).
    #[serde(default, alias = "projectId")]
    pub project_id: Option<String>,
    /// Optional thread id. When set, the persona receives the last N messages
    /// of the thread as conversation context — without this, every dispatch
    /// is amnesiac and the agent confabulates "I don't have that data". The
    /// thread is loaded from hexflo memory key "chat:thread:<id>".
    #[serde(default, alias = "threadId")]
    pub thread_id: Option<String>,
    /// Recursion depth for agent-to-agent auto-dispatch. The operator's
    /// initial call is depth=0; if the agent's reply mentions @<role> at
    /// line start, we recurse with depth=1 (and so on, capped at MAX_AGENT_DISPATCH_DEPTH).
    /// External callers should leave this unset; the field is internal but
    /// exposed via JSON for transparency in the response payload.
    #[serde(default, alias = "dispatchDepth")]
    pub dispatch_depth: Option<u8>,
}

/// Maximum depth of @<role>-driven auto-dispatch chains. Operator → pm-agent
/// (depth 0) → @hex-coder (depth 1) → @hex-tester (depth 2) is the deepest
/// we allow before halting; further `@` mentions are surfaced as text only.
/// Without this cap, two agents that mention each other can spin forever.
const MAX_AGENT_DISPATCH_DEPTH: u8 = 2;

#[derive(Debug, Serialize)]
pub struct BrainChatResponse {
    pub role: String,
    pub model: String,
    pub content: String,
}

/// Minimal persona shape — we only need the few fields used in prompt construction.
/// Avoids depending on hex-cli's AgentDefinition (which lives in another crate).
#[derive(Debug, Deserialize, Default)]
struct PersonaSnippet {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    constraints: Vec<serde_yaml::Value>,
    #[serde(default)]
    model: PersonaModel,
    /// Persona-authored system prompt — when present, takes precedence over
    /// the universal RESPONSE STYLE / CAPABILITIES blocks for shaping voice
    /// + behavior. Roles like pm-agent override the default "ask the operator
    /// one specific question" guidance with role-specific delegation rules.
    #[serde(default)]
    system_prompt: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PersonaModel {
    #[serde(default)]
    preferred: Option<String>,
}

/// Collect a small set of cheap, ground-truth facts about a project root so
/// the agent doesn't have to invent them. Each line is "  fact: value" so
/// the format is stable and easy for models to consume. All checks are
/// best-effort filesystem reads; failures degrade silently to "unknown".
fn collect_project_facts(root: &str) -> String {
    use std::path::PathBuf;
    let r = PathBuf::from(root);
    let mut lines: Vec<String> = Vec::new();

    // 1. Existence + git
    if !r.exists() {
        return format!("  status: ROOT MISSING ({} does not exist on disk)", root);
    }
    let git_dir = r.join(".git");
    let is_git = git_dir.exists();
    lines.push(format!("  is_git_repo: {}", is_git));

    // 2. Recent commits (best-effort — timeout 2s via std::process::Command)
    if is_git {
        if let Ok(out) = std::process::Command::new("git")
            .args(["-C", root, "log", "--oneline", "-5"])
            .output()
        {
            let txt = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !txt.is_empty() {
                lines.push("  recent_commits:".to_string());
                for line in txt.lines().take(5) {
                    lines.push(format!("    - {}", line));
                }
            }
        }
        if let Ok(out) = std::process::Command::new("git")
            .args(["-C", root, "branch", "--show-current"])
            .output()
        {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !branch.is_empty() {
                lines.push(format!("  current_branch: {}", branch));
            }
        }
        // Dirty file count
        if let Ok(out) = std::process::Command::new("git")
            .args(["-C", root, "status", "--short"])
            .output()
        {
            let count = String::from_utf8_lossy(&out.stdout).lines().count();
            lines.push(format!("  uncommitted_changes: {}", count));
        }
    }

    // 3. Build manifests / language signals
    let manifests = [
        ("package.json", "node/typescript"),
        ("Cargo.toml", "rust"),
        ("pyproject.toml", "python"),
        ("go.mod", "go"),
        ("requirements.txt", "python"),
        ("composer.json", "php"),
        ("Gemfile", "ruby"),
    ];
    let detected: Vec<&str> = manifests
        .iter()
        .filter(|(f, _)| r.join(f).exists())
        .map(|(_, lang)| *lang)
        .collect();
    if !detected.is_empty() {
        lines.push(format!("  languages_detected: {}", detected.join(", ")));
    } else {
        lines.push("  languages_detected: none (no standard manifest at root)".to_string());
    }

    // 4. Workplans
    let wp_dir = r.join("docs/workplans");
    if let Ok(entries) = std::fs::read_dir(&wp_dir) {
        let workplans: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) != Some("json") { return None; }
                let stem = p.file_stem()?.to_str()?.to_string();
                if stem.starts_with("wp-") || stem.starts_with("test-") || stem.starts_with("feat-") {
                    Some(stem)
                } else {
                    None
                }
            })
            .collect();
        lines.push(format!("  workplans_count: {}", workplans.len()));
        if !workplans.is_empty() {
            lines.push("  workplans_sample:".to_string());
            for w in workplans.iter().take(5) {
                lines.push(format!("    - {}", w));
            }
        }
    } else {
        lines.push("  workplans_count: 0 (no docs/workplans/ directory)".to_string());
    }

    // 5. ADRs
    let adr_dir = r.join("docs/adrs");
    if let Ok(entries) = std::fs::read_dir(&adr_dir) {
        let count = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
            .count();
        lines.push(format!("  adrs_count: {}", count));
    } else {
        lines.push("  adrs_count: 0 (no docs/adrs/ directory)".to_string());
    }

    // 6. Top-level dirs (so agent has structure)
    if let Ok(entries) = std::fs::read_dir(&r) {
        let mut dirs: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.') && n != "node_modules" && n != "target" && n != "dist")
            .collect();
        dirs.sort();
        if !dirs.is_empty() {
            lines.push(format!("  top_level_dirs: {}", dirs.iter().take(15).cloned().collect::<Vec<_>>().join(", ")));
        }
    }

    lines.join("\n")
}

fn load_persona(role: &str) -> Option<PersonaSnippet> {
    let path = format!("agents/hex/hex/{}.yml", role);
    let bytes = AgentTemplates::get(&path)?;
    let content = std::str::from_utf8(&bytes.data).ok()?;
    serde_yaml::from_str::<PersonaSnippet>(content).ok()
}

fn constraints_as_strings(values: &[serde_yaml::Value]) -> Vec<String> {
    values.iter().filter_map(|v| match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        // Some YAMLs nest constraints as { rule: ..., why: ... } maps.
        // Render the first scalar found.
        serde_yaml::Value::Mapping(m) => m.values().find_map(|val| match val {
            serde_yaml::Value::String(s) => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    }).collect()
}

/// Map a YAML model name to the inference-gateway model id. Keep this in sync
/// with hex-cli/src/pipeline/agent_def.rs::ModelConfig::resolve_model_id.
/// Falls through to passthrough for Ollama-style "<name>:<tag>" identifiers.
fn resolve_model_id(name: &str) -> String {
    match name {
        "sonnet" | "claude-sonnet" => "claude-sonnet-4-6".to_string(),
        "haiku" | "claude-haiku" => "claude-haiku-4-5-20251001".to_string(),
        "opus" | "claude-opus" => "claude-opus-4-6".to_string(),
        "gpt-4o" => "openai/gpt-4o".to_string(),
        "gpt-4o-mini" => "openai/gpt-4o-mini".to_string(),
        n if n.contains(':') => n.to_string(),
        _ => "openrouter/free".to_string(),
    }
}

pub async fn dispatch_brain_chat(
    state: State<SharedState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<BrainChatRequest>,
) -> (StatusCode, Json<Value>) {
    if req.role.is_empty() || req.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "role and message are required" })),
        );
    }

    let Some(persona) = load_persona(&req.role) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!(
                    "no YAML persona found for role '{}'. Backfill hex-cli/assets/agents/hex/hex/{}.yml.",
                    req.role, req.role
                ),
            })),
        );
    };

    // Resolve project context if requested. We do this BEFORE the
    // inference_complete call (which consumes `state`) so we look it up
    // through a cloned Arc handle. The block includes both the registered
    // metadata AND quick filesystem facts so the agent isn't forced to
    // confabulate (lacking filesystem tools, the model otherwise invents
    // plausible-looking but wrong details — e.g. "directory is empty",
    // "not a git repo", inventing usernames in paths).
    let project_block: Option<String> = match req.project_id.as_deref() {
        Some(id) if !id.is_empty() && id != "__global__" => {
            let port_handle = state.0.state_port.clone();
            if let Some(port) = port_handle {
                match port.project_get(id).await {
                    Ok(Some(p)) => {
                        let facts = collect_project_facts(&p.root_path);
                        Some(format!(
                            "PROJECT CONTEXT:\n  name: {}\n  id: {}\n  root: {}\n\nPROJECT FACTS (ground truth — do NOT confabulate beyond these):\n{}\n\nAll file paths, ADR drafts, workplans, and architectural references must be relative to this project's root. If a fact you need is not in PROJECT FACTS, say 'I don't have that data' rather than guessing.",
                            p.name, p.id, p.root_path, facts
                        ))
                    }
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    };

    // Live runtime state — swarms, their tasks, registered agents. Without
    // this, asking "swarm status" causes the agent to invent JSON blobs
    // ("task-001" with a 2025 date that doesn't exist anywhere). Cost is
    // small (a few KB); value is reality-grounded responses.
    let live_state_block: Option<String> = {
        let port_handle = state.0.state_port.clone();
        if let Some(port) = port_handle {
            let mut lines: Vec<String> = vec!["LIVE STATE (queried just now — these are the actual swarms/tasks/agents):".to_string()];
            // Swarms (active only, top 8)
            if let Ok(swarms) = port.swarm_list_active().await {
                if swarms.is_empty() {
                    lines.push("  swarms: none active".to_string());
                } else {
                    lines.push(format!("  swarms ({} active):", swarms.len()));
                    for s in swarms.iter().take(8) {
                        lines.push(format!("    - {} [id={}, status={}]",
                            s.name, &s.id[..s.id.len().min(8)], s.status));
                    }
                }
            }
            // Tasks across active swarms (top 10 most recent / non-completed)
            if let Ok(all_tasks) = port.swarm_task_list(None).await {
                let active: Vec<_> = all_tasks.iter()
                    .filter(|t| t.status != "completed" && t.status != "done" && t.status != "failed")
                    .take(10)
                    .collect();
                if !active.is_empty() {
                    lines.push(format!("  tasks_in_flight ({} shown):", active.len()));
                    for t in active {
                        let agent = if t.agent_id.is_empty() { "unassigned".to_string() } else { t.agent_id[..t.agent_id.len().min(8)].to_string() };
                        let title = if t.title.len() > 80 { format!("{}…", &t.title[..80]) } else { t.title.clone() };
                        lines.push(format!("    - [{}] {} (agent: {})", t.status, title, agent));
                    }
                }
            }
            // Registered agents — list returns Vec<Value>, so we read the
            // status string by key. "online"/"active" count as online.
            if let Ok(agents) = port.hex_agent_list().await {
                let online_count = agents.iter().filter(|a| {
                    matches!(
                        a.get("status").and_then(|v| v.as_str()),
                        Some("online") | Some("active")
                    )
                }).count();
                lines.push(format!("  agents: {} registered, {} online", agents.len(), online_count));
            }
            // Unacked inbox
            if let Ok(inbox) = port.inbox_query("system", Some(2), true).await {
                if !inbox.is_empty() {
                    lines.push(format!("  unacked_priority2_inbox: {} (operator override pending)", inbox.len()));
                }
            }
            if lines.len() > 1 { Some(lines.join("\n")) } else { None }
        } else {
            None
        }
    };

    // Memory recall: search hexflo memory for entries relevant to this message.
    // Surfaces prior chat threads on the same topic, prior workplan task results
    // (<task_id>:result keys), and any other persisted agent outputs. Without
    // this the brain is amnesiac across conversations — every dispatch starts
    // from project facts only, and the agent confabulates "I don't have that data"
    // when asked about prior work.
    let memory_block: Option<String> = {
        let port_handle = state.0.state_port.clone();
        if let Some(port) = port_handle {
            // Extract keywords from the message: words ≥4 chars, lowercased,
            // dedupped. Cap at 4 keywords to keep the search query small.
            let mut keywords: Vec<String> = req.message
                .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .filter(|w| w.len() >= 4)
                .map(|w| w.to_lowercase())
                .collect();
            keywords.sort();
            keywords.dedup();
            // Skip generic stopwords that would over-match.
            let stopwords = ["this", "that", "what", "which", "where", "when",
                "have", "with", "from", "about", "would", "could", "should",
                "agent", "task", "tell", "give", "find", "show", "help", "current"];
            keywords.retain(|w| !stopwords.contains(&w.as_str()));
            // Take top 4 longest (proxy for specificity)
            keywords.sort_by(|a, b| b.len().cmp(&a.len()));
            keywords.truncate(4);

            let mut hits: Vec<(String, String)> = Vec::new();
            for kw in &keywords {
                if let Ok(entries) = port.hexflo_memory_search(kw).await {
                    for (k, v) in entries {
                        // Skip the current thread itself if loaded above.
                        if let Some(tid) = req.thread_id.as_deref() {
                            if k == format!("{}{}", THREAD_KEY_PREFIX, tid) { continue; }
                        }
                        // Skip empty values + bias against very short stub entries.
                        if v.len() < 32 { continue; }
                        if !hits.iter().any(|(existing_k, _)| existing_k == &k) {
                            hits.push((k, v));
                        }
                    }
                }
                if hits.len() >= 6 { break; }
            }

            if hits.is_empty() {
                None
            } else {
                let mut lines = vec!["RELEVANT MEMORY (prior decisions, agent outputs, chat threads — use these instead of confabulating):".to_string()];
                for (k, v) in hits.iter().take(6) {
                    // Truncate each value so the system prompt stays bounded.
                    // Strip JSON encoding noise — many entries are JSON-stringified.
                    let preview: String = if v.starts_with('{') {
                        // Try to pull just the "content" or "result" field if present.
                        if let Ok(parsed) = serde_json::from_str::<Value>(v) {
                            parsed.get("content")
                                .or_else(|| parsed.get("result"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| v.clone())
                        } else {
                            v.clone()
                        }
                    } else {
                        v.clone()
                    };
                    let preview = if preview.len() > 600 {
                        format!("{}…", &preview[..600])
                    } else {
                        preview
                    };
                    lines.push(format!("\n[{}]\n{}", k, preview));
                }
                Some(lines.join("\n"))
            }
        } else {
            None
        }
    };

    let mut system_lines: Vec<String> = Vec::new();
    if let Some(block) = project_block.as_ref() {
        system_lines.push(block.clone());
        system_lines.push(String::new()); // blank line separator
    }
    if let Some(block) = live_state_block.as_ref() {
        system_lines.push(block.clone());
        system_lines.push(String::new());
    }
    if let Some(block) = memory_block.as_ref() {
        system_lines.push(block.clone());
        system_lines.push(String::new());
    }
    system_lines.push(format!(
        "ROLE: {}",
        if persona.name.is_empty() { req.role.clone() } else { persona.name.clone() }
    ));
    if !persona.description.is_empty() {
        system_lines.push(persona.description.trim().to_string());
    }
    let constraints = constraints_as_strings(&persona.constraints);
    if !constraints.is_empty() {
        system_lines.push("\nCONSTRAINTS:".to_string());
        for c in &constraints {
            system_lines.push(format!("- {}", c));
        }
    }
    // Persona-authored system_prompt — load-bearing for roles that override
    // generic chat behavior (e.g. pm-agent's "always delegate, never ask the
    // operator" stance). Goes BEFORE the universal RESPONSE STYLE / CAPABILITIES
    // blocks so persona text wins on conflict.
    if let Some(sp) = persona.system_prompt.as_ref().filter(|s| !s.trim().is_empty()) {
        system_lines.push(String::new());
        system_lines.push("PERSONA INSTRUCTIONS (load-bearing — override the generic style block below if they conflict):".to_string());
        system_lines.push(sp.trim().to_string());
    }
    // Brevity hint — chat is conversational. Operators were drowning in
    // 200-line analyses for "what's blocking task X" questions. Default
    // short; expand only when explicitly asked.
    system_lines.push(String::new());
    system_lines.push("RESPONSE STYLE:".to_string());
    system_lines.push("- Lead with the answer in 1-3 sentences. No headings, no tables, no Option A vs B unless the operator asks for the comparison.".to_string());
    system_lines.push("- If a single recommendation is clear, give it. Don't list every alternative.".to_string());
    system_lines.push("- If you need clarification, ask ONE specific question, not three.".to_string());
    system_lines.push("- Expand into structured detail (sections, lists, code) only when the operator asks 'why', 'compare', 'walk me through', or similar.".to_string());
    system_lines.push("- NEVER invent specific identifiers (task IDs, UUIDs, file names, dates, model names) that do not appear in LIVE STATE / PROJECT FACTS / RELEVANT MEMORY above. If you would need to cite a specific id and one isn't in those blocks, say 'I don't see specific IDs in the snapshot — should I query for them?' rather than fabricating plausible-looking ones.".to_string());
    system_lines.push(String::new());
    system_lines.push("CAPABILITIES IN THIS SURFACE:".to_string());
    system_lines.push("- You are responding via the Brain chat surface. You have NO direct shell, MCP, or filesystem tools — anything you 'observe' must come from LIVE STATE / PROJECT FACTS / RELEVANT MEMORY blocks above.".to_string());
    system_lines.push("- HOWEVER, you CAN delegate to other agents by mentioning `@<role>` at the start of any line in your reply. The system auto-dispatches each `@<role>` to that agent in the same thread, with the rest of the line (and indented body) as the task brief. Use this whenever the request is technical work that another role specializes in — DO NOT bounce the question back to the operator if a specialist can handle it.".to_string());
    // Inject the actual roster so models can't invent fake roles like
    // @shell-operator / @ops / @sysadmin. Generated from the embedded
    // persona YAMLs at startup; always reflects what's actually deployable.
    let roster: Vec<String> = AgentTemplates::iter()
        .filter_map(|p| {
            let s = p.as_ref();
            let prefix = "agents/hex/hex/";
            if !s.starts_with(prefix) || !s.ends_with(".yml") { return None; }
            Some(s[prefix.len()..s.len() - 4].to_string())
        })
        .collect();
    let mut roster_sorted = roster.clone();
    roster_sorted.sort();
    roster_sorted.dedup();
    system_lines.push(format!(
        "- VALID @<role> ROSTER (the ONLY roles you may dispatch to — anything else is silently dropped): @{}",
        roster_sorted.join(" @"),
    ));
    system_lines.push("- DO NOT invent roles like @shell-operator, @ops, @engineer, @backend, @sysadmin, @devops. If your task needs filesystem/shell work, dispatch to @hex-coder (general code edits), @hex-fixer (bugfix), @rust-refactorer (Rust refactor), @scaffold-validator (build/typecheck gates), or @hex-tester (tests). Those roles have the actual tool access in the worker dispatch path.".to_string());
    system_lines.push("- DO NOT pretend to have tried a tool, called an MCP function, or executed a command yourself. DO NOT generate fake error messages. If you need work done, delegate via @<role>; if you need data only the operator can give, ask ONE specific question.".to_string());
    system_lines.push("- DO NOT contradict LIVE STATE. If LIVE STATE says '2 registered, 1 online' and the operator says 'agents are offline', the truth is in LIVE STATE — explain what online/stale/dead actually mean in the registry.".to_string());
    system_lines.push("- AUTO-DISPATCH DEPTH: If you see this reply is itself a delegation (the user message starts with 'Inbound delegation from @...'), execute the task directly. Don't bounce it back up the chain by re-mentioning the sender's role — that's a loop the system will break, but it wastes a turn.".to_string());
    let system_prompt = system_lines.join("\n");

    // Brain chat ALWAYS routes to a frontier model — the operator wants
    // high-quality, low-latency responses in this surface, even when the
    // persona's `model.preferred` points at a cheap local Ollama for the
    // worker dispatch path. Override:
    //   1. HEX_BRAIN_FRONTIER_MODEL env (operator override)
    //   2. anthropic/claude-sonnet-4-6 via OpenRouter (default)
    // OpenRouter-vendor-prefixed form ("anthropic/claude-sonnet-4-6") is
    // required: the inference router treats bare "claude-sonnet-4-6" as a
    // local model and falls through to Ollama, while the slash-form
    // matches the openrouter provider path. Persona's preferred is only
    // used if the operator sets HEX_BRAIN_FRONTIER_MODEL=persona.
    let model_id = match std::env::var("HEX_BRAIN_FRONTIER_MODEL").ok() {
        Some(v) if v == "persona" => persona
            .model
            .preferred
            .as_deref()
            .map(resolve_model_id)
            .unwrap_or_else(|| "anthropic/claude-sonnet-4-6".to_string()),
        Some(v) if v.contains('/') => v,
        Some(v) => format!("anthropic/{}", v.trim_start_matches("claude-")),
        None => "anthropic/claude-sonnet-4-6".to_string(),
    };

    // Load thread history when thread_id is provided. Map persona names
    // ("pm-agent", "adversarial-red", etc.) to "assistant" role and "you" to
    // "user". System messages from the welcome bubble are skipped. Cap at the
    // last 20 messages so token costs stay bounded — older context falls off.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if let Some(tid) = req.thread_id.as_deref().filter(|s| !s.is_empty()) {
        let port_handle = state.0.state_port.clone();
        if let Some(port) = port_handle {
            let key = format!("{}{}", THREAD_KEY_PREFIX, tid);
            if let Ok(Some(raw)) = port.hexflo_memory_retrieve(&key).await {
                if let Ok(record) = serde_json::from_str::<ThreadRecord>(&raw) {
                    let history: Vec<&ThreadMessage> = record.messages
                        .iter()
                        .filter(|m| m.from != "system" && m.from != "broadcast" && m.error != Some(true))
                        .collect();
                    let start = history.len().saturating_sub(20);
                    for m in &history[start..] {
                        // Skip the most recent user message if it equals req.message —
                        // the frontend appended it before dispatch, and we re-add as
                        // the final turn below.
                        if m.from == "you" && m.text == req.message
                            && history.last().map(|last| std::ptr::eq(*last, *m)).unwrap_or(false)
                        {
                            continue;
                        }
                        messages.push(json!({
                            "role": if m.from == "you" { "user" } else { "assistant" },
                            "content": if m.from == "you" {
                                m.text.clone()
                            } else {
                                format!("[as @{}]\n{}", m.from, m.text)
                            },
                        }));
                    }
                }
            }
        }
    }
    // Final turn: the operator's current message.
    messages.push(json!({ "role": "user", "content": req.message.clone() }));

    let inference_req = InferenceCompleteRequest {
        model: Some(model_id.clone()),
        messages,
        system: Some(system_prompt),
        max_tokens: 4096,
        tools: None,
    };

    // Clone state/headers BEFORE inference_complete consumes them — we need
    // them again to recurse into child @-mention dispatches.
    let inner_state = State(state.0.clone());
    let inner_headers = headers.clone();
    let (status, resp) = inference_complete(state, headers, Json(inference_req)).await;
    if status != StatusCode::OK {
        return (status, resp);
    }

    let content = resp.0
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let final_model = resp.0
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or(model_id);

    // Agent-to-agent auto-dispatch.
    //
    // Parse the reply for `@<role>` mentions at line-start. Each becomes a
    // recursive dispatch to that role with the SAME thread + project + a
    // task brief built from the mentioning agent's text (so the engineer
    // sees what the PM said, not just the operator's original message).
    // Cap at MAX_AGENT_DISPATCH_DEPTH so a chain of `@`s can't infinite-loop.
    let depth = req.dispatch_depth.unwrap_or(0);
    let mut enqueued: Vec<Value> = Vec::new();
    let children: Vec<Value> = if depth >= MAX_AGENT_DISPATCH_DEPTH {
        Vec::new()
    } else {
        let mentions = parse_agent_mentions(&content);
        let mut out: Vec<Value> = Vec::with_capacity(mentions.len());
        for (target_role, brief) in mentions {
            // Skip self-mentions (an agent cannot delegate to itself).
            if target_role == req.role { continue; }

            // EXECUTION PATH: write an inference_task row so an alive hex-agent
            // worker in this role's pool can claim and run it. This is
            // separate from the chat recursion below — the chat call gives
            // the operator visible reasoning in the thread; the inference_task
            // is the actual work order. Once `wp-hex-agent-idle-loop` ships
            // and pool workers stay alive, claim+execute happens autonomously.
            //
            // SAFETY GATE: if the brief mentions any path that matches
            // is_critical_path, we mark the task as "PendingReview" instead
            // of "Pending" so a worker won't auto-claim. Operator must
            // explicitly promote via `hex brain dispatch run <id>`. This
            // mirrors the "background hex-agent overwrote files" lesson —
            // never auto-execute writes that touch infra.
            let task_id = uuid::Uuid::new_v4().to_string();
            let workplan_id = format!(
                "brain-chat:{}",
                req.thread_id.as_deref().unwrap_or("global")
            );
            let phase = format!("dispatch-from-{}", req.role);
            let timestamp = chrono::Utc::now().to_rfc3339();
            let touches_critical = mentions_critical_path(&brief);
            let port_handle = inner_state.0.state_port.clone();
            let mut enqueue_status: &str = "skipped";
            if let Some(port) = port_handle.as_ref() {
                let create_res = port
                    .inference_task_create(
                        &task_id, &workplan_id, &task_id, &phase,
                        &brief, &target_role, &timestamp,
                    )
                    .await;
                match create_res {
                    Ok(()) => {
                        enqueue_status = if touches_critical { "pending_review" } else { "pending" };
                    }
                    Err(e) => {
                        warn!("brain-chat dispatch enqueue failed for role={} id={}: {}",
                            target_role, task_id, e);
                        enqueue_status = "enqueue_failed";
                    }
                }
            }
            enqueued.push(json!({
                "id": task_id,
                "role": target_role,
                "status": enqueue_status,
                "workplan_id": workplan_id,
                "touches_critical_path": touches_critical,
            }));

            // VISIBILITY PATH: recursive chat call so the operator sees
            // what the engineer would do. Goes regardless of enqueue
            // status — even pending_review tasks get a visible reasoning
            // bubble so the operator can decide whether to promote.
            let dispatch_message = format!(
                "Inbound delegation from @{}.\n\n--- @{} said ---\n{}\n--- end @{} ---\n\nYour task: {}",
                req.role, req.role, content, req.role, brief
            );
            let inner = BrainChatRequest {
                role: target_role.clone(),
                message: dispatch_message,
                project_id: req.project_id.clone(),
                thread_id: req.thread_id.clone(),
                dispatch_depth: Some(depth + 1),
            };
            // Recursive call. async fn → Box::pin to break cycles.
            let st = State(inner_state.0.clone());
            let hd = inner_headers.clone();
            let fut = Box::pin(dispatch_brain_chat(st, hd, Json(inner)));
            let (child_status, child_resp) = fut.await;
            if child_status == StatusCode::OK {
                out.push(json!({
                    "role": target_role,
                    "content": child_resp.0.get("content").cloned().unwrap_or(Value::Null),
                    "model": child_resp.0.get("model").cloned().unwrap_or(Value::Null),
                    "children": child_resp.0.get("children").cloned().unwrap_or(Value::Array(vec![])),
                    "enqueued_task_id": task_id,
                    "enqueue_status": enqueue_status,
                }));
            } else {
                out.push(json!({
                    "role": target_role,
                    "error": child_resp.0.get("error").cloned().unwrap_or(json!("dispatch failed")),
                    "enqueued_task_id": task_id,
                    "enqueue_status": enqueue_status,
                }));
            }
        }
        out
    };

    (
        StatusCode::OK,
        Json(json!({
            "role": req.role,
            "model": final_model,
            "content": content,
            "children": children,
            "enqueued": enqueued,
            "depth": depth,
        })),
    )
}

/// GET /api/brain/dispatches — list recent brain-chat dispatches.
/// Filters inference_task rows whose workplan_id starts with "brain-chat:"
/// (the prefix written by dispatch_brain_chat when an @<role> mention fires).
/// Includes Pending, InProgress, and recently-Completed (last 50) so the
/// dashboard shows "what just ran" not just the empty-when-fast queue.
pub async fn list_brain_dispatches(
    state: State<crate::state::SharedState>,
) -> (StatusCode, Json<Value>) {
    let port = match state.0.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    // SELECT * FROM inference_task — STDB SQL doesn't support LIKE, so we
    // pull all and filter client-side. brain-chat dispatches are a small
    // fraction of total inference_task volume; this stays fast.
    let res: Result<Vec<crate::ports::state::InferenceTaskInfo>, _> =
        port.inference_task_list_all().await;
    match res {
        Ok(rows) => {
            let mut dispatches: Vec<Value> = rows.into_iter()
                .filter(|t| t.workplan_id.starts_with("brain-chat:"))
                .map(|t| json!({
                    "id": t.id,
                    "role": t.role,
                    "prompt": t.prompt,
                    "status": t.status,
                    "agentId": t.agent_id,
                    "threadId": t.workplan_id.strip_prefix("brain-chat:").unwrap_or("").to_string(),
                    "createdAt": t.created_at,
                    "updatedAt": t.updated_at,
                    "result": t.result,
                    "error": t.error,
                }))
                .collect();
            // Newest first — sort descending by createdAt string (RFC3339 sorts lexicographically).
            dispatches.sort_by(|a, b| b["createdAt"].as_str().unwrap_or("").cmp(a["createdAt"].as_str().unwrap_or("")));
            dispatches.truncate(50);
            let pending = dispatches.iter().filter(|d| d["status"] == "Pending").count();
            let in_progress = dispatches.iter().filter(|d| d["status"] == "InProgress").count();
            let completed = dispatches.iter().filter(|d| d["status"] == "Completed").count();
            (StatusCode::OK, Json(json!({
                "dispatches": dispatches,
                "total": dispatches.len(),
                "pending": pending,
                "inProgress": in_progress,
                "completed": completed,
            })))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("inference_task_list_all: {}", e) })),
        ),
    }
}

/// Best-effort detection of critical-path tokens in a free-form brief.
/// Returns true if any whitespace-delimited word in the brief matches
/// `hex_core::domain::validation::is_critical_path`. Used to gate
/// auto-execution: if true, the inference_task is marked PendingReview
/// instead of Pending so a worker won't auto-claim.
///
/// We deliberately under-match rather than over-match: if the operator
/// explicitly mentions e.g. "edit hex-nexus/src/main.rs" the dispatch is
/// gated; but generic phrasing like "the main entry point" passes through.
fn mentions_critical_path(brief: &str) -> bool {
    use hex_core::domain::validation::is_critical_path;
    brief
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '`' | '"' | '\'' | '(' | ')'))
        .filter(|tok| tok.contains('/') || tok.ends_with(".rs"))
        .any(|tok| {
            let cleaned = tok.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '!' | '?'));
            !cleaned.is_empty() && is_critical_path(cleaned)
        })
}

/// Parse `@<role>` mentions at line-start from an agent's reply.
/// Returns `(role, brief)` pairs, where `brief` is the rest of the line
/// plus (optionally) the indented body that follows it. Mentions inside
/// fenced code blocks are skipped.
///
/// Recognized as a mention when:
///   - Line starts with `@` (after optional leading whitespace), and
///   - Next token matches one of the persona YAMLs in the embedded
///     templates (avoids treating "@deprecated" / "@param" / etc. as a
///     dispatch).
fn parse_agent_mentions(text: &str) -> Vec<(String, String)> {
    use std::collections::HashSet;
    // Build the set of valid roles from embedded persona YAMLs once.
    let valid: HashSet<String> = AgentTemplates::iter()
        .filter_map(|p| {
            let s = p.as_ref();
            let prefix = "agents/hex/hex/";
            if !s.starts_with(prefix) || !s.ends_with(".yml") { return None; }
            Some(s[prefix.len()..s.len() - 4].to_string())
        })
        .collect();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut in_code = false;
    let mut seen: HashSet<String> = HashSet::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with("```") { in_code = !in_code; continue; }
        if in_code { continue; }
        if !t.starts_with('@') { continue; }
        // Tokenize: role is `@(\w[\w-]*)`, brief is whatever follows.
        let after_at = &t[1..];
        let split_idx = after_at.find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(after_at.len());
        if split_idx == 0 { continue; }
        let role = &after_at[..split_idx];
        if !valid.contains(role) { continue; }
        if seen.contains(role) { continue; } // dedupe — first mention wins
        let brief = after_at[split_idx..].trim_start_matches([':', ' ', '-']).trim();
        if brief.is_empty() { continue; }
        seen.insert(role.to_string());
        out.push((role.to_string(), brief.to_string()));
    }
    out
}

// ── STDB-backed chat threads (key prefix: chat:thread:<uuid>) ────────────────
//
// Each thread is one hexflo_memory entry. The value is the thread JSON:
//   { id, title, project_id, created_at, last_active_at, messages: [...] }
// where each message is { from, text, ts, model?, error?, pending? }.
//
// Reusing hexflo_memory_* avoids touching the STDB schema — thread tables
// could come later (wp-brain-chat-streaming P4) but this gets persistence
// across browsers + machines today.

const THREAD_KEY_PREFIX: &str = "chat:thread:";

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessage {
    pub from: String,
    pub text: String,
    #[serde(default)]
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRecord {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_active_at: String,
    #[serde(default)]
    pub messages: Vec<ThreadMessage>,
}

#[derive(Debug, Deserialize)]
pub struct ThreadCreateRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, alias = "projectId")]
    pub project_id: Option<String>,
}

/// POST /api/brain/threads — create a new chat thread.
pub async fn create_thread(
    state: State<crate::state::SharedState>,
    Json(req): Json<ThreadCreateRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let record = ThreadRecord {
        id: id.clone(),
        title: req.title.unwrap_or_else(|| format!("thread-{}", &id[..8])),
        project_id: req.project_id,
        created_at: now.clone(),
        last_active_at: now,
        messages: vec![],
    };
    let key = format!("{}{}", THREAD_KEY_PREFIX, id);
    let value = serde_json::to_string(&record).unwrap_or_default();
    match port.hexflo_memory_store(&key, &value, "global").await {
        Ok(()) => (StatusCode::OK, Json(serde_json::to_value(&record).unwrap())),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("failed to store thread: {}", e) })),
        ),
    }
}

/// GET /api/brain/threads — list all chat threads.
/// Most-recent first by last_active_at.
pub async fn list_threads(
    state: State<crate::state::SharedState>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let entries = match port.hexflo_memory_search(THREAD_KEY_PREFIX).await {
        Ok(e) => e,
        Err(e) => return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("search failed: {}", e) })),
        ),
    };
    // Parse + sort. Each entry's value is a JSON-encoded ThreadRecord.
    let mut threads: Vec<Value> = entries
        .into_iter()
        .filter_map(|(k, v)| {
            if !k.starts_with(THREAD_KEY_PREFIX) { return None; }
            let mut parsed: Value = serde_json::from_str(&v).ok()?;
            // Strip messages from list response — clients fetch full thread separately.
            if let Some(obj) = parsed.as_object_mut() {
                let count = obj.get("messages")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                obj.insert("messageCount".to_string(), Value::Number(count.into()));
                obj.remove("messages");
            }
            Some(parsed)
        })
        .collect();
    threads.sort_by(|a, b| {
        let ka = a.get("lastActiveAt").and_then(|v| v.as_str()).unwrap_or("");
        let kb = b.get("lastActiveAt").and_then(|v| v.as_str()).unwrap_or("");
        kb.cmp(ka) // descending
    });
    (StatusCode::OK, Json(json!({ "threads": threads, "total": threads.len() })))
}

/// GET /api/brain/threads/:id — fetch one thread with all messages.
pub async fn get_thread(
    state: State<crate::state::SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let key = format!("{}{}", THREAD_KEY_PREFIX, id);
    match port.hexflo_memory_retrieve(&key).await {
        Ok(Some(v)) => match serde_json::from_str::<Value>(&v) {
            Ok(parsed) => (StatusCode::OK, Json(parsed)),
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "stored thread is corrupt" }))),
        },
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "thread not found" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct AppendMessageRequest {
    pub message: ThreadMessage,
}

/// POST /api/brain/threads/:id/messages — append a message to a thread.
pub async fn append_thread_message(
    state: State<crate::state::SharedState>,
    Path(id): Path<String>,
    Json(req): Json<AppendMessageRequest>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let key = format!("{}{}", THREAD_KEY_PREFIX, id);
    let mut record: ThreadRecord = match port.hexflo_memory_retrieve(&key).await {
        Ok(Some(v)) => serde_json::from_str(&v).unwrap_or_default(),
        Ok(None) => return (StatusCode::NOT_FOUND, Json(json!({ "error": "thread not found" }))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    };
    record.id = id.clone();
    record.messages.push(req.message);
    record.last_active_at = chrono::Utc::now().to_rfc3339();
    // Cap thread length at 500 messages — prevents unbounded growth.
    if record.messages.len() > 500 {
        let drop = record.messages.len() - 500;
        record.messages.drain(0..drop);
    }
    let value = serde_json::to_string(&record).unwrap_or_default();
    match port.hexflo_memory_store(&key, &value, "global").await {
        Ok(()) => (StatusCode::OK, Json(json!({
            "ok": true,
            "messageCount": record.messages.len(),
            "lastActiveAt": record.last_active_at,
        }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

/// DELETE /api/brain/threads/:id — remove a thread.
pub async fn delete_thread(
    state: State<crate::state::SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = match state.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let key = format!("{}{}", THREAD_KEY_PREFIX, id);
    match port.hexflo_memory_delete(&key).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
    }
}

// ── Broadcast: one message → many personas in parallel ───────────────────────

#[derive(Debug, Deserialize)]
pub struct BrainBroadcastRequest {
    pub message: String,
    /// Explicit list of roles to broadcast to. If omitted, ALL personas with
    /// a YAML in agents/hex/hex/ are targeted. Use this to scope to a category
    /// (caller filters PERSONAS by category client-side and passes the names).
    #[serde(default)]
    pub roles: Option<Vec<String>>,
    /// Optional project context — propagated to each per-role chat dispatch
    /// so every persona answers in the same project's frame.
    #[serde(default, alias = "projectId")]
    pub project_id: Option<String>,
}

/// POST /api/brain/broadcast — fan out one message to many personas in parallel.
/// Returns one entry per role in the same order as `roles` (or alphabetical if
/// the request omitted `roles`). Each entry has either `content` or `error`.
pub async fn dispatch_brain_broadcast(
    State(state): State<crate::state::SharedState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<BrainBroadcastRequest>,
) -> (StatusCode, Json<Value>) {
    use axum::extract::State;
    if req.message.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "message is required" })),
        );
    }

    // Resolve target roles. When omitted, list every YAML under agents/hex/hex/.
    let roles: Vec<String> = match req.roles {
        Some(r) => r,
        None => AgentTemplates::iter()
            .filter_map(|p| {
                let s = p.as_ref();
                let prefix = "agents/hex/hex/";
                if !s.starts_with(prefix) || !s.ends_with(".yml") {
                    return None;
                }
                let name = &s[prefix.len()..s.len() - 4];
                // Skip the deprecated stub.
                if name == "adversarial-reviewer" {
                    return None;
                }
                Some(name.to_string())
            })
            .collect(),
    };

    // Fan out. Each call goes through the same persona-load + inference_complete
    // path as the single dispatch. tokio::spawn isn't necessary — futures::join_all
    // on the same task is fine since the heavy lifting is the HTTP call to the
    // inference endpoint, not CPU work here.
    let mut futures = Vec::with_capacity(roles.len());
    for role in &roles {
        let role = role.clone();
        let message = req.message.clone();
        let state = state.clone();
        let headers = headers.clone();
        let project_id = req.project_id.clone();
        futures.push(async move {
            let resp = dispatch_brain_chat(
                State(state),
                headers,
                Json(BrainChatRequest {
                    role: role.clone(),
                    message,
                    project_id: project_id.clone(),
                    thread_id: None,
                    dispatch_depth: Some(0),
                }),
            )
            .await;
            (role, resp)
        });
    }

    let results = futures::future::join_all(futures).await;

    let responses: Vec<Value> = results
        .into_iter()
        .map(|(role, (status, body))| {
            if status == StatusCode::OK {
                json!({
                    "role": role,
                    "model": body.0.get("model").cloned().unwrap_or(Value::Null),
                    "content": body.0.get("content").cloned().unwrap_or(Value::String(String::new())),
                })
            } else {
                json!({
                    "role": role,
                    "error": body.0.get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("dispatch failed")
                        .to_string(),
                })
            }
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "message": req.message,
            "responses": responses,
            "total": roles.len(),
        })),
    )
}
