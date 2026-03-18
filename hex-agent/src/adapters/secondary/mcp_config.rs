//! MCP configuration loader (ADR-033).
//!
//! Reads MCP server configurations from .claude/settings.json files.

use crate::ports::mcp_client::ServerConfig as McpServerConfig;
use std::path::Path;

/// Load MCP server configs from Claude Code settings files.
/// Merges project-level and user-level, with project taking precedence.
pub fn load_mcp_configs(project_dir: &Path) -> Vec<McpServerConfig> {
    let mut configs = Vec::new();

    // User-level (~/.claude/settings.json)
    let home = std::env::var("HOME").unwrap_or_default();
    let user_settings = Path::new(&home).join(".claude").join("settings.json");
    if let Some(user_configs) = read_mcp_servers(&user_settings) {
        configs.extend(user_configs);
    }

    // Project-level (overrides user)
    let project_settings = project_dir.join(".claude").join("settings.json");
    if let Some(project_configs) = read_mcp_servers(&project_settings) {
        // Project configs override user configs with same name
        for pc in project_configs {
            if let Some(pos) = configs.iter().position(|c| c.name == pc.name) {
                configs[pos] = pc;
            } else {
                configs.push(pc);
            }
        }
    }

    // Filter to stdio transport only (we don't support SSE yet)
    configs.retain(|c| c.transport == "stdio");

    configs
}

fn read_mcp_servers(path: &Path) -> Option<Vec<McpServerConfig>> {
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    let servers = parsed.get("mcpServers")?.as_object()?;

    let mut configs = Vec::new();
    for (name, config) in servers {
        let command = config.get("command")?.as_str()?.to_string();
        let args: Vec<String> = config
            .get("args")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let env: std::collections::HashMap<String, String> = config
            .get("env")
            .and_then(|e| serde_json::from_value(e.clone()).ok())
            .unwrap_or_default();
        let transport = config
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("stdio")
            .to_string();

        configs.push(McpServerConfig {
            name: name.clone(),
            command,
            args,
            env,
            transport,
        });
    }

    Some(configs)
}
