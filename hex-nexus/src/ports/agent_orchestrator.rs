//! Port for remote agent orchestration (ADR-040).

use async_trait::async_trait;
use crate::ports::inference_router::FleetCapacity;
use crate::remote::transport::*;

/// Fleet-wide agent status summary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetStatus {
    pub total_agents: u32,
    pub online_agents: u32,
    pub busy_agents: u32,
    pub capacity: FleetCapacity,
}

/// Orchestrates remote code generation across the agent fleet.
///
/// Primary adapters (routes, CLI) use this port to submit code generation
/// requests and query fleet status without depending on the use case directly.
#[async_trait]
pub trait IAgentOrchestratorPort: Send + Sync {
    /// Submit a code generation request, routed to the best available agent.
    async fn submit_code_gen(
        &self,
        request: CodeGenRequest,
    ) -> Result<CodeGenResult, TransportError>;

    /// Get fleet-wide status summary.
    async fn fleet_status(&self) -> Result<FleetStatus, TransportError>;

    /// Handle agent death — reassign tasks, deregister servers.
    async fn handle_agent_death(&self, dead_agent_id: &str) -> Result<u32, TransportError>;
}
