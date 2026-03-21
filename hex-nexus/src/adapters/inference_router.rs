//! Adapter implementing IInferenceRouterPort — routes inference requests
//! to the best available agent based on model availability, load, and locality (ADR-040).

use std::sync::Arc;
use async_trait::async_trait;

use crate::ports::inference_router::{FleetCapacity, IInferenceRouterPort};
use crate::ports::remote_registry::IRemoteRegistryPort;
use crate::ports::agent_transport::IAgentTransportPort;
use crate::remote::transport::{
    CodeGenRequest, CodeGenResult, InferenceServer, InferenceServerStatus, TransportError,
};

pub struct InferenceRouterAdapter {
    registry: Arc<dyn IRemoteRegistryPort>,
    #[allow(dead_code)]
    transport: Arc<dyn IAgentTransportPort>,
}

impl InferenceRouterAdapter {
    pub fn new(
        registry: Arc<dyn IRemoteRegistryPort>,
        transport: Arc<dyn IAgentTransportPort>,
    ) -> Self {
        Self { registry, transport }
    }
}

#[async_trait]
impl IInferenceRouterPort for InferenceRouterAdapter {
    async fn select_server(
        &self,
        model: &str,
        preferred_agent_id: Option<&str>,
    ) -> Result<Option<InferenceServer>, TransportError> {
        // 1. Get all inference servers that have the requested model
        let servers = self.registry.list_inference_servers(Some(model)).await?;
        if servers.is_empty() {
            return Ok(None);
        }

        // 2. If preferred agent specified, check if it has the model and is available
        if let Some(preferred) = preferred_agent_id {
            if let Some(s) = servers
                .iter()
                .find(|s| s.agent_id == preferred && s.status == InferenceServerStatus::Available)
            {
                return Ok(Some(s.clone()));
            }
        }

        // 3. Filter to available servers with load < 0.8
        let mut candidates: Vec<&InferenceServer> = servers
            .iter()
            .filter(|s| s.status == InferenceServerStatus::Available && s.current_load < 0.8)
            .collect();

        if candidates.is_empty() {
            // Fall back to any available server regardless of load
            candidates = servers
                .iter()
                .filter(|s| s.status == InferenceServerStatus::Available)
                .collect();
        }

        if candidates.is_empty() {
            return Ok(None);
        }

        // 4. Sort by load ascending (least loaded first)
        candidates.sort_by(|a, b| {
            a.current_load
                .partial_cmp(&b.current_load)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 5. Return the best one
        Ok(Some(candidates[0].clone()))
    }

    async fn route_request(
        &self,
        request: CodeGenRequest,
    ) -> Result<CodeGenResult, TransportError> {
        let model = request.model.as_deref().unwrap_or("default");

        // Select best server for the requested model
        let server = self.select_server(model, None).await?.ok_or_else(|| {
            TransportError::Protocol(format!(
                "No inference server available for model '{}'",
                model
            ))
        })?;

        tracing::info!(
            model,
            agent_id = %server.agent_id,
            server_id = %server.server_id,
            "Routing code gen request"
        );

        // Stub: actual routing will be completed in T3-4 orchestrator
        // The full pipeline requires sending an InferenceRequest via transport,
        // subscribing for InferenceComplete, and assembling the CodeGenResult.
        Err(TransportError::Protocol(
            "Inference routing not yet wired to transport — requires orchestrator (T3-4)".into(),
        ))
    }

    async fn has_model(&self, model: &str) -> Result<bool, TransportError> {
        let servers = self.registry.list_inference_servers(Some(model)).await?;
        Ok(!servers.is_empty())
    }

    async fn fleet_capacity(&self) -> Result<FleetCapacity, TransportError> {
        let all_servers = self.registry.list_inference_servers(None).await?;
        let available = all_servers
            .iter()
            .filter(|s| s.status == InferenceServerStatus::Available)
            .count();
        let total_vram: u32 = all_servers.iter().map(|s| s.gpu_vram_mb).sum();
        let avg_load = if all_servers.is_empty() {
            0.0
        } else {
            all_servers.iter().map(|s| s.current_load).sum::<f32>() / all_servers.len() as f32
        };

        // Collect unique models across all servers
        let mut all_models: Vec<String> =
            all_servers.iter().flat_map(|s| s.models.clone()).collect();
        all_models.sort();
        all_models.dedup();

        Ok(FleetCapacity {
            total_servers: all_servers.len() as u32,
            available_servers: available as u32,
            total_models: all_models,
            total_gpu_vram_mb: total_vram,
            avg_load,
        })
    }
}
