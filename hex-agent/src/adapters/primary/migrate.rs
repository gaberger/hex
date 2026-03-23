//! Migration CLI — reads .claude/settings.json hooks, .claude/skills/*.md,
//! and .claude/agents/*.yml and imports them into hex-hub's state backend
//! via IStatePort (works with both SpacetimeDB and SQLite).

use std::path::Path;

/// Migrate Claude Code configuration into hex-hub state backend.
///
/// Reads from the filesystem and POSTs to hex-hub's REST API.
/// Non-destructive — does not delete source files.
pub struct ConfigMigrator {
    hub_url: String,
}

impl ConfigMigrator {
    pub fn new(hub_url: &str) -> Self {
        Self {
            hub_url: hub_url.to_string(),
        }
    }

    /// Run the full migration pipeline.
    pub async fn migrate(&self, project_dir: &str) -> Result<MigrationReport, MigrationError> {
        let mut report = MigrationReport::default();

        // 1. Migrate skills from .claude/skills/*.md
        let skill_dir = format!("{}/.claude/skills", project_dir);
        if Path::new(&skill_dir).exists() {
            match self.migrate_skills(&skill_dir).await {
                Ok(count) => report.skills_imported = count,
                Err(e) => report.errors.push(format!("Skills: {}", e)),
            }
        }

        // 2. Migrate agents from .claude/agents/*.yml
        let agent_dir = format!("{}/.claude/agents", project_dir);
        if Path::new(&agent_dir).exists() {
            match self.migrate_agents(&agent_dir).await {
                Ok(count) => report.agents_imported = count,
                Err(e) => report.errors.push(format!("Agents: {}", e)),
            }
        }

        // 3. Migrate hooks from .claude/settings.json
        let settings_path = format!("{}/.claude/settings.json", project_dir);
        if Path::new(&settings_path).exists() {
            match self.migrate_hooks(&settings_path).await {
                Ok(count) => report.hooks_imported = count,
                Err(e) => report.errors.push(format!("Hooks: {}", e)),
            }
        }

        Ok(report)
    }

    async fn migrate_skills(&self, dir: &str) -> Result<usize, MigrationError> {
        let mut count = 0;
        let entries = std::fs::read_dir(dir)
            .map_err(|e| MigrationError(format!("Cannot read {}: {}", dir, e)))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| MigrationError(format!("Cannot read {}: {}", path.display(), e)))?;

                let (frontmatter, body) = parse_frontmatter(&content);
                let name = frontmatter.get("name").cloned()
                    .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()))
                    .unwrap_or_default();
                let description = frontmatter.get("description").cloned().unwrap_or_default();

                // Build triggers from frontmatter
                let mut triggers = Vec::new();
                if let Some(trigger_str) = frontmatter.get("trigger") {
                    triggers.push(serde_json::json!({
                        "trigger_type": "slash_command",
                        "trigger_value": format!("/{}", trigger_str),
                    }));
                }
                // Default slash command from name
                triggers.push(serde_json::json!({
                    "trigger_type": "slash_command",
                    "trigger_value": format!("/{}", name),
                }));

                let payload = serde_json::json!({
                    "name": name,
                    "description": description,
                    "triggersJson": serde_json::to_string(&triggers).unwrap_or_default(),
                    "body": body,
                    "source": "migrate-config",
                });

                let url = format!("{}/api/state/skills", self.hub_url);
                let resp = reqwest::Client::new().post(&url).json(&payload).send().await
                    .map_err(|e| MigrationError(e.to_string()))?;

                if resp.status().is_success() {
                    count += 1;
                    eprintln!("  ✓ Skill: {}", name);
                } else {
                    eprintln!("  ✗ Skill {}: HTTP {}", name, resp.status());
                }
            }
        }
        Ok(count)
    }

    async fn migrate_agents(&self, dir: &str) -> Result<usize, MigrationError> {
        let mut count = 0;
        let entries = std::fs::read_dir(dir)
            .map_err(|e| MigrationError(format!("Cannot read {}: {}", dir, e)))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yml" || e == "yaml") {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| MigrationError(format!("Cannot read {}: {}", path.display(), e)))?;

                let yaml: serde_json::Value = serde_yaml::from_str(&content)
                    .map_err(|e| MigrationError(format!("Bad YAML {}: {}", path.display(), e)))?;

                let name = yaml.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");

                let payload = serde_json::json!({
                    "name": name,
                    "description": yaml.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                    "rolePrompt": yaml.get("role_prompt").and_then(|v| v.as_str()).unwrap_or(""),
                    "allowedToolsJson": serde_json::to_string(
                        &yaml.get("allowed_tools").cloned().unwrap_or(serde_json::json!([]))
                    ).unwrap_or_default(),
                    "constraintsJson": serde_json::to_string(
                        &yaml.get("constraints").cloned().unwrap_or(serde_json::json!({}))
                    ).unwrap_or_default(),
                    "model": yaml.get("model").and_then(|v| v.as_str()).unwrap_or(""),
                    "maxTurns": yaml.get("max_turns").and_then(|v| v.as_u64()).unwrap_or(50),
                    "metadataJson": serde_json::to_string(
                        &yaml.get("metadata").cloned().unwrap_or(serde_json::json!({}))
                    ).unwrap_or_default(),
                    "source": "migrate-config",
                });

                let url = format!("{}/api/state/agent-definitions", self.hub_url);
                let resp = reqwest::Client::new().post(&url).json(&payload).send().await
                    .map_err(|e| MigrationError(e.to_string()))?;

                if resp.status().is_success() {
                    count += 1;
                    eprintln!("  ✓ Agent: {}", name);
                } else {
                    eprintln!("  ✗ Agent {}: HTTP {}", name, resp.status());
                }
            }
        }
        Ok(count)
    }

    async fn migrate_hooks(&self, settings_path: &str) -> Result<usize, MigrationError> {
        let content = std::fs::read_to_string(settings_path)
            .map_err(|e| MigrationError(format!("Cannot read {}: {}", settings_path, e)))?;

        let settings: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| MigrationError(format!("Bad JSON in {}: {}", settings_path, e)))?;

        let hooks = match settings.get("hooks") {
            Some(serde_json::Value::Array(arr)) => arr.clone(),
            Some(serde_json::Value::Object(obj)) => {
                // Claude Code format: { "PreToolUse": [...], "PostTask": [...] }
                let mut flat = Vec::new();
                for (event, items) in obj {
                    if let serde_json::Value::Array(arr) = items {
                        for item in arr {
                            let mut hook = item.clone();
                            if let serde_json::Value::Object(ref mut m) = hook {
                                m.insert("event_type".into(), serde_json::Value::String(
                                    camel_to_snake(event),
                                ));
                            }
                            flat.push(hook);
                        }
                    }
                }
                flat
            }
            _ => return Ok(0),
        };

        let mut count = 0;
        for hook in hooks {
            let event_type = hook.get("event_type")
                .or_else(|| hook.get("event"))
                .and_then(|v| v.as_str())
                .unwrap_or("post_task");

            let command = hook.get("command").and_then(|v| v.as_str()).unwrap_or("");

            let payload = serde_json::json!({
                "eventType": event_type,
                "handlerType": "shell",
                "handlerConfigJson": serde_json::json!({ "command": command }).to_string(),
                "timeoutSecs": hook.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30),
                "blocking": hook.get("blocking").and_then(|v| v.as_bool()).unwrap_or(false),
                "toolPattern": hook.get("tool_pattern").and_then(|v| v.as_str()).unwrap_or(""),
            });

            let url = format!("{}/api/state/hooks", self.hub_url);
            let resp = reqwest::Client::new().post(&url).json(&payload).send().await
                .map_err(|e| MigrationError(e.to_string()))?;

            if resp.status().is_success() {
                count += 1;
                eprintln!("  ✓ Hook: {} → {}", event_type, truncate(command, 60));
            } else {
                eprintln!("  ✗ Hook {}: HTTP {}", event_type, resp.status());
            }
        }
        Ok(count)
    }
}

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub skills_imported: usize,
    pub agents_imported: usize,
    pub hooks_imported: usize,
    pub errors: Vec<String>,
}

impl std::fmt::Display for MigrationReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Migration complete:")?;
        writeln!(f, "  Skills:  {} imported", self.skills_imported)?;
        writeln!(f, "  Agents:  {} imported", self.agents_imported)?;
        writeln!(f, "  Hooks:   {} imported", self.hooks_imported)?;
        if !self.errors.is_empty() {
            writeln!(f, "  Errors:")?;
            for e in &self.errors {
                writeln!(f, "    - {}", e)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct MigrationError(pub String);
impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for MigrationError {}

/// Parse YAML frontmatter from markdown content.
fn parse_frontmatter(content: &str) -> (std::collections::HashMap<String, String>, String) {
    let mut map = std::collections::HashMap::new();
    if !content.starts_with("---") {
        return (map, content.to_string());
    }
    let rest = &content[3..];
    if let Some(end) = rest.find("---") {
        let fm = &rest[..end];
        let body = rest[end + 3..].trim().to_string();
        for line in fm.lines() {
            if let Some((key, val)) = line.split_once(':') {
                map.insert(key.trim().to_string(), val.trim().to_string());
            }
        }
        (map, body)
    } else {
        (map, content.to_string())
    }
}

fn camel_to_snake(s: &str) -> String {
    let mut result = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap_or(ch));
        } else {
            result.push(ch);
        }
    }
    result
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}...", &s[..max]) }
}
