//! Role Hierarchy API — parses persona YAML files to build organizational structure.
//!
//! NOTE: This shows persona definitions (static role templates), not live agents.
//! For runtime agent state, see /api/hex-agents (agent_registry table).

use axum::{extract::State, http::StatusCode, response::Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub preferred: Option<String>,
    pub fallback: Option<String>,
    pub upgrade_threshold: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOrgNode {
    pub name: String,
    pub role: String,
    pub tier: String,
    pub reports_to: Option<String>,
    pub direct_reports: Vec<String>,
    pub communication: Option<CommunicationConfig>,
    pub model: Option<ModelConfig>,
    pub context_level: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaStatus {
    pub name: String,
    pub role: String,
    pub tier: String,
    pub status: String, // "online" | "idle" | "offline"
    pub last_heartbeat: Option<String>,
    pub active_agents: u32,
    pub reports_to: Option<String>,
    pub direct_reports: Vec<String>,
    pub communication: Option<CommunicationConfig>,
    pub model: Option<ModelConfig>,
    pub context_level: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PersonaStatusResponse {
    pub personas: Vec<PersonaStatus>,
    pub root: String,
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

/// GET /api/org/personas
///
/// Returns org chart enriched with live agent heartbeat status.
/// Shows which personas currently have online agents.
pub async fn get_persona_status(
    State(state): State<Arc<crate::state::AppState>>,
) -> Result<Json<PersonaStatusResponse>, StatusCode> {
    // Get static persona definitions
    let personas_static = parse_agent_yamls().map_err(|e| {
        tracing::error!(error = %e, "Failed to parse agent YAMLs");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Get live agent data from SpacetimeDB
    let port = state.state_port.as_deref().ok_or_else(|| {
        tracing::error!("State port not available");
        StatusCode::SERVICE_UNAVAILABLE
    })?;

    let live_agents = port
        .hex_agent_list()
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Failed to query live agents");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Build persona -> live agent mapping
    let mut persona_status_map: std::collections::HashMap<String, (String, Option<String>, u32)> =
        std::collections::HashMap::new();

    for agent_json in live_agents {
        // Extract role from agent.role field or parse from agent.name ("hex-agent-ceo" -> "ceo")
        let mut role = agent_json
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if role.is_empty() {
            // Try extracting from name like "hex-agent-ceo"
            if let Some(name) = agent_json.get("name").and_then(|v| v.as_str()) {
                if let Some(suffix) = name.strip_prefix("hex-agent-") {
                    role = suffix.to_string();
                }
            }
        }

        // If role is empty, try to extract it from the agent name
        // Names like "hex-coder-bazzite.lan" or "hex-agent-ceo" map to persona "hex-coder" or "ceo"
        if role.is_empty() {
            let name = agent_json
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Extract role from name patterns:
            // "hex-coder-bazzite.lan" -> "hex-coder"
            // "hex-agent-ceo" -> "ceo"
            // "ceo-bazzite.lan" -> "ceo"
            if let Some(extracted) = name.split('-').next() {
                if extracted.starts_with("hex") && name.contains('-') {
                    // "hex-coder-..." -> "hex-coder"
                    if let Some(second) = name.split('-').nth(1) {
                        if !second.is_empty() && second != "agent" {
                            role = format!("{}-{}", extracted, second);
                        }
                    }
                } else if !extracted.is_empty() {
                    // "ceo-..." -> "ceo"
                    role = extracted.to_string();
                }
            }

            // Try "hex-agent-{role}" pattern
            if role.is_empty() && name.starts_with("hex-agent-") {
                if let Some(suffix) = name.strip_prefix("hex-agent-") {
                    role = suffix.to_string();
                }
            }
        }

        if role.is_empty() {
            continue; // Skip unassigned agents
        }

        let entry = persona_status_map.entry(role.clone()).or_insert((
            "offline".to_string(),
            None,
            0,
        ));

        entry.2 += 1; // Increment agent count

        // Mark persona as online if it has any registered agents (even if stale)
        // This ensures the dashboard shows all available personas
        entry.0 = "online".to_string();

        // Update heartbeat if this agent is more recent
        let agent_status = agent_json
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("offline");

        if agent_status == "online" || agent_status == "idle" || agent_status == "stale" {

            // Keep most recent heartbeat
            let heartbeat = agent_json
                .get("last_heartbeat")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if heartbeat.is_some() &&
               (entry.1.is_none() || heartbeat > entry.1) {
                entry.1 = heartbeat;
            }
        }
    }

    // Merge static persona data with live status
    let mut personas: Vec<PersonaStatus> = personas_static
        .iter()
        .map(|p| {
            let (status, last_heartbeat, active_count) = persona_status_map
                .get(&p.name)
                .cloned()
                .unwrap_or_else(|| ("offline".to_string(), None, 0));

            PersonaStatus {
                name: p.name.clone(),
                role: p.role.clone(),
                tier: p.tier.clone(),
                status,
                last_heartbeat,
                active_agents: active_count,
                reports_to: p.reports_to.clone(),
                direct_reports: p.direct_reports.clone(),
                communication: p.communication.clone(),
                model: p.model.clone(),
                context_level: p.context_level.clone(),
            }
        })
        .collect();

    // Find root
    let root = personas
        .iter()
        .find(|p| p.reports_to.is_none())
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "ceo".to_string());

    Ok(Json(PersonaStatusResponse { personas, root }))
}

pub fn parse_agent_yamls() -> Result<Vec<AgentOrgNode>, String> {
    // Try multiple possible paths for agent YAMLs
    let mut possible_paths = vec![];

    // 1. Current working directory + relative path
    if let Ok(cwd) = std::env::current_dir() {
        possible_paths.push(cwd.join("hex-cli/assets/agents/hex/hex"));

        // Handle /var/home vs /home symlink on some systems
        let cwd_str = cwd.to_string_lossy();
        if cwd_str.starts_with("/var/home/") {
            let alt = std::path::PathBuf::from(cwd_str.replace("/var/home/", "/home/"));
            possible_paths.push(alt.join("hex-cli/assets/agents/hex/hex"));
        } else if cwd_str.starts_with("/home/") {
            let alt = std::path::PathBuf::from(cwd_str.replace("/home/", "/var/home/"));
            possible_paths.push(alt.join("hex-cli/assets/agents/hex/hex"));
        }
    }

    // 2. HEX_PROJECT_ROOT env var
    if let Ok(root) = std::env::var("HEX_PROJECT_ROOT") {
        possible_paths.push(std::path::PathBuf::from(&root).join("hex-cli/assets/agents/hex/hex"));

        // Handle symlink variants
        if root.starts_with("/var/home/") {
            let alt = root.replace("/var/home/", "/home/");
            possible_paths.push(std::path::PathBuf::from(alt).join("hex-cli/assets/agents/hex/hex"));
        } else if root.starts_with("/home/") {
            let alt = root.replace("/home/", "/var/home/");
            possible_paths.push(std::path::PathBuf::from(alt).join("hex-cli/assets/agents/hex/hex"));
        }
    }

    // 3. Relative path (when run from project root)
    possible_paths.push(std::path::PathBuf::from("hex-cli/assets/agents/hex/hex"));

    for path in possible_paths.iter() {
        if path.exists() {
            tracing::debug!(path = ?path, "Found agent YAML directory");
            return parse_from_filesystem(path);
        }
    }

    // TODO: Parse from rust-embed assets when deployed
    Err(format!(
        "Agent YAML directory not found. Tried: {}",
        possible_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
    ))
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

        let model = if let Some(model_val) = yaml.get("model") {
            Some(ModelConfig {
                preferred: model_val.get("preferred")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                fallback: model_val.get("fallback")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                upgrade_threshold: model_val.get("upgrade_threshold")
                    .and_then(|v| v.as_f64()),
            })
        } else {
            None
        };

        let context_level = yaml.get("context_level")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        nodes.push(AgentOrgNode {
            name,
            role,
            tier,
            reports_to,
            direct_reports,
            communication,
            model,
            context_level,
        });
    }

    Ok(nodes)
}
