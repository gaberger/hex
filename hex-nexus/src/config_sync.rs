//! Config sync: reads repo config files and pushes to SpacetimeDB on startup.
//!
//! Also handles project auto-registration from `.hex/project.yaml` (ADR-043).
//!
//! TODO(T16): Add config change history tracking — store previous values with
//! timestamps so the dashboard can show a diff/audit log of config changes.

use std::path::Path;

/// Report of a config sync operation (T7: enhanced reporting).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncReport {
    /// Number of config items successfully synced.
    pub synced: usize,
    /// Number of config items that failed to sync.
    pub failed: usize,
    /// Per-item details (category → status).
    pub items: Vec<SyncItem>,
    /// Timestamp of this sync run.
    pub timestamp: String,
}

/// A single config item sync result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncItem {
    pub category: String,
    pub status: String, // "ok" or "failed"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SyncReport {
    fn new() -> Self {
        Self {
            synced: 0,
            failed: 0,
            items: Vec::new(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn record_ok(&mut self, category: &str) {
        self.synced += 1;
        self.items.push(SyncItem {
            category: category.to_string(),
            status: "ok".to_string(),
            error: None,
        });
    }

    fn record_fail(&mut self, category: &str, error: String) {
        self.failed += 1;
        self.items.push(SyncItem {
            category: category.to_string(),
            status: "failed".to_string(),
            error: Some(error),
        });
    }
}

// ── Project Manifest (ADR-043) ──────────────────────────

/// Parsed `.hex/project.yaml` manifest for auto-registration.
#[derive(Debug, serde::Deserialize)]
pub struct ProjectManifest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub auto_register: bool,
}

/// Read and parse `.hex/project.yaml` from the given project root.
pub fn read_project_manifest(project_root: &Path) -> Option<ProjectManifest> {
    let manifest_path = project_root.join(".hex/project.yaml");
    if !manifest_path.exists() {
        return None;
    }
    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match serde_yaml::from_str::<ProjectManifest>(&content) {
            Ok(manifest) => {
                tracing::debug!(name = %manifest.name, "Parsed project manifest");
                Some(manifest)
            }
            Err(e) => {
                tracing::warn!(path = %manifest_path.display(), "Failed to parse project.yaml: {}", e);
                None
            }
        },
        Err(e) => {
            tracing::warn!(path = %manifest_path.display(), "Failed to read project.yaml: {}", e);
            None
        }
    }
}

/// Auto-register the project in SpacetimeDB if `.hex/project.yaml` has `auto_register: true`.
pub async fn auto_register_project(project_root: &Path, stdb_host: &str, stdb_db: &str) {
    let manifest = match read_project_manifest(project_root) {
        Some(m) => m,
        None => return,
    };

    if !manifest.auto_register {
        tracing::debug!(name = %manifest.name, "Project manifest found but auto_register is false");
        return;
    }

    let root_path = project_root.to_string_lossy().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let project_id = crate::state::make_project_id(&root_path);

    let client = reqwest::Client::new();
    match call_reducer(
        &client,
        stdb_host,
        stdb_db,
        "register_project",
        serde_json::json!([
            project_id,
            manifest.name,
            manifest.description,
            root_path,
            now,
            false,
        ]),
    )
    .await
    {
        Ok(()) => {
            tracing::info!(
                name = %manifest.name,
                project_id = %project_id,
                "Auto-registered project in SpacetimeDB (ADR-043)"
            );
        }
        Err(e) => {
            tracing::warn!(
                name = %manifest.name,
                "Failed to auto-register project: {} (SpacetimeDB may not be running)",
                e
            );
        }
    }
}

/// Sync project config files to SpacetimeDB with detailed reporting.
///
/// Returns a [`SyncReport`] with per-category success/failure status.
/// This is used by the hydration pipeline (T7) to verify config sync completed.
pub async fn sync_project_config_with_report(
    project_root: &Path,
    stdb_host: &str,
    stdb_db: &str,
) -> SyncReport {
    let mut report = SyncReport::new();
    let client = reqwest::Client::new();
    let now = chrono::Utc::now().to_rfc3339();

    // 1. Blueprint
    let blueprint_path = project_root.join(".hex/blueprint.json");
    if blueprint_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&blueprint_path) {
            match call_reducer(
                &client,
                stdb_host,
                stdb_db,
                "sync_config",
                serde_json::json!([
                    "blueprint",
                    project_root.file_name().unwrap_or_default().to_string_lossy(),
                    content,
                    ".hex/blueprint.json",
                    &now,
                ]),
            )
            .await
            {
                Ok(()) => report.record_ok("blueprint"),
                Err(e) => report.record_fail("blueprint", e),
            }
        }
    }

    // 2. Settings (MCP servers + hooks)
    let settings_path = project_root.join(".claude/settings.json");
    if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(mcp) = parsed.get("mcpServers") {
                    match call_reducer(
                        &client, stdb_host, stdb_db, "sync_config",
                        serde_json::json!([
                            "mcp_servers",
                            project_root.file_name().unwrap_or_default().to_string_lossy(),
                            mcp.to_string(),
                            ".claude/settings.json",
                            &now,
                        ]),
                    ).await {
                        Ok(()) => report.record_ok("mcp_servers"),
                        Err(e) => report.record_fail("mcp_servers", e),
                    }
                }
                if let Some(hooks) = parsed.get("hooks") {
                    match call_reducer(
                        &client, stdb_host, stdb_db, "sync_config",
                        serde_json::json!([
                            "hooks",
                            project_root.file_name().unwrap_or_default().to_string_lossy(),
                            hooks.to_string(),
                            ".claude/settings.json",
                            &now,
                        ]),
                    ).await {
                        Ok(()) => report.record_ok("hooks"),
                        Err(e) => report.record_fail("hooks", e),
                    }
                }
            }
        }
    }

    // 3. Skills + agents + MCP tools (count-based)
    let global_skills_dir = project_root.join("skills");
    if global_skills_dir.is_dir() {
        let mut skill_count = 0usize;
        if let Ok(entries) = std::fs::read_dir(&global_skills_dir) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() { continue; }
                let dir_name = entry.file_name().to_string_lossy().to_string();
                let skill_md = entry.path().join("SKILL.md");
                if !skill_md.exists() { continue; }
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    let (name, trigger, desc) = parse_skill_frontmatter(&content, &dir_name);
                    let project_id = project_root.file_name().unwrap_or_default().to_string_lossy().to_string();
                    if call_reducer(
                        &client, stdb_host, stdb_db, "sync_skill",
                        serde_json::json!([&dir_name, project_id, name, trigger, desc, format!("skills/{}/SKILL.md", dir_name), &now]),
                    ).await.is_ok() {
                        skill_count += 1;
                    }
                }
            }
        }
        if skill_count > 0 {
            report.record_ok(&format!("skills({})", skill_count));
        }
    }

    // 4. MCP tool definitions from config/mcp-tools.json
    let tools_path = project_root.join("config/mcp-tools.json");
    if tools_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&tools_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                let version = parsed
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0.0.0")
                    .to_string();

                if let Some(tools) = parsed.get("tools").and_then(|t| t.as_array()) {
                    let mut tool_count = 0usize;
                    for tool in tools {
                        let name = tool["name"].as_str().unwrap_or_default().to_string();
                        if name.is_empty() {
                            continue;
                        }
                        let category = tool["category"].as_str().unwrap_or("").to_string();
                        let description =
                            tool["description"].as_str().unwrap_or("").to_string();
                        let route_method =
                            tool["route"]["method"].as_str().unwrap_or("").to_string();
                        let route_path =
                            tool["route"]["path"].as_str().unwrap_or("").to_string();
                        let input_schema = tool
                            .get("inputSchema")
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "{}".to_string());

                        if call_reducer(
                            &client,
                            stdb_host,
                            stdb_db,
                            "mcp_tool_sync",
                            serde_json::json!([
                                name,
                                category,
                                description,
                                route_method,
                                route_path,
                                input_schema,
                                &version,
                                &now,
                            ]),
                        )
                        .await
                        .is_ok()
                        {
                            tool_count += 1;
                        }
                    }
                    if tool_count > 0 {
                        report.record_ok(&format!("mcp_tools({})", tool_count));
                    } else if !tools.is_empty() {
                        report.record_fail(
                            "mcp_tools",
                            "All tool syncs failed".to_string(),
                        );
                    }
                }
            }
        }
    }

    report
}

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

    // 3a. Sync global skills from skills/*/SKILL.md (repo catalog)
    let global_skills_dir = project_root.join("skills");
    if global_skills_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&global_skills_dir) {
            for entry in entries.flatten() {
                if !entry.path().is_dir() { continue; }
                let dir_name = entry.file_name().to_string_lossy().to_string();
                let skill_md = entry.path().join("SKILL.md");
                if !skill_md.exists() { continue; }
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    let (name, trigger, desc) = parse_skill_frontmatter(&content, &dir_name);
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
                            &dir_name,
                            project_id,
                            name,
                            trigger,
                            desc,
                            format!("skills/{}/SKILL.md", dir_name),
                            &now,
                        ]),
                    )
                    .await;
                }
            }
            tracing::info!("Synced global skills catalog");
        }
    }

    // 3b. Sync project skills from .claude/skills/*/SKILL.md and .claude/skills/*.md
    let project_skills_dir = project_root.join(".claude/skills");
    if project_skills_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&project_skills_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let filename = entry.file_name().to_string_lossy().to_string();
                let project_id = project_root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                if path.is_dir() {
                    // Subdirectory with SKILL.md inside
                    let skill_md = path.join("SKILL.md");
                    if !skill_md.exists() { continue; }
                    if let Ok(content) = std::fs::read_to_string(&skill_md) {
                        let (name, trigger, desc) = parse_skill_frontmatter(&content, &filename);
                        let skill_id = format!("{}-project", filename);
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
                                format!(".claude/skills/{}/SKILL.md", filename),
                                &now,
                            ]),
                        )
                        .await;
                    }
                } else if filename.ends_with(".md") {
                    // Standalone .md file
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let (name, trigger, desc) = parse_skill_frontmatter(&content, &filename);
                        let skill_id = format!("{}-project", filename.trim_end_matches(".md"));
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
            }
            tracing::info!("Synced project skills");
        }
    }

    // 4. Sync MCP tool definitions from config/mcp-tools.json
    let tools_path = project_root.join("config/mcp-tools.json");
    if tools_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&tools_path) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                let version = parsed
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("0.0.0")
                    .to_string();

                if let Some(tools) = parsed.get("tools").and_then(|t| t.as_array()) {
                    for tool in tools {
                        let name = tool["name"].as_str().unwrap_or_default().to_string();
                        if name.is_empty() {
                            continue;
                        }
                        let category = tool["category"].as_str().unwrap_or("").to_string();
                        let description = tool["description"].as_str().unwrap_or("").to_string();
                        let route_method = tool["route"]["method"].as_str().unwrap_or("").to_string();
                        let route_path = tool["route"]["path"].as_str().unwrap_or("").to_string();
                        let input_schema = tool
                            .get("inputSchema")
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "{}".to_string());

                        let _ = call_reducer(
                            &client,
                            stdb_host,
                            stdb_db,
                            "mcp_tool_sync",
                            serde_json::json!([
                                name,
                                category,
                                description,
                                route_method,
                                route_path,
                                input_schema,
                                &version,
                                &now,
                            ]),
                        )
                        .await;
                    }
                    tracing::info!("Synced {} MCP tool definitions", tools.len());
                }
            }
        }
    }

    // 5. Sync agent definitions from .claude/agents/*.yml
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

/// After config sync completes, notify all project agents if config changed (ADR-060).
/// This allows running agents to pick up config changes (new skills, hooks, MCP tools)
/// without restarting their session.
pub async fn notify_config_change(
    state_port: &dyn crate::ports::state::IStatePort,
    project_id: &str,
    report: &SyncReport,
) {
    if report.synced == 0 {
        return; // Nothing changed — no notification needed
    }

    let changed_categories: Vec<&str> = report
        .items
        .iter()
        .filter(|i| i.status == "ok")
        .map(|i| i.category.as_str())
        .collect();

    let payload = serde_json::json!({
        "reason": "config_sync",
        "changed": changed_categories,
        "synced_count": report.synced,
    })
    .to_string();

    if let Err(e) = state_port
        .inbox_notify_all(project_id, 1, "config_change", &payload)
        .await
    {
        tracing::warn!("Failed to notify agents of config change: {}", e);
    } else {
        tracing::info!(
            "Notified project agents of config change ({} items synced)",
            report.synced
        );
    }
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
