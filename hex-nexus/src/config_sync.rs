//! Config sync: reads repo config files and pushes to SpacetimeDB on startup.
//!
//! TODO(T16): Add config change history tracking — store previous values with
//! timestamps so the dashboard can show a diff/audit log of config changes.

use std::path::Path;

/// Sync project config files to SpacetimeDB.
/// Called once during nexus startup after SpacetimeDB connection is established.
pub async fn sync_project_config(project_root: &Path, stdb_host: &str, stdb_db: &str) {
    let client = reqwest::Client::new();
    let now = chrono::Utc::now().to_rfc3339();

    // 1. Sync blueprint
    let blueprint_path = project_root.join(".hex/blueprint.json");
    if blueprint_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&blueprint_path) {
            let _ = call_reducer(
                &client,
                stdb_host,
                stdb_db,
                "sync_config",
                serde_json::json!([
                    "blueprint",
                    project_root
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    content,
                    ".hex/blueprint.json",
                    &now,
                ]),
            )
            .await;
            tracing::info!("Synced blueprint config");
        }
    }

    // 2. Sync MCP servers + hooks from .claude/settings.json
    let settings_path = project_root.join(".claude/settings.json");
    if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                // Sync MCP servers
                if let Some(mcp) = parsed.get("mcpServers") {
                    let _ = call_reducer(
                        &client,
                        stdb_host,
                        stdb_db,
                        "sync_config",
                        serde_json::json!([
                            "mcp_servers",
                            project_root
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            mcp.to_string(),
                            ".claude/settings.json",
                            &now,
                        ]),
                    )
                    .await;
                    tracing::info!("Synced MCP servers config");
                }
                // Sync hooks
                if let Some(hooks) = parsed.get("hooks") {
                    let _ = call_reducer(
                        &client,
                        stdb_host,
                        stdb_db,
                        "sync_config",
                        serde_json::json!([
                            "hooks",
                            project_root
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            hooks.to_string(),
                            ".claude/settings.json",
                            &now,
                        ]),
                    )
                    .await;
                    tracing::info!("Synced hooks config");
                }
            }
        }
    }

    // 3. Sync skills from .claude/skills/*.md
    let skills_dir = project_root.join(".claude/skills");
    if skills_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&skills_dir) {
            for entry in entries.flatten() {
                let filename = entry.file_name().to_string_lossy().to_string();
                if !filename.ends_with(".md") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    let (name, trigger, desc) = parse_skill_frontmatter(&content, &filename);
                    let skill_id = filename.trim_end_matches(".md").to_string();
                    let project_id = project_root
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let _ = call_reducer(
                        &client,
                        stdb_host,
                        stdb_db,
                        "sync_skill",
                        serde_json::json!([
                            skill_id,
                            project_id,
                            name,
                            trigger,
                            desc,
                            format!(".claude/skills/{}", filename),
                            &now,
                        ]),
                    )
                    .await;
                }
            }
            tracing::info!("Synced skills");
        }
    }

    // 4. Sync agent definitions from .claude/agents/*.yml
    let agents_dir = project_root.join(".claude/agents");
    if agents_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let filename = entry.file_name().to_string_lossy().to_string();
                if !filename.ends_with(".yml") && !filename.ends_with(".yaml") {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    let agent_id = filename
                        .split('.')
                        .next()
                        .unwrap_or(&filename)
                        .to_string();
                    let project_id = project_root
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let name = extract_yaml_field(&content, "name")
                        .unwrap_or_else(|| agent_id.clone());
                    let role = extract_yaml_field(&content, "role").unwrap_or_default();
                    let model = extract_yaml_field(&content, "model").unwrap_or_default();

                    let _ = call_reducer(
                        &client,
                        stdb_host,
                        stdb_db,
                        "sync_agent_def",
                        serde_json::json!([
                            agent_id,
                            project_id,
                            name,
                            role,
                            model,
                            "[]",
                            "[]",
                            format!(".claude/agents/{}", filename),
                            &now,
                        ]),
                    )
                    .await;
                }
            }
            tracing::info!("Synced agent definitions");
        }
    }
}

fn parse_skill_frontmatter(content: &str, filename: &str) -> (String, String, String) {
    let mut name = filename.trim_end_matches(".md").to_string();
    let mut trigger = format!("/{}", name);
    let mut desc = String::new();

    // Look for YAML frontmatter between ---
    if content.starts_with("---") {
        if let Some(end) = content[3..].find("---") {
            let fm = &content[3..3 + end];
            for line in fm.lines() {
                if let Some(val) = line.strip_prefix("name:") {
                    name = val.trim().to_string();
                }
                if let Some(val) = line.strip_prefix("trigger:") {
                    trigger = val.trim().to_string();
                }
                if let Some(val) = line.strip_prefix("description:") {
                    desc = val.trim().to_string();
                }
            }
        }
    }
    (name, trigger, desc)
}

fn extract_yaml_field(content: &str, field: &str) -> Option<String> {
    let prefix = format!("{}:", field);
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) {
            return Some(
                trimmed[prefix.len()..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    None
}

async fn call_reducer(
    client: &reqwest::Client,
    host: &str,
    db: &str,
    reducer: &str,
    args: serde_json::Value,
) -> Result<(), String> {
    let url = format!("{}/v1/database/{}/call/{}", host, db, reducer);
    match client.post(&url).json(&args).send().await {
        Ok(res) if res.status().is_success() => Ok(()),
        Ok(res) => Err(format!("{} returned {}", reducer, res.status())),
        Err(e) => Err(format!("{} failed: {}", reducer, e)),
    }
}
