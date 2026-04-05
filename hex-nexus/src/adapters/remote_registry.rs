//! SpacetimeDB-primary implementation of IRemoteRegistryPort (ADR-040, P4).
//!
//! SpacetimeDB (hexflo-coordination module, database "hex") is the
//! authoritative source of remote agent state.  A local HashMap acts as a
//! write-through cache for low-latency reads when SpacetimeDB is unreachable.
//!
//! Mutations write to both HashMap and SpacetimeDB.  Reads try SpacetimeDB
//! first (via SQL query), falling back to the HashMap cache on failure.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use async_trait::async_trait;
use serde_json::Value;
use tracing;

use crate::ports::remote_registry::IRemoteRegistryPort;
use crate::remote::transport::{
    AgentCapabilities, InferenceServer, InferenceServerStatus, RemoteAgent, RemoteAgentStatus,
    TransportError,
};

pub struct RemoteRegistryAdapter {
    agents: Arc<RwLock<HashMap<String, RemoteAgent>>>,
    servers: Arc<RwLock<HashMap<String, InferenceServer>>>,
    /// Base URL for SpacetimeDB HTTP API (e.g. "http://127.0.0.1:3033").
    stdb_url: String,
    /// HTTP client shared across fire-and-forget reducer calls.
    http: reqwest::Client,
}

impl Default for RemoteRegistryAdapter {
    fn default() -> Self {
        Self::new(None)
    }
}

impl RemoteRegistryAdapter {
    /// Create a new adapter.  When `stdb_url` is `None` the default
    /// `http://127.0.0.1:3033` is used.
    pub fn new(stdb_url: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");

        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            servers: Arc::new(RwLock::new(HashMap::new())),
            stdb_url: stdb_url.unwrap_or_else(|| "http://127.0.0.1:3033".to_string()),
            http,
        }
    }

    /// Execute a SQL query against SpacetimeDB and return rows.
    /// Returns `None` if SpacetimeDB is unreachable (caller falls back to cache).
    async fn sql_query(&self, query: &str) -> Option<Vec<Value>> {
        let url = format!(
            "{}/v1/database/{}/sql",
            self.stdb_url,
            hex_core::STDB_DATABASE_CORE,
        );
        let resp = self
            .http
            .post(&url)
            .body(query.to_string())
            .header("Content-Type", "text/plain")
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            tracing::warn!(
                status = %resp.status(),
                "SpacetimeDB SQL query returned non-success"
            );
            return None;
        }

        let body = resp.text().await.ok()?;
        let parsed: Value = serde_json::from_str(&body).ok()?;
        parsed
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|table| table.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
    }

    /// Parse a SpacetimeDB row (array of columns) into a `RemoteAgent`.
    /// Column order matches `register_remote_agent` reducer args:
    ///   [agent_id, name, host, project_dir, capabilities_json, tunnel_id, connected_at, status, last_heartbeat]
    fn parse_agent_row(row: &Value) -> Option<RemoteAgent> {
        let cols = row.as_array()?;
        let str_col = |i: usize| -> String {
            cols.get(i)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };

        let capabilities: AgentCapabilities =
            serde_json::from_str(&str_col(4)).unwrap_or_default();
        let tunnel_id = {
            let t = str_col(5);
            if t.is_empty() { None } else { Some(t) }
        };
        let status: RemoteAgentStatus =
            serde_json::from_str(&format!("\"{}\"", str_col(7))).unwrap_or(RemoteAgentStatus::Online);

        Some(RemoteAgent {
            agent_id: str_col(0),
            name: str_col(1),
            host: str_col(2),
            project_dir: str_col(3),
            capabilities,
            last_heartbeat: str_col(8),
            connected_at: str_col(6),
            tunnel_id,
            status,
        })
    }

    /// Fire-and-forget call to a SpacetimeDB reducer on the "hex" database.
    /// Failures are logged as warnings but never propagated — the HashMap is
    /// the write-through cache for the local process.
    fn spawn_reducer_call(&self, reducer: &str, args: serde_json::Value) {
        let url = format!(
            "{}/v1/database/{}/call/{}",
            self.stdb_url,
            hex_core::STDB_DATABASE_CORE,
            reducer,
        );
        let client = self.http.clone();
        let reducer_name = reducer.to_string();
        tokio::spawn(async move {
            match client.post(&url).json(&args).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(reducer = %reducer_name, "SpacetimeDB reducer call succeeded");
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!(
                        reducer = %reducer_name,
                        %status,
                        body = %body.chars().take(300).collect::<String>(),
                        "SpacetimeDB reducer call returned non-success"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        reducer = %reducer_name,
                        error = %e,
                        "SpacetimeDB reducer call failed (fire-and-forget)"
                    );
                }
            }
        });
    }
}

#[async_trait]
impl IRemoteRegistryPort for RemoteRegistryAdapter {
    async fn register_agent(&self, agent: RemoteAgent) -> Result<(), TransportError> {
        let id = agent.agent_id.clone();
        tracing::info!(agent_id = %id, name = %agent.name, "registering remote agent");

        // Fire-and-forget: replicate to SpacetimeDB for cross-host visibility
        let capabilities_json = serde_json::to_string(&agent.capabilities).unwrap_or_default();
        let tunnel_id = agent.tunnel_id.clone().unwrap_or_default();
        self.spawn_reducer_call(
            "register_remote_agent",
            serde_json::json!([
                agent.agent_id,
                agent.name,
                agent.host,
                agent.project_dir,
                capabilities_json,
                tunnel_id,
                agent.connected_at,
            ]),
        );

        self.agents.write().await.insert(id, agent);
        Ok(())
    }

    async fn update_agent_status(
        &self,
        agent_id: &str,
        status: RemoteAgentStatus,
    ) -> Result<(), TransportError> {
        let mut agents = self.agents.write().await;
        let agent = agents.get_mut(agent_id).ok_or_else(|| {
            TransportError::Protocol(format!("agent not found: {agent_id}"))
        })?;
        tracing::info!(agent_id = %agent_id, ?status, "updating agent status");
        agent.status = status.clone();

        // Fire-and-forget: replicate status to SpacetimeDB
        let status_str = serde_json::to_value(&status)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| format!("{:?}", status).to_lowercase());
        self.spawn_reducer_call(
            "update_remote_status",
            serde_json::json!([agent_id, status_str]),
        );

        Ok(())
    }

    async fn heartbeat(&self, agent_id: &str) -> Result<(), TransportError> {
        let mut agents = self.agents.write().await;
        let agent = agents.get_mut(agent_id).ok_or_else(|| {
            TransportError::Protocol(format!("agent not found: {agent_id}"))
        })?;
        let now = chrono::Utc::now().to_rfc3339();
        agent.last_heartbeat = now.clone();

        // Fire-and-forget: replicate heartbeat to SpacetimeDB
        self.spawn_reducer_call(
            "update_remote_heartbeat",
            serde_json::json!([agent_id, now]),
        );

        Ok(())
    }

    async fn deregister_agent(&self, agent_id: &str) -> Result<(), TransportError> {
        tracing::info!(agent_id = %agent_id, "deregistering remote agent");
        self.agents.write().await.remove(agent_id);

        // Fire-and-forget: remove from SpacetimeDB
        self.spawn_reducer_call(
            "deregister_remote_agent",
            serde_json::json!([agent_id]),
        );

        // Also remove all inference servers owned by this agent.
        self.deregister_agent_servers(agent_id).await?;
        Ok(())
    }

    async fn list_agents(
        &self,
        status_filter: Option<RemoteAgentStatus>,
    ) -> Result<Vec<RemoteAgent>, TransportError> {
        // SpacetimeDB-primary: query the authoritative source first.
        let query = match &status_filter {
            Some(status) => {
                let status_str = serde_json::to_value(status)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| format!("{:?}", status).to_lowercase());
                format!(
                    "SELECT agent_id, name, host, project_dir, capabilities, tunnel_id, connected_at, status, last_heartbeat FROM remote_agent WHERE status = '{}'",
                    status_str
                )
            }
            None => "SELECT agent_id, name, host, project_dir, capabilities, tunnel_id, connected_at, status, last_heartbeat FROM remote_agent".to_string(),
        };

        if let Some(rows) = self.sql_query(&query).await {
            let agents: Vec<RemoteAgent> = rows
                .iter()
                .filter_map(Self::parse_agent_row)
                .collect();
            tracing::debug!(count = agents.len(), "list_agents: served from SpacetimeDB");
            return Ok(agents);
        }

        // Fallback: read from local write-through cache.
        tracing::debug!("list_agents: SpacetimeDB unreachable, falling back to cache");
        let agents = self.agents.read().await;
        let iter = agents.values();
        let result: Vec<RemoteAgent> = match status_filter {
            Some(ref status) => iter.filter(|a| a.status == *status).cloned().collect(),
            None => iter.cloned().collect(),
        };
        Ok(result)
    }

    async fn get_agent(&self, agent_id: &str) -> Result<Option<RemoteAgent>, TransportError> {
        // SpacetimeDB-primary: query the authoritative source first.
        let query = format!(
            "SELECT agent_id, name, host, project_dir, capabilities, tunnel_id, connected_at, status, last_heartbeat FROM remote_agent WHERE agent_id = '{}'",
            agent_id.replace('\'', "''")
        );
        if let Some(rows) = self.sql_query(&query).await {
            let agent = rows.first().and_then(Self::parse_agent_row);
            tracing::debug!(agent_id = %agent_id, found = agent.is_some(), "get_agent: served from SpacetimeDB");
            return Ok(agent);
        }

        // Fallback: read from local write-through cache.
        tracing::debug!(agent_id = %agent_id, "get_agent: SpacetimeDB unreachable, falling back to cache");
        Ok(self.agents.read().await.get(agent_id).cloned())
    }

    async fn register_inference_server(
        &self,
        server: InferenceServer,
    ) -> Result<(), TransportError> {
        let id = server.server_id.clone();
        tracing::info!(
            server_id = %id,
            agent_id = %server.agent_id,
            models = ?server.models,
            "registering inference server"
        );
        self.servers.write().await.insert(id, server);
        Ok(())
    }

    async fn update_server_load(
        &self,
        server_id: &str,
        load: f32,
    ) -> Result<(), TransportError> {
        let mut servers = self.servers.write().await;
        let server = servers.get_mut(server_id).ok_or_else(|| {
            TransportError::Protocol(format!("inference server not found: {server_id}"))
        })?;
        server.current_load = load;
        server.status = if load > 0.8 {
            InferenceServerStatus::Busy
        } else {
            InferenceServerStatus::Available
        };
        Ok(())
    }

    async fn list_inference_servers(
        &self,
        model_filter: Option<&str>,
    ) -> Result<Vec<InferenceServer>, TransportError> {
        let servers = self.servers.read().await;
        let iter = servers.values();
        let result: Vec<InferenceServer> = match model_filter {
            Some(model) => iter
                .filter(|s| s.models.iter().any(|m| m.contains(model)))
                .cloned()
                .collect(),
            None => iter.cloned().collect(),
        };
        Ok(result)
    }

    async fn deregister_agent_servers(&self, agent_id: &str) -> Result<(), TransportError> {
        let mut servers = self.servers.write().await;
        servers.retain(|_, s| s.agent_id != agent_id);
        tracing::info!(agent_id = %agent_id, "deregistered all inference servers for agent");
        Ok(())
    }
}
