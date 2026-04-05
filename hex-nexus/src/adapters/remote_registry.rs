//! In-memory + SpacetimeDB implementation of IRemoteRegistryPort (ADR-040, P4.2).
//!
//! HashMap is the fast-path cache for local reads.  Every mutation is also
//! fire-and-forget replicated to SpacetimeDB (hexflo-coordination module,
//! database "hex") so the dashboard and other hosts see agent state in
//! real-time via WebSocket subscriptions.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use async_trait::async_trait;
use tracing;

use crate::ports::remote_registry::IRemoteRegistryPort;
use crate::remote::transport::{
    InferenceServer, InferenceServerStatus, RemoteAgent, RemoteAgentStatus, TransportError,
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
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            servers: Arc::new(RwLock::new(HashMap::new())),
            stdb_url: stdb_url.unwrap_or_else(|| "http://127.0.0.1:3033".to_string()),
            http: reqwest::Client::new(),
        }
    }

    /// Fire-and-forget call to a SpacetimeDB reducer on the "hex" database.
    /// Failures are logged as warnings but never propagated — the HashMap is
    /// the source of truth for the local process.
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
        let agents = self.agents.read().await;
        let iter = agents.values();
        let result: Vec<RemoteAgent> = match status_filter {
            Some(ref status) => iter.filter(|a| a.status == *status).cloned().collect(),
            None => iter.cloned().collect(),
        };
        Ok(result)
    }

    async fn get_agent(&self, agent_id: &str) -> Result<Option<RemoteAgent>, TransportError> {
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
