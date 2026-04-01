//! Slash command parser and built-in skill handlers for `hex chat`.
//!
//! Commands are parsed client-side before sending to inference — no model
//! call is made for slash commands.

use std::time::Duration;

/// Result of dispatching a slash command.
pub enum SkillResult {
    /// Display these lines inline as a dimmed system message.
    Lines(Vec<String>),
    /// Clear conversation history (keeps system context).
    ClearHistory,
    /// Switch the active model for the session.
    SwitchModel(String),
    /// Save session to disk.
    Save,
    /// No visible effect.
    Noop,
    /// Unknown command — show error hint.
    Unknown(String),
    /// Inject this text as a user message and trigger inference.
    InjectMessage(String),
}

/// Returns true if the trimmed input starts with '/'.
pub fn is_slash_command(input: &str) -> bool {
    input.trim_start().starts_with('/')
}

/// Load user-defined skills from .claude/skills/ directories.
/// Returns (slash_name, markdown_body) pairs.
/// Scans: (1) CWD/.claude/skills/*.md, (2) ~/.claude/skills/*.md
/// Project-local files override global on name collision.
pub fn load_user_skills() -> Vec<(String, String)> {
    use std::collections::HashMap;
    let mut skills: HashMap<String, String> = HashMap::new();

    // Global first (lower priority)
    if let Some(home) = dirs::home_dir() {
        let global_dir = home.join(".claude/skills");
        load_skills_from_dir(&global_dir, &mut skills);
    }

    // Project-local (overrides global)
    let local_dir = std::path::Path::new(".claude/skills");
    load_skills_from_dir(local_dir, &mut skills);

    let mut result: Vec<(String, String)> = skills.into_iter().collect();
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

fn load_skills_from_dir(dir: &std::path::Path, skills: &mut std::collections::HashMap<String, String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = match path.file_stem().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        skills.insert(name, body);
    }
}

/// Dispatch a slash command and return its result.
pub async fn dispatch(input: &str, nexus_url: &str, user_skills: &[(String, String)]) -> SkillResult {
    let trimmed = input.trim();
    let (cmd, rest) = match trimmed.find(' ') {
        Some(pos) => (&trimmed[..pos], trimmed[pos + 1..].trim()),
        None => (trimmed, ""),
    };

    match cmd {
        "/help" => SkillResult::Lines(vec![
            "Available slash commands:".to_string(),
            "  /help              — show this help".to_string(),
            "  /clear             — clear conversation history".to_string(),
            "  /model <name>      — switch model for this session".to_string(),
            "  /context           — show current project context".to_string(),
            "  /adr <query>       — search ADRs".to_string(),
            "  /plan              — list active workplans / swarms".to_string(),
            "  /save              — save session to ~/.hex/sessions/".to_string(),
            "  /skills            — list user-defined skill commands".to_string(),
            "  /hex <cmd>         — run a hex CLI command (e.g. /hex plan list)".to_string(),
            "  /<skill-name>      — invoke a skill from .claude/skills/".to_string(),
        ]),
        "/clear" => SkillResult::ClearHistory,
        "/model" => {
            if rest.is_empty() {
                SkillResult::Lines(vec!["Usage: /model <name>  (e.g. /model qwen/qwen3-8b)".to_string()])
            } else {
                SkillResult::SwitchModel(rest.to_string())
            }
        }
        "/context" => fetch_context(nexus_url).await,
        "/adr" => search_adrs(nexus_url, rest).await,
        "/plan" => list_plans(nexus_url).await,
        "/save" => SkillResult::Save,
        "/skills" => {
            if user_skills.is_empty() {
                SkillResult::Lines(vec!["No skills found in .claude/skills/ or ~/.claude/skills/".to_string()])
            } else {
                let mut lines = vec!["Available skills:".to_string()];
                for (name, body) in user_skills {
                    let desc = body.lines()
                        .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
                        .unwrap_or("")
                        .trim_start_matches(|c: char| c == '-' || c == ' ')
                        .chars()
                        .take(60)
                        .collect::<String>();
                    lines.push(format!("  /{:<20} — {}", name, desc));
                }
                SkillResult::Lines(lines)
            }
        }
        "/hex" => {
            if rest.is_empty() {
                SkillResult::Lines(vec!["Usage: /hex <subcommand>  (e.g. /hex plan list)".to_string()])
            } else {
                run_hex_command(rest).await
            }
        }
        _ => {
            // Check user skills before returning Unknown
            let skill_name = cmd.trim_start_matches('/');
            if let Some((_, body)) = user_skills.iter().find(|(n, _)| n == skill_name) {
                let truncated = if body.len() > 8000 {
                    format!("{}… (truncated)", &body[..8000])
                } else {
                    body.clone()
                };
                return SkillResult::InjectMessage(truncated);
            }
            SkillResult::Unknown(format!("{} — try /skills to list available commands", cmd))
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn get_json(url: &str) -> Option<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    if resp.status().is_success() {
        resp.json().await.ok()
    } else {
        None
    }
}

async fn get_json_with_query(base: &str, params: &[(&str, &str)]) -> Option<serde_json::Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client.get(base).query(params).send().await.ok()?;
    if resp.status().is_success() {
        resp.json().await.ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

async fn fetch_context(nexus_url: &str) -> SkillResult {
    let status = get_json(&format!("{}/api/status", nexus_url)).await;
    let mut lines = vec!["Current project context:".to_string()];

    match &status {
        Some(s) => {
            if let Some(name) = s.get("name").and_then(|v| v.as_str()) {
                lines.push(format!("  Project: {}", name));
            }
            if let Some(ver) = s.get("version").and_then(|v| v.as_str()) {
                lines.push(format!("  Nexus: v{}", ver));
            }
            if let Some(hash) = s.get("buildHash").and_then(|v| v.as_str()) {
                lines.push(format!("  Build: {}", hash));
            }
        }
        None => {
            lines.push("  (nexus offline — no context available)".to_string());
        }
    }

    SkillResult::Lines(lines)
}

async fn search_adrs(nexus_url: &str, query: &str) -> SkillResult {
    if query.is_empty() {
        return SkillResult::Lines(vec!["Usage: /adr <search query>".to_string()]);
    }

    let val = get_json_with_query(&format!("{}/api/adrs", nexus_url), &[("q", query)]).await;

    let mut lines = vec![format!("ADRs matching '{}':", query)];

    match val.as_ref().and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => {
            for adr in arr.iter().take(10) {
                let id = adr.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let title = adr.get("title").and_then(|v| v.as_str()).unwrap_or("(untitled)");
                let status = adr.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                lines.push(format!("  {} [{}] — {}", id, status, title));
            }
        }
        Some(_) => lines.push("  No results found.".to_string()),
        None => lines.push("  (could not reach nexus)".to_string()),
    }

    SkillResult::Lines(lines)
}

async fn list_plans(nexus_url: &str) -> SkillResult {
    let val = get_json(&format!("{}/api/hexflo/swarms", nexus_url)).await;

    let mut lines = vec!["Active swarms / workplans:".to_string()];

    match val.as_ref().and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => {
            for s in arr.iter().take(15) {
                let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                let status = s.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                let task_count = s.get("tasks")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        let done = a.iter().filter(|t| {
                            t.get("status").and_then(|s| s.as_str()) == Some("completed")
                        }).count();
                        format!("{}/{}", done, a.len())
                    })
                    .unwrap_or_else(|| "?".to_string());
                lines.push(format!("  {} [{}] — {} tasks done", name, status, task_count));
            }
        }
        Some(_) => lines.push("  No active swarms.".to_string()),
        None => lines.push("  (could not reach nexus)".to_string()),
    }

    SkillResult::Lines(lines)
}

async fn run_hex_command(subcommand: &str) -> SkillResult {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(_) => return SkillResult::Lines(vec!["Error: cannot find hex binary".to_string()]),
    };
    // Split on whitespace — never use sh -c
    let argv: Vec<&str> = subcommand.split_whitespace().collect();
    if argv.is_empty() {
        return SkillResult::Lines(vec!["Usage: /hex <subcommand>".to_string()]);
    }

    let output = match tokio::process::Command::new(&exe)
        .args(&argv)
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return SkillResult::Lines(vec![format!("Error spawning hex: {}", e)]),
    };

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    if lines.len() > 200 {
        lines.truncate(200);
        lines.push("... (output truncated at 200 lines)".to_string());
    }
    if lines.is_empty() {
        lines.push("(no output)".to_string());
    }
    SkillResult::Lines(lines)
}
