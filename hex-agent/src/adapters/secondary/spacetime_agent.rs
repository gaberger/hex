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
// Feature-gated implementation (real SpacetimeDB SDK)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(feature = "spacetimedb")]
mod real {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::RwLock;

    // Generated client bindings for the agent-definition-registry module.
    use hex_hub_core::spacetime_bindings::agent_definition_registry::{
        AgentDefinition as StdbAgentDef,
        AgentDefinitionTableAccess,
        DbConnection,
    };
    use spacetimedb_sdk::{DbContext, Table};

    /// Convert a SpacetimeDB row into our DTO, then into domain type.
    fn stdb_row_to_definition(row: &StdbAgentDef) -> Option<AgentDefinition> {
        let dto = AgentDefDto {
            name: row.name.clone(),
            description: row.description.clone(),
            role_prompt: row.role_prompt.clone(),
            allowed_tools_json: row.allowed_tools_json.clone(),
            constraints_json: row.constraints_json.clone(),
            model: row.model.clone(),
            max_turns: row.max_turns,
            metadata_json: row.metadata_json.clone(),
        };
        dto_to_definition(dto).ok()
    }

    /// SpacetimeDB-backed agent definition loader.
    ///
    /// On creation, subscribes to the `agent_definition` table. Definitions are
    /// cached in-memory via DashMap and served from cache on `load()` / `load_by_name()`.
    /// If SpacetimeDB connection fails, falls back to REST.
    pub struct SpacetimeAgentLoader {
        /// Cached definitions keyed by name for fast lock-free lookup.
        cache: Arc<dashmap::DashMap<String, AgentDefinition>>,
        /// Whether SpacetimeDB subscription is active and has applied initial rows.
        subscribed: Arc<AtomicBool>,
        /// The SpacetimeDB connection handle (None until connect() succeeds).
        connection: Arc<RwLock<Option<DbConnection>>>,
        hub_url: String,
    }

    impl SpacetimeAgentLoader {
        pub fn new(hub_url: &str) -> Self {
            Self {
                cache: Arc::new(dashmap::DashMap::new()),
                subscribed: Arc::new(AtomicBool::new(false)),
                connection: Arc::new(RwLock::new(None)),
                hub_url: hub_url.to_string(),
            }
        }

        /// Connect to SpacetimeDB and subscribe to agent_definition table.
        ///
        /// If connection fails, falls back to REST and logs a warning.
        pub async fn connect(&self, host: &str, database: &str) -> Result<(), AgentLoadError> {
            // Guard: empty URI causes SDK panic — fall back to REST
            if host.is_empty() || database.is_empty() {
                tracing::info!("SpacetimeDB agent loader: no host/database configured, using REST fallback");
                return Ok(());
            }

            let cache = self.cache.clone();
            let subscribed = self.subscribed.clone();

            // Clones for the on_insert callback
            let cache_insert = cache.clone();
            // Clone for the on_delete callback
            let cache_delete = cache.clone();

            // Clone for the on_applied callback
            let subscribed_applied = subscribed.clone();

            match DbConnection::builder()
                .with_uri(host)
                .with_database_name(database)
                .on_connect(move |conn, _identity, _token| {
                    // Register table callbacks before subscribing
                    conn.db().agent_definition().on_insert(move |_ctx, row| {
                        if let Some(def) = stdb_row_to_definition(row) {
                            tracing::debug!(name = %def.name, "SpacetimeDB agent_definition inserted");
                            cache_insert.insert(def.name.clone(), def);
                        }
                    });

                    conn.db().agent_definition().on_delete(move |_ctx, row| {
                        tracing::debug!(name = %row.name, "SpacetimeDB agent_definition deleted");
                        cache_delete.remove(&row.name);
                    });

                    // Subscribe to the agent_definition table
                    conn.subscription_builder()
                        .on_applied(move |_ctx| {
                            tracing::info!("SpacetimeDB agent_definition subscription applied");
                            subscribed_applied.store(true, Ordering::Release);
                        })
                        .on_error(|_ctx, err| {
                            tracing::error!(?err, "SpacetimeDB agent_definition subscription error");
                        })
                        .subscribe(["SELECT * FROM agent_definition"]);
                })
                .on_connect_error(|_ctx, err| {
                    tracing::warn!(?err, "SpacetimeDB agent loader connection error");
                })
                .on_disconnect(|_ctx, err| {
                    if let Some(e) = err {
                        tracing::warn!(?e, "SpacetimeDB agent loader disconnected with error");
                    } else {
                        tracing::info!("SpacetimeDB agent loader disconnected cleanly");
                    }
                })
                .build()
            {
                Ok(conn) => {
                    // Spawn a background thread to process WebSocket messages
                    conn.run_threaded();

                    // Wait briefly for initial subscription to apply
                    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
                    while tokio::time::Instant::now() < deadline {
                        if subscribed.load(Ordering::Acquire) {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    }

                    if subscribed.load(Ordering::Acquire) {
                        tracing::info!(
                            count = self.cache.len(),
                            "SpacetimeDB agent loader connected, {} definitions cached",
                            self.cache.len()
                        );
                        *self.connection.write().await = Some(conn);
                        return Ok(());
                    }

                    // Subscription didn't apply in time — store connection anyway
                    // (it may apply later), but also seed from REST
                    tracing::warn!(
                        "SpacetimeDB subscription did not apply within 5s, seeding from REST"
                    );
                    *self.connection.write().await = Some(conn);
                    self.seed_from_rest().await
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "SpacetimeDB connection failed, falling back to REST"
                    );
                    self.seed_from_rest().await
                }
            }
        }

        /// Seed the cache from REST as a fallback.
        async fn seed_from_rest(&self) -> Result<(), AgentLoadError> {
            let map = fetch_from_hub(&self.hub_url).await?;
            for (name, def) in map {
                self.cache.insert(name, def);
            }
            Ok(())
        }
    }

    #[async_trait]
    impl AgentLoaderPort for SpacetimeAgentLoader {
        async fn load(&self, _dirs: &[&str]) -> Result<Vec<AgentDefinition>, AgentLoadError> {
            Ok(self.cache.iter().map(|entry| entry.value().clone()).collect())
        }

        async fn load_by_name(&self, _dirs: &[&str], name: &str) -> Result<AgentDefinition, AgentLoadError> {
            self.cache.get(name).map(|entry| entry.value().clone()).ok_or_else(|| AgentLoadError::NotFound {
                name: name.to_string(),
                dirs: vec!["spacetimedb".to_string()],
            })
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Stub implementation (no SpacetimeDB SDK — REST only)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(not(feature = "spacetimedb"))]
mod stub {
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

#[cfg(feature = "spacetimedb")]
pub use real::SpacetimeAgentLoader;
#[cfg(not(feature = "spacetimedb"))]
pub use stub::SpacetimeAgentLoader;
