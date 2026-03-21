//! Use case: Remote Agent Orchestrator (ADR-040, T3-4)
//!
//! Coordinates remote code generation across the agent fleet.
//! Depends ONLY on port traits — never on adapters (hexagonal architecture).

use std::sync::Arc;

use crate::ports::agent_transport::IAgentTransportPort;
use crate::ports::inference_router::{FleetCapacity, IInferenceRouterPort};
use crate::ports::remote_registry::IRemoteRegistryPort;
use crate::remote::transport::*;

/// Orchestrates remote code generation across the agent fleet.
///
/// Responsibilities:
/// - Route code gen requests to the best available agent
/// - Handle streaming partial results back to the caller
/// - Implement fallback logic when agents disconnect mid-task
/// - Aggregate results from multi-agent code generation
pub struct RemoteAgentOrchestrator {
    registry: Arc<dyn IRemoteRegistryPort>,
    transport: Arc<dyn IAgentTransportPort>,
    router: Arc<dyn IInferenceRouterPort>,
}

impl RemoteAgentOrchestrator {
    pub fn new(
        registry: Arc<dyn IRemoteRegistryPort>,
        transport: Arc<dyn IAgentTransportPort>,
        router: Arc<dyn IInferenceRouterPort>,
    ) -> Self {
        Self {
            registry,
            transport,
            router,
        }
    }

    /// Submit a code generation request. Routes to the best agent and returns the result.
    ///
    /// Fallback chain:
    /// 1. Remote GPU agent (has model, low load)
    /// 2. Any online agent with an inference server
    /// 3. Error: no agents available
    pub async fn submit_code_gen(
        &self,
        request: CodeGenRequest,
    ) -> Result<CodeGenResult, TransportError> {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| "default".to_string());

        // 1. Find best server via router
        let server = self.router.select_server(&model, None).await?;

        let server = match server {
            Some(s) => s,
            None => {
                tracing::warn!(
                    model = model.as_str(),
                    "No inference server available, checking for any online agent"
                );
                // Fallback: try any online agent
                let agents = self
                    .registry
                    .list_agents(Some(RemoteAgentStatus::Online))
                    .await?;
                if agents.is_empty() {
                    return Err(TransportError::Protocol(
                        "No agents available for code generation".into(),
                    ));
                }
                // Use the first available agent's first server (if any)
                let agent = &agents[0];
                let servers = self.registry.list_inference_servers(None).await?;
                servers
                    .into_iter()
                    .find(|s| s.agent_id == agent.agent_id)
                    .ok_or_else(|| {
                        TransportError::Protocol("Agent has no inference servers".into())
                    })?
            }
        };

        tracing::info!(
            agent_id = server.agent_id.as_str(),
            server_id = server.server_id.as_str(),
            model = model.as_str(),
            "Routing code gen request to agent"
        );

        // 2. Send TaskAssign to the agent
        let task_id = uuid::Uuid::new_v4().to_string();
        self.transport
            .send(
                &server.agent_id,
                AgentMessage::TaskAssign {
                    task_id: task_id.clone(),
                    request: request.clone(),
                },
            )
            .await?;

        // 3. Subscribe to responses from this agent
        let mut rx = self.transport.subscribe(&server.agent_id).await?;

        // 4. Wait for TaskComplete or TaskFailed
        let timeout = tokio::time::Duration::from_secs(300); // 5 min timeout
        let result = tokio::time::timeout(timeout, async {
            while let Some(msg) = rx.recv().await {
                match msg {
                    AgentMessage::TaskComplete {
                        task_id: ref tid,
                        ref result,
                    } if *tid == task_id => {
                        return Ok(result.clone());
                    }
                    AgentMessage::TaskFailed {
                        task_id: ref tid,
                        ref error,
                    } if *tid == task_id => {
                        return Err(TransportError::Protocol(format!(
                            "Agent failed: {}",
                            error
                        )));
                    }
                    AgentMessage::StreamChunk {
                        task_id: ref tid,
                        ref chunk,
                        sequence,
                    } if *tid == task_id => {
                        tracing::debug!(
                            task_id = tid.as_str(),
                            sequence,
                            chunk_len = chunk.len(),
                            "Received stream chunk"
                        );
                        // TODO: Forward chunks to caller via a channel for real-time streaming
                    }
                    _ => {} // Ignore other messages
                }
            }
            Err(TransportError::ConnectionLost(
                "Agent disconnected during code generation".into(),
            ))
        })
        .await;

        match result {
            Ok(r) => r,
            Err(_) => {
                tracing::error!(task_id = task_id.as_str(), "Code gen timed out after 300s");
                // Attempt to cancel the task on the agent
                let _ = self
                    .transport
                    .send(
                        &server.agent_id,
                        AgentMessage::TaskCancel {
                            task_id,
                            reason: "Timeout".into(),
                        },
                    )
                    .await;
                Err(TransportError::Timeout(
                    "Code generation timed out after 300s".into(),
                ))
            }
        }
    }

    /// Get current fleet capacity summary.
    pub async fn fleet_status(&self) -> Result<FleetStatus, TransportError> {
        let agents = self.registry.list_agents(None).await?;
        let capacity = self.router.fleet_capacity().await?;

        let online = agents
            .iter()
            .filter(|a| a.status == RemoteAgentStatus::Online)
            .count();
        let busy = agents
            .iter()
            .filter(|a| a.status == RemoteAgentStatus::Busy)
            .count();

        Ok(FleetStatus {
            total_agents: agents.len() as u32,
            online_agents: online as u32,
            busy_agents: busy as u32,
            capacity,
        })
    }

    /// Reassign all tasks from a dead agent to other available agents.
    pub async fn handle_agent_death(&self, dead_agent_id: &str) -> Result<u32, TransportError> {
        tracing::warn!(
            agent_id = dead_agent_id,
            "Handling agent death — reassigning tasks"
        );
        // For now, just log and deregister servers.
        // Full task reassignment requires integration with HexFlo task tracking.
        self.registry
            .deregister_agent_servers(dead_agent_id)
            .await?;
        self.registry
            .update_agent_status(dead_agent_id, RemoteAgentStatus::Dead)
            .await?;
        Ok(0) // TODO: return count of reassigned tasks
    }
}

/// Summary of fleet-wide agent status.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetStatus {
    pub total_agents: u32,
    pub online_agents: u32,
    pub busy_agents: u32,
    pub capacity: FleetCapacity,
}
