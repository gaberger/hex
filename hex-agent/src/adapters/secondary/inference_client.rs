//! Nexus inference client adapter — calls hex-nexus POST /api/inference/complete
//! with task-type headers for RL model selection (ADR-2603232005).
//!
//! This adapter provides a clean, typed interface for LLM inference that:
//! - Sends `X-Hex-Agent-Id` for agent identity tracking
//! - Sends `X-Hex-Task-Type` for RL model selection
//! - Parses OpenRouter cost from the response
//! - Supports both blocking and streaming (simulated via single-response chunking)

use crate::ports::inference_client::{
    ChatMessage, ChatRole, InferenceClientError, InferenceClientPort, InferenceRequest,
    InferenceResponse, InferenceStreamChunk,
};
use async_trait::async_trait;
use futures::stream;
use reqwest::Client;
use serde_json::json;
use std::time::Instant;

/// Adapter that bridges hex-agent to the nexus `/api/inference/complete` endpoint
/// with task-type awareness and agent identity headers.
pub struct NexusInferenceClient {
    client: Client,
    nexus_url: String,
    /// Agent ID from session state (set by `hex hook session-start`).
    agent_id: Option<String>,
}

impl NexusInferenceClient {
    /// Create a new client pointing at the given nexus URL.
    ///
    /// # Arguments
    /// * `nexus_url` — Base URL of hex-nexus (e.g. `http://localhost:5555`)
    /// * `agent_id` — Optional agent ID for `X-Hex-Agent-Id` header
    pub fn new(nexus_url: &str, agent_id: Option<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(180))
                .build()
                .unwrap_or_else(|_| Client::new()),
            nexus_url: nexus_url.trim_end_matches('/').to_string(),
            agent_id,
        }
    }

    /// Create from the default nexus URL (`http://localhost:5555`) and
    /// auto-resolve agent ID from the session file.
    pub fn from_session() -> Self {
        let agent_id = Self::resolve_agent_id();
        Self::new("http://localhost:5555", agent_id)
    }

    /// Resolve agent ID from `~/.hex/sessions/agent-{CLAUDE_SESSION_ID}.json`.
    fn resolve_agent_id() -> Option<String> {
        let session_id = std::env::var("CLAUDE_SESSION_ID").ok()?;
        let session_file = dirs::home_dir()?
            .join(".hex")
            .join("sessions")
            .join(format!("agent-{}.json", session_id));
        let data = std::fs::read_to_string(session_file).ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
        parsed["agent_id"].as_str().map(|s| s.to_string())
    }

    /// Probe the nexus inference endpoint. Returns `true` if the endpoint is reachable.
    pub async fn probe(&self) -> bool {
        let url = format!("{}/api/inference/complete", self.nexus_url);
        let result = Client::new()
            .post(&url)
            .json(&json!({"messages": [], "model": "probe"}))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await;
        result.is_ok()
    }

    /// Update the agent ID (e.g. after session registration).
    pub fn set_agent_id(&mut self, agent_id: String) {
        self.agent_id = Some(agent_id);
    }

    /// Convert ChatMessages to OpenAI-compatible JSON.
    fn messages_to_json(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    ChatRole::System => "system",
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                };
                json!({ "role": role, "content": m.content })
            })
            .collect()
    }
}

#[async_trait]
impl InferenceClientPort for NexusInferenceClient {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceClientError> {
        let url = format!("{}/api/inference/complete", self.nexus_url);
        let start = Instant::now();

        let mut body = json!({
            "messages": Self::messages_to_json(&request.messages),
            "max_tokens": request.max_tokens,
        });

        if let Some(ref model) = request.model {
            body["model"] = json!(model);
        }
        if let Some(ref system) = request.system_prompt {
            body["system"] = json!(system);
        }
        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }

        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-Hex-Task-Type", request.task_type.to_string());

        if let Some(ref agent_id) = self.agent_id {
            req = req.header("X-Hex-Agent-Id", agent_id.as_str());
        }

        let response = req.json(&body).send().await.map_err(|e| {
            InferenceClientError::NexusDown {
                url: url.clone(),
                cause: e.to_string(),
            }
        })?;

        let status = response.status().as_u16();

        // Handle error responses
        if status == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);
            return Err(InferenceClientError::RateLimited {
                retry_after_ms: retry_after * 1000,
            });
        }
        if status == 402 {
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceClientError::BudgetExceeded(text));
        }
        if status >= 400 {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".into());

            // Check for model-unavailable patterns
            if text.contains("No inference endpoints") || text.contains("not available") {
                return Err(InferenceClientError::ModelUnavailable(text));
            }

            return Err(InferenceClientError::ApiError {
                status,
                message: text,
            });
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| InferenceClientError::InvalidResponse(format!("invalid JSON: {e}")))?;

        let content = data["content"]
            .as_str()
            .unwrap_or("(empty)")
            .to_string();
        let model_used = data["model"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let input_tokens = data["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = data["output_tokens"].as_u64().unwrap_or(0);

        // Parse OpenRouter cost (returned as string by nexus)
        let cost_usd = data["openrouter_cost_usd"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| data["openrouter_cost_usd"].as_f64());

        Ok(InferenceResponse {
            content,
            model_used,
            input_tokens,
            output_tokens,
            cost_usd,
            duration_ms,
        })
    }

    async fn complete_stream(
        &self,
        request: InferenceRequest,
    ) -> Result<
        Box<dyn futures::Stream<Item = InferenceStreamChunk> + Send + Unpin>,
        InferenceClientError,
    > {
        // The nexus /api/inference/complete endpoint doesn't support SSE streaming yet.
        // Simulate streaming by returning the full response as two chunks:
        // 1. TextDelta with the content
        // 2. Done with usage stats
        let response = self.complete(request).await?;

        let chunks = vec![
            InferenceStreamChunk::TextDelta(response.content),
            InferenceStreamChunk::Done {
                model_used: response.model_used,
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
                cost_usd: response.cost_usd,
                duration_ms: response.duration_ms,
            },
        ];

        Ok(Box::new(stream::iter(chunks)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::inference_client::{ChatMessage, ChatRole, TaskType};

    #[test]
    fn messages_to_json_converts_correctly() {
        let messages = vec![
            ChatMessage {
                role: ChatRole::User,
                content: "Hello".to_string(),
            },
            ChatMessage {
                role: ChatRole::Assistant,
                content: "Hi there".to_string(),
            },
        ];

        let json = NexusInferenceClient::messages_to_json(&messages);
        assert_eq!(json.len(), 2);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[0]["content"], "Hello");
        assert_eq!(json[1]["role"], "assistant");
        assert_eq!(json[1]["content"], "Hi there");
    }

    #[test]
    fn task_type_display() {
        assert_eq!(TaskType::Reasoning.to_string(), "reasoning");
        assert_eq!(TaskType::StructuredOutput.to_string(), "structured_output");
        assert_eq!(TaskType::CodeGeneration.to_string(), "code_generation");
        assert_eq!(TaskType::CodeEdit.to_string(), "code_edit");
        assert_eq!(TaskType::General.to_string(), "general");
    }

    #[test]
    fn default_request_values() {
        let req = InferenceRequest::default();
        assert_eq!(req.max_tokens, 4096);
        assert_eq!(req.task_type, TaskType::General);
        assert!(req.model.is_none());
        assert!(req.system_prompt.is_none());
        assert!(req.temperature.is_none());
        assert!(req.messages.is_empty());
    }

    #[test]
    fn client_trims_trailing_slash() {
        let client = NexusInferenceClient::new("http://localhost:5555/", None);
        assert_eq!(client.nexus_url, "http://localhost:5555");
    }

    #[test]
    fn set_agent_id_updates() {
        let mut client = NexusInferenceClient::new("http://localhost:5555", None);
        assert!(client.agent_id.is_none());
        client.set_agent_id("agent-123".to_string());
        assert_eq!(client.agent_id.as_deref(), Some("agent-123"));
    }
}
