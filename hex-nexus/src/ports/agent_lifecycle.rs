//! Port for remote agent lifecycle management (ADR-040).

use async_trait::async_trait;
use crate::remote::transport::*;

/// Manages the full lifecycle of remote agents.
///
/// Primary adapters (routes, CLI) use this port to spawn, accept,
/// disconnect, and query remote agents without depending on concrete adapters.
#[async_trait]
pub trait IAgentLifecyclePort: Send + Sync {
    /// Spawn a remote agent via SSH tunnel → WS → register → heartbeat.
    async fn spawn_remote_agent(
        &self,
        config: SshTunnelConfig,
        agent_name: String,
        project_dir: String,
    ) -> Result<RemoteAgent, TransportError>;

    /// Accept an incoming agent connection (agent initiated).
    async fn accept_agent(
        &self,
        agent_id: String,
        capabilities: AgentCapabilities,
        project_dir: String,
    ) -> Result<RemoteAgent, TransportError>;

    /// Disconnect and clean up a remote agent.
    async fn disconnect_agent(&self, agent_id: &str) -> Result<(), TransportError>;

    /// List all managed agents.
    async fn list_agents(&self) -> Result<Vec<RemoteAgent>, TransportError>;

    /// Get a specific agent by ID.
    async fn get_agent(&self, agent_id: &str) -> Result<Option<RemoteAgent>, TransportError>;
}
