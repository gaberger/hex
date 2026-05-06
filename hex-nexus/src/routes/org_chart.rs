//! Role Hierarchy API — parses persona YAML files to build organizational structure.
//!
//! NOTE: This shows persona definitions (static role templates), not live agents.
//! For runtime agent state, see /api/hex-agents (agent_registry table).

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOrgNode {
    pub name: String,
    pub role: String,
    pub tier: String,
    pub reports_to: Option<String>,
    pub direct_reports: Vec<String>,
    pub communication: Option<CommunicationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationConfig {
    pub channels: Vec<String>,
    pub peers: Vec<String>,
    pub can_dm: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct OrgChartResponse {
    pub nodes: Vec<AgentOrgNode>,
    pub root: String, // CEO or top-level node
}

/// GET /api/org/chart
///
/// Returns hierarchical persona organization parsed from YAML files.
/// These are static role definitions, not live agent instances.
pub async fn get_org_chart(
    State(_state): State<Arc<crate::state::AppState>>,
) -> Result<Json<OrgChartResponse>, StatusCode> {
    // Read agent YAMLs from embedded assets or filesystem
    let agents = parse_agent_yamls().map_err(|e| {
        tracing::error!(error = %e, "Failed to parse agent YAMLs");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Find root node (CEO or agent with no reports_to)
    let root = agents
        .iter()
        .find(|a| a.reports_to.is_none())
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "ceo".to_string());

    Ok(Json(OrgChartResponse { nodes: agents, root }))
}

fn parse_agent_yamls() -> Result<Vec<AgentOrgNode>, String> {
    // Check if running in development mode with filesystem access
    let agent_dir = std::path::Path::new("hex-cli/assets/agents/hex/hex");

    if agent_dir.exists() {
        parse_from_filesystem(agent_dir)
    } else {
        // TODO: Parse from rust-embed assets when deployed
        Err("Agent YAML parsing from embedded assets not yet implemented".to_string())
    }
}

fn parse_from_filesystem(dir: &std::path::Path) -> Result<Vec<AgentOrgNode>, String> {
    use std::fs;

    let mut nodes = Vec::new();

    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("yml") {
            continue;
        }

        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;

        // Parse YAML
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse {:?}: {}", path, e))?;

        // Extract org-relevant fields
        let name = yaml["name"]
            .as_str()
            .ok_or_else(|| format!("Missing 'name' in {:?}", path))?
            .to_string();

        let role = yaml["role"]
            .as_str()
            .unwrap_or("Unknown")
            .to_string();

        let tier = yaml["tier"]
            .as_str()
            .unwrap_or("ic")
            .to_string();

        let reports_to = yaml["reports_to"]
            .as_str()
            .map(|s| s.to_string());

        let direct_reports = yaml["direct_reports"]
            .as_sequence()
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let communication = if let Some(comm) = yaml.get("communication") {
            Some(CommunicationConfig {
                channels: comm["channels"]
                    .as_sequence()
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                peers: comm["peers"]
                    .as_sequence()
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                can_dm: comm["can_dm"]
                    .as_sequence()
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        } else {
            None
        };

        nodes.push(AgentOrgNode {
            name,
            role,
            tier,
            reports_to,
            direct_reports,
            communication,
        });
    }

    Ok(nodes)
}
