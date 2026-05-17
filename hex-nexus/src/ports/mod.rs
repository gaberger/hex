pub mod events;
pub mod live_context;
pub mod secret_grant;
pub mod session;
pub mod state;
pub mod ssh_tunnel;
pub mod agent_transport;
pub mod remote_registry;
pub mod inference_router;
pub mod agent_lifecycle;
pub mod agent_orchestrator;
pub mod adr_review;

pub use secret_grant::ISecretGrantPort;
pub use session::ISessionPort;
pub use state::IStatePort;
// Focused sub-traits (ADR-2604050900 P6) — prefer these over IStatePort for narrow dependencies.
pub use state::{
    IRlStatePort, IPatternStatePort, IAgentStatePort, IWorkplanStatePort,
    IChatStatePort, ISkillStatePort, IAgentDefStatePort, ISwarmStatePort,
    IInferenceTaskStatePort, IHexFloMemoryStatePort, IQualityGateStatePort,
    IProjectStatePort, ICoordinationStatePort, IHexAgentStatePort,
    IInboxStatePort, INeuralLabStatePort,
};
pub use ssh_tunnel::ISshTunnelPort;
pub use agent_transport::IAgentTransportPort;
pub use remote_registry::IRemoteRegistryPort;
pub use inference_router::IInferenceRouterPort;
pub use agent_lifecycle::IAgentLifecyclePort;
pub use agent_orchestrator::IAgentOrchestratorPort;
pub use adr_review::IAdrReviewPort;

// Re-export hex-core ports for downstream consumers
pub use hex_core::ports::coordination;
pub use hex_core::ports::file_system;
pub use hex_core::ports::inference;
pub use hex_core::ports::secret;

// Re-export hex-core domain types
pub use hex_core::domain;
