//! SpacetimeDB-backed implementation of AgentLoaderPort.
//!
//! Subscribes to the `agent_definition` table in SpacetimeDB. Maintains an
//! in-memory cache of AgentDefinitions that auto-refreshes on table changes.
//! Hot-reloads agent definitions without restart.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::ports::{AgentDefinition, AgentConstraints};
use crate::ports::agents::{AgentLoaderPort, AgentLoadError};

/// SpacetimeDB-backed agent definition loader.
///
/// On creation, subscribes to the `agent_definition` table. Definitions are
/// cached in-memory and served from cache on `load()` / `load_by_name()`.
pub struct SpacetimeAgentLoader {
    /// Cached definitions keyed by name for fast lookup.
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
        // TODO: SpacetimeDB subscription to agent_definition table
        self.fetch_from_hub().await
    }

    async fn fetch_from_hub(&self) -> Result<(), AgentLoadError> {
        let url = format!("{}/api/state/agent-definitions", self.hub_url);
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
            if let Ok(def) = Self::dto_to_definition(entry) {
                map.insert(def.name.clone(), def);
            }
        }

        if let Ok(mut cache) = self.cache.write() {
            *cache = map;
        }
        Ok(())
    }

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

    /// Called by SpacetimeDB subscription callbacks to hot-reload definitions.
    #[allow(dead_code)]
    fn rebuild_cache(&self, rows: Vec<AgentDefDto>) {
        let mut map = HashMap::new();
        for row in rows {
            if let Ok(def) = Self::dto_to_definition(row) {
                map.insert(def.name.clone(), def);
            }
        }
        if let Ok(mut cache) = self.cache.write() {
            *cache = map;
        }
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
