//! Port for intelligent inference request routing (ADR-040).

use async_trait::async_trait;
use crate::remote::transport::{
    CodeGenRequest, CodeGenResult, InferenceServer, TransportError,
};

/// Routes code generation and inference requests to the best available agent.
///
/// Routing priority:
/// 1. Model availability — does the agent have the requested model?
/// 2. Current load — pick the least-loaded agent (< 0.8 threshold)
/// 3. Network locality — prefer local for small tasks
/// 4. Project affinity — prefer agents with cached project context
#[async_trait]
pub trait IInferenceRouterPort: Send + Sync {
    /// Select the best inference server for a given request.
    /// Returns None if no suitable server is available.
    async fn select_server(
        &self,
        model: &str,
        preferred_agent_id: Option<&str>,
    ) -> Result<Option<InferenceServer>, TransportError>;

    /// Route a code generation request to the best agent and return the result.
    /// Handles fallback: remote GPU → local → direct LLM bridge → error.
    async fn route_request(
        &self,
        request: CodeGenRequest,
    ) -> Result<CodeGenResult, TransportError>;

    /// Check if any inference server has a specific model available.
    async fn has_model(&self, model: &str) -> Result<bool, TransportError>;

    /// Get current fleet capacity summary.
    async fn fleet_capacity(&self) -> Result<FleetCapacity, TransportError>;
}

/// Summary of fleet-wide inference capacity.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetCapacity {
    pub total_servers: u32,
    pub available_servers: u32,
    pub total_models: Vec<String>,
    pub total_gpu_vram_mb: u32,
    pub avg_load: f32,
}
