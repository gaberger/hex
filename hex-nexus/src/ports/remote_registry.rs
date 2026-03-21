//! Port for remote agent registry and discovery (ADR-040).

use async_trait::async_trait;
use crate::remote::transport::{
    RemoteAgent, RemoteAgentStatus, InferenceServer, TransportError,
};

/// Registry for remote agents and their inference servers.
///
/// Backed by SpacetimeDB for real-time state sync across the fleet.
/// Provides discovery, health monitoring, and capacity tracking.
#[async_trait]
pub trait IRemoteRegistryPort: Send + Sync {
    // ── Agent Registry ────────────────────────────
    /// Register a newly connected remote agent.
    async fn register_agent(&self, agent: RemoteAgent) -> Result<(), TransportError>;

    /// Update an agent's status (online → busy, busy → stale, etc.)
    async fn update_agent_status(
        &self,
        agent_id: &str,
        status: RemoteAgentStatus,
    ) -> Result<(), TransportError>;

    /// Record a heartbeat from an agent (resets stale timer).
    async fn heartbeat(&self, agent_id: &str) -> Result<(), TransportError>;

    /// Remove an agent from the registry (on disconnect or death).
    async fn deregister_agent(&self, agent_id: &str) -> Result<(), TransportError>;

    /// List all registered remote agents, optionally filtered by status.
    async fn list_agents(
        &self,
        status_filter: Option<RemoteAgentStatus>,
    ) -> Result<Vec<RemoteAgent>, TransportError>;

    /// Get a specific agent by ID.
    async fn get_agent(&self, agent_id: &str) -> Result<Option<RemoteAgent>, TransportError>;

    // ── Inference Server Registry ─────────────────
    /// Register an inference server provided by an agent.
    async fn register_inference_server(
        &self,
        server: InferenceServer,
    ) -> Result<(), TransportError>;

    /// Update an inference server's load metric.
    async fn update_server_load(
        &self,
        server_id: &str,
        load: f32,
    ) -> Result<(), TransportError>;

    /// List all available inference servers, optionally filtered by model name.
    async fn list_inference_servers(
        &self,
        model_filter: Option<&str>,
    ) -> Result<Vec<InferenceServer>, TransportError>;

    /// Remove all inference servers for a given agent (on agent disconnect).
    async fn deregister_agent_servers(&self, agent_id: &str) -> Result<(), TransportError>;
}
