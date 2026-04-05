//! SpacetimeDB-backed implementation of AgentLoaderPort.
//!
//! Two compilation modes:
//! 1. `spacetimedb` feature: Subscribes to the `agent_definition` table in
//!    SpacetimeDB. Maintains an in-memory DashMap cache that auto-refreshes
//!    via on_insert/on_delete callbacks. Falls back to REST if the connection
//!    fails.
//! 2. Default (no feature): Pure REST fallback against hex-hub HTTP API.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::ports::{AgentDefinition, AgentConstraints};
use crate::ports::agents::{AgentLoaderPort, AgentLoadError};

/// DTO for deserializing agent definitions from the REST API.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentDefDto {
    name: String,
    description: String,
    role_prompt: String,
    allowed_tools_json: String,
    constraints_json: String,
    model: String,
    max_turns: u32,
    metadata_json: String,
}

/// Convert a DTO (from REST or SpacetimeDB row) into a domain AgentDefinition.
fn dto_to_definition(dto: AgentDefDto) -> Result<AgentDefinition, AgentLoadError> {
    let allowed_tools: Vec<String> = serde_json::from_str(&dto.allowed_tools_json)
        .unwrap_or_default();

    let constraints: AgentConstraints = serde_json::from_str(&dto.constraints_json)
        .unwrap_or_default();

    let metadata: HashMap<String, String> = serde_json::from_str(&dto.metadata_json)
        .unwrap_or_default();

    Ok(AgentDefinition {
        name: dto.name,
        description: dto.description,
        role_prompt: dto.role_prompt,
        allowed_tools,
        constraints,
        model: if dto.model.is_empty() { None } else { Some(dto.model) },
        max_turns: dto.max_turns,
        metadata,
    })
}

/// Fetch agent definitions from the hex-hub REST API.
async fn fetch_from_hub(hub_url: &str) -> Result<HashMap<String, AgentDefinition>, AgentLoadError> {
    let url = format!("{}/api/state/agent-definitions", hub_url);
    let resp = reqwest::get(&url).await.map_err(|e| AgentLoadError::ReadError {
        path: url.clone(),
        reason: e.to_string(),
    })?;

    if !resp.status().is_success() {
        return Err(AgentLoadError::ReadError {
            path: url,
            reason: format!("HTTP {}", resp.status()),
        });
    }

    let entries: Vec<AgentDefDto> = resp.json().await.map_err(|e| AgentLoadError::ParseError {
        path: url,
        reason: e.to_string(),
    })?;

    let mut map = HashMap::new();
    for entry in entries {
        if let Ok(def) = dto_to_definition(entry) {
            map.insert(def.name.clone(), def);
        }
    }
    Ok(map)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// REST-only implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// NOTE(ADR-2604050900): The agent-definition-registry WASM module has been
// deleted. Agent definitions are loaded from local YAML files and synced via
// config_sync.rs. This adapter uses REST to fetch definitions from hex-hub.

mod inner {
    use super::*;
    use std::sync::RwLock;

    /// REST-only agent definition loader (SpacetimeDB feature not enabled).
    pub struct SpacetimeAgentLoader {
        cache: Arc<RwLock<HashMap<String, AgentDefinition>>>,
        hub_url: String,
    }

    impl SpacetimeAgentLoader {
        pub fn new(hub_url: &str) -> Self {
            Self {
                cache: Arc::new(RwLock::new(HashMap::new())),
                hub_url: hub_url.to_string(),
            }
        }

        pub async fn connect(&self, _host: &str, _database: &str) -> Result<(), AgentLoadError> {
            tracing::info!("SpacetimeDB feature not enabled, using REST fallback");
            self.fetch_from_hub().await
        }

        async fn fetch_from_hub(&self) -> Result<(), AgentLoadError> {
            let map = fetch_from_hub(&self.hub_url).await?;
            if let Ok(mut cache) = self.cache.write() {
                *cache = map;
            }
            Ok(())
        }
    }

    #[async_trait]
    impl AgentLoaderPort for SpacetimeAgentLoader {
        async fn load(&self, _dirs: &[&str]) -> Result<Vec<AgentDefinition>, AgentLoadError> {
            let cache = self.cache.read().map_err(|e| AgentLoadError::ReadError {
                path: "spacetimedb://agent_definition".into(),
                reason: format!("Cache lock poisoned: {}", e),
            })?;
            Ok(cache.values().cloned().collect())
        }

        async fn load_by_name(&self, _dirs: &[&str], name: &str) -> Result<AgentDefinition, AgentLoadError> {
            let cache = self.cache.read().map_err(|e| AgentLoadError::ReadError {
                path: "spacetimedb://agent_definition".into(),
                reason: format!("Cache lock poisoned: {}", e),
            })?;
            cache.get(name).cloned().ok_or_else(|| AgentLoadError::NotFound {
                name: name.to_string(),
                dirs: vec!["spacetimedb".to_string()],
            })
        }
    }
}

pub use inner::SpacetimeAgentLoader;
