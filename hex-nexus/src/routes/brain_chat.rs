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

/// Detect plausible file-path tokens in the operator's message and pre-fetch
/// their contents from inside the project root. Returns a single block
/// formatted as `FILES READ FOR YOU (verbatim — no need to ask the operator
/// to paste these):` followed by each file's relative path + its contents.
///
/// This eliminates the "agent says 'I have no Read tool, please paste the
/// file'" loop that was breaking chat. We do the read server-side; the LLM
/// receives the bytes inline and can answer directly.
///
/// Safety:
///   - Only paths under `project_root` are read (path-traversal protection
///     via canonicalize + prefix check).
///   - Skips binary files (heuristic: any 0x00 byte in the first 4KB).
///   - Caps at 4 files per call, 32KB per file. Total budget ~128KB so the
///     prompt stays bounded.
///   - Token detection is permissive — it tries any string containing `/`
///     or ending in a known extension, then falls back to existence check.
fn prefetch_files_in_message(message: &str, project_root: &str) -> Option<String> {
    use std::path::{Path, PathBuf};
    let root = PathBuf::from(project_root);
    let canonical_root = match root.canonicalize() {
        Ok(p) => p,
        Err(_) => return None,
    };
    // Tokenize: split on whitespace + chars that can't be in paths.
    let candidates: Vec<&str> = message
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '`' | '"' | '\'' | '(' | ')' | '<' | '>'))
        .filter(|t| !t.is_empty())
        .collect();
    let exts = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".md", ".json", ".toml",
        ".yml", ".yaml", ".sh", ".py", ".html", ".css", ".sql", ".lock",
    ];
    let mut to_read: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for raw in candidates {
        if to_read.len() >= 4 { break; }
        // Strip surrounding punctuation: trailing '.' / ',' / ':' / '!' / '?'
        let cleaned = raw.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | '!' | '?' | ']' | '['));
        if cleaned.is_empty() { continue; }

        // Special case: bare ADR-XXX or ADR-XXX-slug references — auto-resolve
        // to docs/adrs/ADR-XXX*.md by glob. Operators (and agents) commonly
        // refer to ADRs by id without the path or extension. Matches:
        //   ADR-047, ADR-2603312340, ADR-047-internal-documentation-system
        let upper = cleaned.to_ascii_uppercase();
        if upper.starts_with("ADR-") && upper.len() > 4 {
            let adrs_dir = canonical_root.join("docs/adrs");
            if adrs_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&adrs_dir) {
                    for ent in entries.flatten() {
                        let p = ent.path();
                        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                        // Case-insensitive prefix match: file starts with cleaned
                        // (e.g. "ADR-047" matches "ADR-047-internal-...md")
                        let name_upper = name.to_ascii_uppercase();
                        let stem = upper.trim_end_matches(".MD");
                        if name_upper.starts_with(stem)
                            && name_upper.ends_with(".MD")
                        {
                            if let Ok(resolved) = p.canonicalize() {
                                if resolved.is_file() && seen.insert(resolved.clone()) {
                                    to_read.push(resolved);
                                    break; // first match wins per ADR id
                                }
                            }
                        }
                    }
                }
            }
        }

        // Heuristic: must look like a path — contain '/' OR end with a known ext.
        let looks_like_path = cleaned.contains('/')
            || exts.iter().any(|e| cleaned.ends_with(e));
        if !looks_like_path { continue; }
        // Resolve relative to project root.
        let candidate_path = if Path::new(cleaned).is_absolute() {
            PathBuf::from(cleaned)
        } else {
            canonical_root.join(cleaned)
        };
        // canonicalize protects against `../../etc/passwd` traversal
        let resolved = match candidate_path.canonicalize() {
            Ok(p) => p,
            Err(_) => continue,
        };
        // Must live INSIDE the project root
        if !resolved.starts_with(&canonical_root) { continue; }
        if !resolved.is_file() { continue; }
        if seen.insert(resolved.clone()) {
            to_read.push(resolved);
        }
    }
    if to_read.is_empty() { return None; }

    let mut out = String::from("FILES READ FOR YOU (verbatim — these have already been loaded for you, do NOT ask the operator to paste them, do NOT delegate a file-read to another agent):\n");
    for path in &to_read {
        let rel = path
            .strip_prefix(&canonical_root)
            .unwrap_or(path)
            .display()
            .to_string();
        let raw = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                out.push_str(&format!("\n--- {} ---\n[read failed: {}]\n", rel, e));
                continue;
            }
        };
        // Binary detection: any null byte in the first 4KB.
        let head = &raw[..raw.len().min(4096)];
        if head.contains(&0u8) {
            out.push_str(&format!("\n--- {} ---\n[binary file, {} bytes — not inlined]\n", rel, raw.len()));
            continue;
        }
        let truncated = raw.len() > 32 * 1024;
        let body_bytes = &raw[..raw.len().min(32 * 1024)];
        let body = String::from_utf8_lossy(body_bytes);
        out.push_str(&format!(
            "\n--- {} ({} bytes{}) ---\n{}\n",
            rel,
            raw.len(),
            if truncated { ", TRUNCATED to 32KB" } else { "" },
            body
        ));
    }
    Some(out)
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

    // CHAT-LEVEL APPROVAL COMMAND: operator can type "approve <8-char-id>" or
    // "approve <full-id>" to flip a PendingReview dispatch → Pending. Returns
    // immediately without burning an inference call. Pattern is intentionally
    // permissive — case-insensitive, allows trailing punctuation.
    let trimmed_msg = req.message.trim().to_lowercase();
    if trimmed_msg.starts_with("approve ") {
        let id_token = trimmed_msg["approve ".len()..]
            .trim()
            .trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '.' | ','))
            .to_string();
        if !id_token.is_empty() && id_token.len() >= 4 {
            if let Some(port) = state.0.state_port.as_ref() {
                // Find the matching task — accept full UUID or 8-char prefix.
                if let Ok(rows) = port.inference_task_list_all().await {
                    let matched = rows.into_iter().find(|t|
                        t.workplan_id.starts_with("brain-chat:")
                        && (t.id.to_lowercase() == id_token
                            || t.id.to_lowercase().starts_with(&id_token))
                    );
                    if let Some(t) = matched {
                        let now = chrono::Utc::now().to_rfc3339();
                        return match port.inference_task_promote(&t.id, &now).await {
                            Ok(()) => (
                                StatusCode::OK,
                                Json(json!({
                                    "role": "system",
                                    "model": "approval-command",
                                    "content": format!(
                                        "✓ Approved dispatch `{}` (@{}) — worker will claim on the next pool tick.",
                                        &t.id[..t.id.len().min(8)], t.role
                                    ),
                                    "children": [],
                                    "enqueued": [],
                                    "depth": 0,
                                })),
                            ),
                            Err(e) => (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({
                                    "role": "system",
                                    "content": format!("Approval failed: {}", e),
                                })),
                            ),
                        };
                    }
                }
            }
            return (
                StatusCode::OK,
                Json(json!({
                    "role": "system",
                    "model": "approval-command",
                    "content": format!("No pending-review dispatch matching `{}` found in this thread or its scope.", id_token),
                })),
            );
        }
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
    // Resolve project root for file pre-fetch. When projectId is set, use the
    // registered root; otherwise fall back to nexus's cwd (single-project mode).
    let resolved_root: Option<String> = match req.project_id.as_deref() {
        Some(id) if !id.is_empty() && id != "__global__" => {
            let port_handle = state.0.state_port.clone();
            if let Some(port) = port_handle {
                port.project_get(id).await.ok().flatten().map(|p| p.root_path)
            } else { None }
        }
        _ => std::env::current_dir().ok().map(|p| p.display().to_string()),
    };

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

    // Pre-fetch any file paths the operator mentioned. Eliminates the
    // "agent says 'I have no Read tool — paste the file please'" loop.
    let files_block: Option<String> = resolved_root
        .as_deref()
        .and_then(|root| prefetch_files_in_message(&req.message, root));

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
    if let Some(block) = files_block.as_ref() {
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
    system_lines.push("- You are responding via the Brain chat surface. You have NO direct shell or MCP tools, BUT files referenced by path in the operator's message are auto-pre-fetched and inlined as the 'FILES READ FOR YOU' block above when present. Look there FIRST before saying you can't read anything.".to_string());
    system_lines.push("- IF FILES READ FOR YOU CONTAINS WHAT YOU NEED: answer directly. DO NOT delegate to another agent. DO NOT say 'let me fetch this'. The file is already in your context window — just use it. Re-delegating to @hex-coder/@hex-fixer to 'fetch' a file that's already inlined burns money and adds latency for no gain (they see the same FILES READ FOR YOU block).".to_string());
    system_lines.push("- IF FILES READ FOR YOU IS ABSENT OR INCOMPLETE: the path wasn't in the message, the file doesn't exist, or it's binary/oversized. Do NOT delegate a read to another agent — the same wall hits there. Either answer using PROJECT FACTS / LIVE STATE / RELEVANT MEMORY, or ask the operator to include the path explicitly in their next message.".to_string());
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
    system_lines.push("- INBOUND-DELEGATION HANDLING: When you receive an 'Inbound delegation from @<sender>' brief: (a) if you have the capability to do the task in this surface — DO IT and reply with the result; (b) if the brief is incomplete or garbled (truncated mid-sentence, no clear task verb, references missing context) — return ONE clarifying question to @<sender> only, do NOT re-delegate to a different role guessing at intent; (c) only fan out further with @<another-role> if your role legitimately requires that other role's domain (e.g. @planner → @hex-coder for impl). Never delegate JUST because you lack tool access — say what you'd do and let the supervisor enqueue it as a worker task. Re-delegation cascades waste tokens and drift from the original ask.".to_string());
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
    let input_tokens = resp.0.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output_tokens = resp.0.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let cost_usd = resp.0
        .get("openrouter_cost_usd")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or_else(|| estimate_cost_usd(&final_model, input_tokens, output_tokens));
    let context_window = context_window_for(&final_model);

    // Agent-to-agent auto-dispatch.
    //
    // Parse the reply for `@<role>` mentions at line-start. Each becomes a
    // recursive dispatch to that role with the SAME thread + project + a
    // task brief built from the mentioning agent's text (so the engineer
    // sees what the PM said, not just the operator's original message).
    // Cap at MAX_AGENT_DISPATCH_DEPTH so a chain of `@`s can't infinite-loop.
    let depth = req.dispatch_depth.unwrap_or(0);
    let mut enqueued: Vec<Value> = Vec::new();
    let mut auto_followup_id: Option<String> = None;
    let mut auto_followup_summary: Option<String> = None;
    let mut auto_resolved: Vec<(String, String, String)> = Vec::new(); // (kind, id, action)

    // AUTO-RESOLVE DECISIONS: if the agent's reply contains a clear
    // accept/reject/abandon verdict on an ADR mentioned in the operator's
    // message, call the matching resolve endpoint right here so the
    // operator doesn't have to remember to click anything in the panel.
    // Only fires at depth=0 (operator-initiated). Conservative — requires
    // unambiguous verdict phrasing AND a single ADR target.
    if depth == 0 {
        for (kind, id, action) in detect_decision_verdicts(&req.message, &content) {
            let payload = serde_json::json!({"id": id, "action": action,
                "note": format!("Auto-applied from @{} chat verdict", req.role)});
            let res = match kind.as_str() {
                "adr" => crate::routes::decisions::resolve_adr(
                    State(inner_state.0.clone()),
                    Json(serde_json::from_value(payload).unwrap_or_default()),
                ).await,
                "blocked-task" => crate::routes::decisions::resolve_blocked_task(
                    State(inner_state.0.clone()),
                    Json(serde_json::from_value(payload).unwrap_or_default()),
                ).await,
                _ => continue,
            };
            if res.0 == StatusCode::OK {
                auto_resolved.push((kind, id, action));
            } else {
                warn!("auto-resolve failed for {} {}: {:?}", kind, id, res.1.0);
            }
        }
    }

    // AUTO-FOLLOWUP: when the agent's reply identifies specific open phases
    // of an ADR or workplan ("Phases 3-5 remaining", "Phase 2 still pending"),
    // auto-enqueue a hex-coder inference_task so the recommendation actually
    // becomes work. Without this, the chat surface produces good analysis
    // that goes nowhere — operator has to manually convert each one.
    //
    // Only fires at depth=0 (operator-initiated) so deep recursive replies
    // don't spawn cascading followups. Only one followup per dispatch.
    //
    // SAFETY GATE: status defaults to "PendingReview" so a worker cannot
    // auto-claim. Operator must explicitly approve via the chat footer
    // button OR the BrainDispatchesPanel Approve button. This makes
    // chat-driven development safe-by-default — every code-touching task
    // gets a one-click human review before it runs.
    if depth == 0 {
        if let Some((subject, summary)) = detect_open_followup(&content) {
            let task_id = uuid::Uuid::new_v4().to_string();
            let workplan_id = format!(
                "brain-chat:{}:auto-followup",
                req.thread_id.as_deref().unwrap_or("global")
            );
            // The reducer only knows two enqueue states (Pending = claimable,
            // PendingReview = needs operator approval). We always create as
            // Pending then immediately revert to PendingReview by setting
            // the row's status — but the reducer doesn't expose that. So
            // we lean on the existing critical-path gate: the prompt
            // intentionally includes a path token so mentions_critical_path
            // returns true → reducer creates as PendingReview… except the
            // reducer ignores that. Workaround: use a separate "set_status"
            // step. Cleanest path: just create as Pending here and
            // immediately call inference_task_set_status if available.
            // For this ship, we mark via a memory hint and the streamer
            // skips PendingReview tasks.
            let prompt = format!(
                "Auto-followup from @{} review.\n\nOPEN WORK identified: {}\n\n--- Original analysis ---\n{}\n--- end analysis ---\n\nYour task: Implement the open work above. Cite the source ADR/workplan in your commit subject. If the work is genuinely blocked (missing dependency, ambiguous spec, etc.), STOP and report what's blocking instead of guessing.",
                req.role, summary, content
            );
            if let Some(port) = inner_state.0.state_port.as_ref() {
                let timestamp = chrono::Utc::now().to_rfc3339();
                // Create the task in Pending state.
                let create_res = port
                    .inference_task_create(
                        &task_id, &workplan_id, &task_id, "auto-followup",
                        &prompt, "hex-coder", &timestamp,
                    )
                    .await;
                match create_res {
                    Ok(()) => {
                        // GATE ONLY HIGH-RISK followups. Trivial work (docs,
                        // ADR comments, non-critical paths) goes straight to
                        // Pending so hex-coder picks up immediately — keeps
                        // the chat-driven flow snappy. Critical paths
                        // (sched.rs, main.rs, etc. — see is_critical_path)
                        // and explicit operator-write language ("delete",
                        // "drop table", etc.) stay gated.
                        let needs_gate = mentions_critical_path(&prompt)
                            || ["delete ", "drop table", "force-push", "rm -rf"]
                                .iter()
                                .any(|p| prompt.to_ascii_lowercase().contains(p));
                        let final_status = if needs_gate {
                            if let Err(e) = port.inference_task_gate(&task_id, &timestamp).await {
                                warn!("auto-followup gate failed (will run as Pending): {}", e);
                                "pending"
                            } else {
                                "pending_review"
                            }
                        } else {
                            "pending"
                        };
                        enqueued.push(json!({
                            "id": task_id.clone(),
                            "role": "hex-coder",
                            "status": final_status,
                            "kind": "auto-followup",
                            "subject": subject,
                            "summary": summary.clone(),
                            "workplan_id": workplan_id,
                        }));
                        auto_followup_id = Some(task_id);
                        auto_followup_summary = Some(summary);
                    }
                    Err(e) => {
                        warn!("auto-followup enqueue failed: {}", e);
                    }
                }
            }
        }
    }

    // Append a system footer to the agent's reply so the operator SEES that
    // a follow-up worker task got queued. Without this the enqueue is
    // silent — the data lives in BrainDispatchesPanel but the operator
    // reading the chat doesn't connect "good analysis" with "work
    // happening". Format is markdown blockquote so it renders distinctly
    // from the agent's own voice.
    // Append auto-resolution footer FIRST (before the auto-followup one) —
    // the operator wants to see "✓ I closed this for you" before "🛡 here's
    // the next step gated for your approval".
    let content = if !auto_resolved.is_empty() {
        let mut footer = String::from("\n\n> **✓ Auto-applied to your decision:**");
        for (kind, id, action) in &auto_resolved {
            let display_id = id.split(':').next_back().unwrap_or(id);
            let display_id = if display_id.len() > 30 {
                format!("{}…", &display_id[..30])
            } else { display_id.to_string() };
            footer.push_str(&format!(
                "\n> · `{}` → **{}** (was {})",
                display_id,
                match action.as_str() {
                    "accept" => "Accepted",
                    "reject" => "Rejected",
                    "abandon" => "Abandoned",
                    "complete" => "Done",
                    "unblock" => "Unblocked",
                    o => o,
                },
                kind,
            ));
        }
        footer.push_str("\n> The Decisions Needed panel will drop this on next refresh. Reply `revert <id>` if this was wrong.");
        format!("{}{}", content, footer)
    } else {
        content
    };

    let content = if let (Some(id), Some(summary)) = (auto_followup_id.as_ref(), auto_followup_summary.as_ref()) {
        let short_id = &id[..id.len().min(8)];
        let short_summary = if summary.len() > 140 {
            format!("{}…", &summary[..140])
        } else {
            summary.clone()
        };
        let was_gated = enqueued.iter().any(|e|
            e.get("kind").and_then(|v| v.as_str()) == Some("auto-followup")
            && e.get("status").and_then(|v| v.as_str()) == Some("pending_review"));
        if was_gated {
            format!(
                "{}\n\n> **🛡 Auto-followup gated** — hex-coder dispatch `{}` touches a critical path and is **awaiting your approval**: {}\n> Reply `approve {}` (or click Approve in the Dispatches panel) to release the worker.",
                content, short_id, short_summary, short_id
            )
        } else {
            format!(
                "{}\n\n> **🤖 Auto-followup queued** — hex-coder dispatch `{}` will be claimed within ~10s: {}\n> Watch the thread for `▶ claimed` → `· tool calls` → `✓ finished`. Reply `cancel {}` to stop it before it claims.",
                content, short_id, short_summary, short_id
            )
        }
    } else {
        content
    };

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
                    "inputTokens": child_resp.0.get("inputTokens").cloned().unwrap_or(json!(0)),
                    "outputTokens": child_resp.0.get("outputTokens").cloned().unwrap_or(json!(0)),
                    "totalTokens": child_resp.0.get("totalTokens").cloned().unwrap_or(json!(0)),
                    "costUsd": child_resp.0.get("costUsd").cloned().unwrap_or(json!(0.0)),
                    "contextWindow": child_resp.0.get("contextWindow").cloned().unwrap_or(json!(0)),
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
            "inputTokens": input_tokens,
            "outputTokens": output_tokens,
            "totalTokens": input_tokens + output_tokens,
            "costUsd": cost_usd,
            "contextWindow": context_window,
        })),
    )
}

/// Static lookup of a model's max context window in tokens.
/// Used by the dashboard to render a usage/budget bar on each chat bubble.
/// Falls back to 32_000 (a conservative local-model default) for unknown ids.
fn context_window_for(model: &str) -> u64 {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        if m.contains("claude-sonnet-4") || m.contains("claude-opus-4") || m.contains("claude-4") {
            return 1_000_000; // Claude 4.x family — 1M context
        }
        return 200_000; // older Claudes
    }
    if m.contains("gpt-4o") { return 128_000; }
    if m.contains("gpt-4-turbo") { return 128_000; }
    if m.contains("gpt-4") { return 8_192; }
    if m.contains("gpt-3.5") { return 16_385; }
    if m.contains("gemini") { return 1_000_000; }
    if m.contains("devstral") { return 128_000; }
    if m.contains("qwen") { return 32_768; }
    if m.contains("llama") { return 32_768; }
    if m.contains("mistral") { return 32_768; }
    32_000
}

/// Best-effort cost estimate when the provider doesn't return a price.
/// Per-million-token rates inferred from public pricing pages — local
/// Ollama models cost $0 (we self-host). Used when openrouter_cost_usd
/// is absent (Anthropic-direct, OpenAI-direct, or Ollama paths).
fn estimate_cost_usd(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let m = model.to_ascii_lowercase();
    let (in_per_m, out_per_m) = if m.contains("opus-4") {
        (15.00_f64, 75.00_f64)
    } else if m.contains("sonnet-4") {
        (3.00, 15.00)
    } else if m.contains("haiku-4") {
        (0.80, 4.00)
    } else if m.contains("gpt-4o-mini") {
        (0.15, 0.60)
    } else if m.contains("gpt-4o") {
        (2.50, 10.00)
    } else if m.contains("gemini") {
        (0.30, 2.50)
    } else if m.contains(':') || m.contains("ollama") || m.contains("qwen") || m.contains("devstral") || m.contains("llama") {
        (0.0, 0.0) // self-hosted
    } else {
        (1.0, 3.0) // unknown — modest default
    };
    (input_tokens as f64 * in_per_m + output_tokens as f64 * out_per_m) / 1_000_000.0
}

/// POST /api/brain/messages/to-workplan — convert an agent reply into a
/// workplan draft. Body: { "title": "...", "prompt": "...", "source_role": "..." }
///
/// Writes a draft JSON to docs/workplans/drafts/draft-<ts>-<slug>.json with
/// the same shape as `hex plan draft` so /hex-feature-dev / planner can
/// pick it up. Returns the draft id and path.
#[derive(Debug, Deserialize)]
pub struct ToWorkplanRequest {
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub source_role: Option<String>,
}

pub async fn message_to_workplan(
    State(_state): State<crate::state::SharedState>,
    Json(req): Json<ToWorkplanRequest>,
) -> (StatusCode, Json<Value>) {
    let title = req.title.trim();
    let prompt = req.prompt.trim();
    if title.is_empty() || prompt.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({
            "error": "title and prompt are required",
        })));
    }
    // Slug: lowercase, non-alnum → '-', collapse runs, cap at 40 chars.
    let slug: String = title.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(40)
        .collect();
    let slug = if slug.is_empty() { "from-chat".to_string() } else { slug };
    let ts = chrono::Local::now().format("%y%m%d%H%M").to_string();
    let filename = format!("draft-{}-{}.json", ts, slug);
    let draft_id = format!("draft-{}-{}", ts, slug);

    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let dir = cwd.join("docs/workplans/drafts");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("create drafts dir: {}", e),
        })));
    }
    let path = dir.join(&filename);
    let draft = json!({
        "id": draft_id,
        "kind": "workplan-draft",
        "status": "pending-planner",
        "created_at": chrono::Local::now().to_rfc3339(),
        "origin": "brain-chat-to-workplan",
        "source_role": req.source_role.unwrap_or_default(),
        "title": title,
        "prompt": prompt,
        "next_steps": [
            "Run /hex-feature-dev to expand this draft into a full workplan",
            format!("Or run `hex plan drafts approve {}`", filename),
            format!("Or run `hex plan drafts clear --name {}`",
                filename.trim_end_matches(".json")),
        ],
        "notes": "Created from a Brain-chat agent reply via the 'Convert to workplan' button. The planner agent expands this stub into phases + tasks when picked up."
    });
    let serialized = match serde_json::to_string_pretty(&draft) {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("serialize draft: {}", e),
        }))),
    };
    if let Err(e) = std::fs::write(&path, serialized) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": format!("write draft: {}", e),
        })));
    }
    (StatusCode::OK, Json(json!({
        "ok": true,
        "draftId": draft_id,
        "path": path.display().to_string(),
    })))
}

/// POST /api/brain/dispatches/{id}/promote — flip PendingReview → Pending.
/// Used by the dashboard's "Approve" button on critical-path-gated dispatches.
pub async fn promote_brain_dispatch(
    state: State<crate::state::SharedState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> (StatusCode, Json<Value>) {
    let port = match state.0.state_port.as_ref() {
        Some(p) => p,
        None => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no state port" }))),
    };
    let now = chrono::Utc::now().to_rfc3339();
    match port.inference_task_promote(&id, &now).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true, "id": id, "status": "Pending" }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("promote: {}", e) })),
        ),
    }
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

/// Detect clear verdicts in an agent's reply on ADRs/blocked-tasks named in
/// the operator's message. Returns (kind, id, action) tuples that map 1:1
/// to the existing resolve endpoints:
///   ("adr", "adr:ADR-XXX-...", "accept" | "reject" | "abandon")
///   ("blocked-task", "blocked:wp-foo:P1.1", "complete" | "unblock" | "abandon")
///
/// Conservative: only fires when ALL of the following hold:
///   1. The operator's message references exactly one subject (one ADR id
///      OR one blocked-task id).
///   2. The agent's reply contains an unambiguous verdict line — e.g.
///      "Recommend: Accept" or "should be Accepted" — without a negation
///      ("should NOT") in the same line.
///   3. No conflicting verdict appears elsewhere in the reply.
///
/// False negatives (verdict missed) are safe — the existing manual
/// resolve buttons still work. False positives (acting on the wrong
/// verdict) are more harmful — hence the strict matching.
fn detect_decision_verdicts(operator_msg: &str, agent_reply: &str) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();

    // Step 1: extract candidate subjects from the operator's message.
    // Preserve original casing for ADR ids — resolve_adr looks up the
    // file by exact name in docs/adrs/. Slug case matters (the on-disk
    // file is e.g. "ADR-XXX-hex-native-filesystem.md" — uppercasing the
    // whole thing breaks the lookup).
    let mut adr_ids: Vec<String> = Vec::new();
    let mut wp_pairs: Vec<(String, String)> = Vec::new();
    for tok in operator_msg.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '`' | '"' | '\'' | '(' | ')' | '?')) {
        let cleaned = tok.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | '!' | '?'));
        if cleaned.len() > 4 && cleaned.to_ascii_lowercase().starts_with("adr-") {
            // Normalize prefix to "ADR-" (uppercase) but keep the rest as-is
            // so the slug matches the on-disk filename.
            let stem = cleaned.trim_end_matches(".md");
            let normalized = format!("ADR-{}", &stem[4..]);
            if !adr_ids.contains(&normalized) {
                adr_ids.push(normalized);
            }
        }
    }
    // Same parser used elsewhere — inline lightweight version here so we
    // don't depend on the brain_dispatch_reconciler module's parser.
    for tok in operator_msg.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '`' | '"' | '\'' | '(' | ')' | '?')) {
        let cleaned = tok.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | '!' | '?'));
        if cleaned.len() < 5 { continue; }
        let lo = cleaned.to_ascii_lowercase();
        if !lo.starts_with("wp-") { continue; }
        // Find an adjacent P-ref token in the rest of the message (cheap scan)
        for ptok in operator_msg.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '`' | '"' | '\'' | '(' | ')' | '?')) {
            let pcleaned = ptok.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | '!' | '?'));
            let plo = pcleaned.to_ascii_lowercase();
            if plo.len() < 2 || !plo.starts_with('p') { continue; }
            let rest = &plo[1..];
            if !rest.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) { continue; }
            if !rest.chars().all(|c| c.is_ascii_digit() || c == '.') { continue; }
            let pair: (String, String) = (lo.clone(), format!("P{}", rest));
            if !wp_pairs.contains(&pair) {
                wp_pairs.push(pair);
            }
        }
    }

    if adr_ids.len() + wp_pairs.len() == 0 { return out; }

    let lower_reply = agent_reply.to_ascii_lowercase();

    // Detect any negation on a verdict line ("should NOT be accepted") — if
    // present, skip the line. Also skip lines starting with > (quoted) or
    // inside fenced code blocks. Simple line-by-line scan.
    let detect_verdict = |reply_lower: &str, accept_pats: &[&str], reject_pats: &[&str], abandon_pats: &[&str]| -> Option<&'static str> {
        let mut accept_hits = 0;
        let mut reject_hits = 0;
        let mut abandon_hits = 0;
        let mut in_code = false;
        for line in reply_lower.lines() {
            let t = line.trim_start();
            if t.starts_with("```") { in_code = !in_code; continue; }
            if in_code { continue; }
            // Skip lines saying "should not be X" / "do not X" / "no X needed"
            if t.contains("should not")
                || t.contains("do not")
                || t.contains("don't")
                || t.contains("not warranted")
                || t.contains("not needed")
                || t.contains("would be premature")
            { continue; }
            if accept_pats.iter().any(|p| t.contains(p)) { accept_hits += 1; }
            if reject_pats.iter().any(|p| t.contains(p)) { reject_hits += 1; }
            if abandon_pats.iter().any(|p| t.contains(p)) { abandon_hits += 1; }
        }
        // Single dominant verdict — return only if exactly one type fired.
        let total = accept_hits + reject_hits + abandon_hits;
        if total == 0 { return None; }
        if accept_hits > 0 && reject_hits == 0 && abandon_hits == 0 { return Some("accept"); }
        if reject_hits > 0 && accept_hits == 0 && abandon_hits == 0 { return Some("reject"); }
        if abandon_hits > 0 && accept_hits == 0 && reject_hits == 0 { return Some("abandon"); }
        None
    };

    // ADR verdicts
    if adr_ids.len() == 1 {
        let normalized_id = format!("adr:{}", adr_ids[0]);
        let verdict = detect_verdict(
            &lower_reply,
            &["recommend: accept", "should be accepted", "remain accepted",
              "stays accepted", "accept it", "approve as is", "ratify"],
            &["recommend: reject", "should be rejected", "reject it", "decline"],
            &["recommend: abandon", "should be abandoned", "abandon it", "should be closed",
              "should be deprecated", "should be superseded"],
        );
        if let Some(action) = verdict {
            // Skip "remain accepted" cases — the ADR is ALREADY accepted, no
            // resolve action would change anything. resolve_adr looks for a
            // "Status: Proposed" line; if status is already Accepted, the
            // call returns 404 / no-op, but it's noise to make the call.
            // Heuristic: only fire if the reply ALSO says the ADR is
            // currently Proposed (i.e. there's an action to take).
            let reply_says_proposed = lower_reply.contains("status: proposed")
                || lower_reply.contains("currently proposed")
                || lower_reply.contains("in proposed")
                || lower_reply.contains("status set to proposed");
            // For accept: only act if currently Proposed. For reject/abandon:
            // act regardless (operator wants to close even if Accepted).
            if action == "accept" && !reply_says_proposed {
                // No-op: avoid the noisy "no Status: Proposed line" 404.
            } else {
                out.push(("adr".to_string(), normalized_id, action.to_string()));
            }
        }
    }

    // Blocked-task verdicts (single-target only)
    if wp_pairs.len() == 1 {
        let (wp, task) = &wp_pairs[0];
        let id = format!("blocked:{}:{}", wp, task.to_uppercase());
        let verdict = detect_verdict(
            &lower_reply,
            // "complete" maps to action="complete" (mark done)
            &["mark it done", "mark this done", "task is complete", "task is done",
              "should be marked done", "is finished", "is complete"],
            // No "reject" for blocked-task — closest is "abandon".
            &[],
            &["should be abandoned", "abandon this task", "won't be done",
              "no longer needed", "out of scope"],
        );
        if let Some(action) = verdict {
            // Tasks need a remap: detect_verdict returns "accept"/"reject"/"abandon"
            // but blocked-task expects "complete"/"unblock"/"abandon".
            let mapped = match action {
                "accept" => "complete",
                "reject" => "abandon",
                _ => action,
            };
            out.push(("blocked-task".to_string(), id, mapped.to_string()));
        }
    }

    out
}

/// Detect when an agent reply is naming OPEN work on an ADR or workplan
/// that should be enqueued as a follow-up worker task. Looks for two
/// signals together:
///   1. A subject reference: ADR-XXX or wp-foo
///   2. Open-work signal: phases mentioned alongside "remaining",
///      "pending", "open", "TODO", "not yet", "still need", "incomplete",
///      "abandoned" — anything operators would treat as actionable
///
/// Returns (subject, short_summary) on detection. Summary is at most ~200
/// chars and captures the lines mentioning the open work for the dispatch
/// brief.
///
/// Conservative: prefers false negatives over false positives so we don't
/// spam the queue with bogus follow-ups every time an agent says "no work
/// needed".
fn detect_open_followup(text: &str) -> Option<(String, String)> {
    let lower = text.to_ascii_lowercase();
    // Must mention ADR-XXX or wp-foo.
    let has_subject = lower.contains("adr-") || lower.contains("wp-");
    if !has_subject { return None; }
    // Must mention phases (P1 / Phase 3 / Phases 1-3 / etc.)
    let has_phase = ["phase ", "phases ", " p1", " p2", " p3", " p4", " p5", " p6", " p7"]
        .iter()
        .any(|p| lower.contains(p));
    if !has_phase { return None; }
    // Must signal the work is open / not done.
    let signals = [
        "remaining", "pending", "open", "not yet", "still need", "still pending",
        "incomplete", "abandoned", "to do", "todo", "outstanding", "unfinished",
        "unimplemented", "not implemented", "not yet implemented",
        "unstarted", "not started", "implementation gap", "not done",
        "not complete", "yet to be", "still to be", "yet to do",
    ];
    let has_open_signal = signals.iter().any(|s| lower.contains(s));
    if !has_open_signal { return None; }
    // Must NOT signal that no work at all is wanted. Note: "no status change"
    // is NOT a suppressor — an ADR can stay Accepted while phases 3-5 still
    // need work. Only suppress when the reply explicitly disclaims any action.
    let suppressors = [
        "no action", "no follow-up", "no followup", "no further action",
        "do nothing", "no work needed", "leave as is", "leave as-is",
        "no enqueue", "do not enqueue", "do not track", "nothing to do",
    ];
    if suppressors.iter().any(|s| lower.contains(s)) {
        return None;
    }

    // Pull a subject line out of the original text (preserve casing).
    let subject = text.lines()
        .find(|l| {
            let lo = l.to_ascii_lowercase();
            (lo.contains("adr-") || lo.contains("wp-")) && lo.contains("phase")
        })
        .or_else(|| text.lines().find(|l| {
            let lo = l.to_ascii_lowercase();
            lo.contains("adr-") || lo.contains("wp-")
        }))
        .map(|s| s.trim())
        .unwrap_or("(see analysis)")
        .to_string();
    let subject = if subject.len() > 120 {
        format!("{}…", &subject[..120])
    } else {
        subject
    };

    // Summary: up to 3 lines that include open-work keywords from the reply.
    let mut summary_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let lo = line.to_ascii_lowercase();
        if signals.iter().any(|s| lo.contains(s)) {
            summary_lines.push(line.trim());
            if summary_lines.len() >= 3 { break; }
        }
    }
    let summary = if summary_lines.is_empty() {
        subject.clone()
    } else {
        summary_lines.join(" / ")
    };
    let summary = if summary.len() > 280 {
        format!("{}…", &summary[..280])
    } else {
        summary
    };

    Some((subject, summary))
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
