pub mod secret_grant;
pub mod session;
pub mod state;
pub mod ssh_tunnel;
pub mod agent_transport;
pub mod remote_registry;
pub mod inference_router;
pub mod agent_lifecycle;
pub mod agent_orchestrator;

pub use secret_grant::ISecretGrantPort;
pub use session::ISessionPort;
pub use state::IStatePort;
pub use ssh_tunnel::ISshTunnelPort;
pub use agent_transport::IAgentTransportPort;
pub use remote_registry::IRemoteRegistryPort;
pub use inference_router::IInferenceRouterPort;
pub use agent_lifecycle::IAgentLifecyclePort;
pub use agent_orchestrator::IAgentOrchestratorPort;

// Re-export hex-core ports for downstream consumers
pub use hex_core::ports::coordination;
pub use hex_core::ports::file_system;
pub use hex_core::ports::inference;
pub use hex_core::ports::secret;

// Re-export hex-core domain types
pub use hex_core::domain;
