//! Adapter implementing IInferenceRouterPort — routes inference requests
//! to the best available agent based on model availability, load, and locality (ADR-040).

use std::sync::Arc;
use async_trait::async_trait;

use crate::ports::inference_router::{FleetCapacity, IInferenceRouterPort};
use crate::ports::remote_registry::IRemoteRegistryPort;
use crate::ports::agent_transport::IAgentTransportPort;
use crate::adapters::inference::ollama::OllamaInferenceAdapter;
use crate::remote::transport::{
    CodeGenRequest, CodeGenResult, InferenceServer, InferenceServerStatus, TransportError,
    TokenUsage,
};
use hex_core::domain::messages::{ContentBlock, Message};
use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};

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
            load = server.current_load,
            "Routing code gen request to selected server"
        );

        // Build an IInferencePort adapter for the selected server.
        // Currently Ollama-only; extend match arms for Vllm/OpenAi/Anthropic
        // as those adapters are implemented.
        let adapter: Box<dyn IInferencePort> = match server.provider {
            crate::remote::transport::InferenceProvider::Ollama
            | crate::remote::transport::InferenceProvider::Vllm
            | crate::remote::transport::InferenceProvider::LlamaCpp => {
                Box::new(OllamaInferenceAdapter::new(Some(server.base_url.clone())))
            }
            other => {
                return Err(TransportError::Protocol(format!(
                    "No adapter implemented for provider {:?}",
                    other
                )));
            }
        };

        // Bridge CodeGenRequest → InferenceRequest
        let inference_req = InferenceRequest {
            model: model.to_string(),
            system_prompt: String::new(),
            messages: vec![Message::user(&request.prompt)],
            tools: vec![],
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: 0.2,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Normal,
        };

        let response = adapter.complete(inference_req).await.map_err(|e| {
            TransportError::Protocol(format!(
                "Inference failed on {} ({}): {}",
                server.server_id, model, e
            ))
        })?;

        // Bridge InferenceResponse → CodeGenResult
        let code = response
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(CodeGenResult {
            code,
            model_used: response.model_used,
            tokens_used: TokenUsage {
                input_tokens: response.input_tokens as u32,
                output_tokens: response.output_tokens as u32,
                total_tokens: (response.input_tokens + response.output_tokens) as u32,
            },
            files_modified: request.target_file.into_iter().collect(),
        })
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
