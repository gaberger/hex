//! Port for bidirectional WebSocket transport between nexus and remote agents (ADR-040).

use async_trait::async_trait;
use tokio::sync::mpsc;
use crate::remote::transport::{AgentMessage, TransportError};

/// Bidirectional WebSocket transport for agent communication.
///
/// Handles message serialization, framing, and delivery over
/// a WebSocket connection (typically tunneled through SSH).
#[async_trait]
pub trait IAgentTransportPort: Send + Sync {
    /// Send a message to a connected agent.
    async fn send(&self, agent_id: &str, message: AgentMessage) -> Result<(), TransportError>;

    /// Get a receiver channel for incoming messages from a specific agent.
    async fn subscribe(&self, agent_id: &str) -> Result<mpsc::Receiver<AgentMessage>, TransportError>;

    /// Check if an agent's transport connection is alive.
    async fn is_connected(&self, agent_id: &str) -> bool;

    /// Disconnect an agent's transport.
    async fn disconnect(&self, agent_id: &str) -> Result<(), TransportError>;
}
