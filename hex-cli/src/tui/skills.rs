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
}

/// Returns true if the trimmed input starts with '/'.
pub fn is_slash_command(input: &str) -> bool {
    input.trim_start().starts_with('/')
}

/// Dispatch a slash command and return its result.
pub async fn dispatch(input: &str, nexus_url: &str) -> SkillResult {
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
        _ => SkillResult::Unknown(cmd.to_string()),
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
            if let Some(name) = s.get("project_name").and_then(|v| v.as_str()) {
                lines.push(format!("  Project: {}", name));
            }
            if let Some(id) = s.get("project_id").and_then(|v| v.as_str()) {
                lines.push(format!("  ID: {}", id));
            }
            if let Some(ver) = s.get("version").and_then(|v| v.as_str()) {
                lines.push(format!("  Nexus: v{}", ver));
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
