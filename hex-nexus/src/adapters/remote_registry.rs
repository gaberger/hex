//! In-memory implementation of IRemoteRegistryPort (ADR-040).
//!
//! Stores remote agent and inference server state in HashMaps.
//! Will be upgraded to SpacetimeDB subscriptions in a future iteration.

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
}

impl RemoteRegistryAdapter {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            servers: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl IRemoteRegistryPort for RemoteRegistryAdapter {
    async fn register_agent(&self, agent: RemoteAgent) -> Result<(), TransportError> {
        let id = agent.agent_id.clone();
        tracing::info!(agent_id = %id, name = %agent.name, "registering remote agent");
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
        agent.status = status;
        Ok(())
    }

    async fn heartbeat(&self, agent_id: &str) -> Result<(), TransportError> {
        let mut agents = self.agents.write().await;
        let agent = agents.get_mut(agent_id).ok_or_else(|| {
            TransportError::Protocol(format!("agent not found: {agent_id}"))
        })?;
        agent.last_heartbeat = chrono::Utc::now().to_rfc3339();
        Ok(())
    }

    async fn deregister_agent(&self, agent_id: &str) -> Result<(), TransportError> {
        tracing::info!(agent_id = %agent_id, "deregistering remote agent");
        self.agents.write().await.remove(agent_id);
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
