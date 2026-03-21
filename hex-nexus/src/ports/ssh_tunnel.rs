//! Port for SSH tunnel lifecycle management (ADR-040).

use async_trait::async_trait;
use crate::remote::transport::{
    SshTunnelConfig, TunnelHandle, TunnelHealth, TunnelInfo, TransportError,
};

/// Manages SSH tunnels for secure remote agent communication.
///
/// Implementations handle tunnel establishment, health monitoring,
/// reconnection with exponential backoff, and graceful teardown.
#[async_trait]
pub trait ISshTunnelPort: Send + Sync {
    /// Establish an SSH tunnel to a remote host.
    /// Returns a handle with the local forwarded port for WebSocket connection.
    async fn connect(&self, config: SshTunnelConfig) -> Result<TunnelHandle, TransportError>;

    /// Check the health of an active tunnel.
    async fn health(&self, tunnel_id: &str) -> Result<TunnelHealth, TransportError>;

    /// Reconnect a degraded or disconnected tunnel with exponential backoff.
    async fn reconnect(&self, tunnel_id: &str) -> Result<TunnelHandle, TransportError>;

    /// Gracefully disconnect a tunnel.
    async fn disconnect(&self, tunnel_id: &str) -> Result<(), TransportError>;

    /// List all active tunnels.
    async fn list_tunnels(&self) -> Result<Vec<TunnelInfo>, TransportError>;
}
