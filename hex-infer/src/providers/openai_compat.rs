//! OpenAI-compatible chat-completions adapter.
//!
//! Salvaged out of `hex-agent/src/adapters/secondary/openai_compat.rs` per
//! ADR-2608241500 P2.3. The HTTP body construction, the Anthropic↔OpenAI
//! message translation, the `<think>` stripping, and the OpenRouter routing
//! preferences are carried over unchanged. What changed is the contract: the
//! original implemented hex-agent's private `AnthropicPort`; this one
//! implements [`IInferencePort`], the single inference contract in
//! `hex-core`, so the whole workspace speaks one shape.
//!
//! Works with: MiniMax, Together AI, Groq, OpenRouter, Ollama's `/v1` shim,
//! and vLLM.
//!
//! # Behaviour differences from the hex-agent original
//!
//! - `temperature` is forwarded. The old `AnthropicPort` signature had no
//!   place for it, so it was silently dropped on every request.
//! - Errors map onto [`InferenceError`] using the same convention the Ollama
//!   adapter established: transport failure → `ProviderUnavailable`,
//!   HTTP 404 → `UnknownProvider`, 429 → `RateLimited`, other 4xx/5xx →
//!   `ApiError`.
//! - [`IInferencePort::health`] is new — `GET {base_url}/models`.
//!
//! `request.grammar` is ignored here. OpenAI-compatible servers express
//! structured output through `response_format`, not GBNF; wiring that is
//! P2.4's job, and silently pretending to honour a grammar would be worse
//! than declaring the gap.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use hex_core::domain::messages::{ContentBlock, Role, StopReason};
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
};
use hex_core::domain::messages::Message;
use hex_core::domain::tools::ToolDefinition;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::vec_stream::VecStream;

/// Default request timeout. Overridable with `HEX_INFERENCE_TIMEOUT_SECS`.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Adapter for any OpenAI-compatible chat completions API.
pub struct OpenAiCompatAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl OpenAiCompatAdapter {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        let timeout = std::env::var("HEX_INFERENCE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(timeout))
                .build()
                .unwrap_or_default(),
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
        }
    }

    /// Convenience constructor for MiniMax M2.7 (Coding Plan Max).
    pub fn minimax(api_key: String) -> Self {
        Self::new(
            api_key,
            "https://api.minimax.io/v1".to_string(),
            "MiniMax-M2.7".to_string(),
        )
    }

    /// Convenience constructor for MiniMax M1 (fallback).
    pub fn minimax_fast(api_key: String) -> Self {
        Self::new(
            api_key,
            "https://api.minimax.io/v1".to_string(),
            "MiniMax-M1".to_string(),
        )
    }

    /// Convenience constructor for Ollama's OpenAI-compatible `/v1` shim.
    ///
    /// Prefer [`super::ollama::OllamaInferenceAdapter`] for local Ollama —
    /// it speaks Ollama's native API and streams for real. This constructor
    /// exists for remote Ollama hosts already fronted as OpenAI-compatible.
    pub fn ollama(model: &str, host: Option<&str>) -> Self {
        let base_url = host
            .unwrap_or("http://127.0.0.1:11434")
            .trim_end_matches('/')
            .to_string();
        Self::new(
            String::new(), // Ollama doesn't need an API key
            format!("{}/v1", base_url),
            model.to_string(),
        )
    }

    /// Convenience constructor for vLLM (self-hosted GPU inference).
    pub fn vllm(model: &str, host: Option<&str>, api_key: Option<&str>) -> Self {
        let base_url = host
            .unwrap_or("http://127.0.0.1:8000")
            .trim_end_matches('/')
            .to_string();
        Self::new(
            api_key.unwrap_or("").to_string(),
            format!("{}/v1", base_url),
            model.to_string(),
        )
    }

    /// Convenience constructor for OpenRouter (300+ models, one API key).
    pub fn openrouter(api_key: String, model: String) -> Self {
        Self::new(
            api_key,
            "https://openrouter.ai/api/v1".to_string(),
            model,
        )
    }

    /// Returns true if this adapter is pointing at OpenRouter.
    fn is_openrouter(&self) -> bool {
        self.base_url.contains("openrouter.ai")
    }

    /// Create from environment variables for self-hosted models.
    ///
    /// Reads, in priority order:
    /// - `HEX_OLLAMA_MODEL` / `HEX_OLLAMA_HOST`
    /// - `HEX_VLLM_MODEL` / `HEX_VLLM_HOST` / `HEX_VLLM_KEY`
    /// - `HEX_INFERENCE_URL` / `HEX_INFERENCE_MODEL` / `HEX_INFERENCE_KEY`
    pub fn from_env_self_hosted() -> Option<Self> {
        if let Ok(model) = std::env::var("HEX_OLLAMA_MODEL") {
            let host = std::env::var("HEX_OLLAMA_HOST").ok();
            return Some(Self::ollama(&model, host.as_deref()));
        }

        if let Ok(model) = std::env::var("HEX_VLLM_MODEL") {
            let host = std::env::var("HEX_VLLM_HOST").ok();
            let key = std::env::var("HEX_VLLM_KEY").ok();
            return Some(Self::vllm(&model, host.as_deref(), key.as_deref()));
        }

        if let Ok(url) = std::env::var("HEX_INFERENCE_URL") {
            let model = std::env::var("HEX_INFERENCE_MODEL").unwrap_or("default".to_string());
            let key = std::env::var("HEX_INFERENCE_KEY").unwrap_or_default();
            return Some(Self::new(key, url, model));
        }

        None
    }

    /// The model this request should use: the request's own model when set,
    /// otherwise the adapter's configured default.
    fn resolve_model<'a>(&'a self, requested: &'a str) -> &'a str {
        if requested.trim().is_empty() {
            &self.model
        } else {
            requested
        }
    }

    /// Convert hex-core messages to OpenAI chat format.
    fn to_openai_messages(system: &str, messages: &[Message]) -> Vec<OaiMessage> {
        let mut out = vec![OaiMessage {
            role: "system".to_string(),
            content: Some(system.to_string()),
            tool_calls: None,
            tool_call_id: None,
        }];

        for msg in messages {
            match msg.role {
                Role::User => {
                    // Collect text content and tool results
                    let mut text_parts = Vec::new();
                    for block in &msg.content {
                        match block {
                            ContentBlock::Text { text } => text_parts.push(text.clone()),
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                out.push(OaiMessage {
                                    role: "tool".to_string(),
                                    content: Some(content.clone()),
                                    tool_calls: None,
                                    tool_call_id: Some(tool_use_id.clone()),
                                });
                            }
                            _ => {}
                        }
                    }
                    if !text_parts.is_empty() {
                        out.push(OaiMessage {
                            role: "user".to_string(),
                            content: Some(text_parts.join("\n")),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                }
                Role::Assistant => {
                    let mut text_parts = Vec::new();
                    let mut tool_calls = Vec::new();

                    for block in &msg.content {
                        match block {
                            ContentBlock::Text { text } => text_parts.push(text.clone()),
                            ContentBlock::ToolUse { id, name, input } => {
                                tool_calls.push(OaiToolCall {
                                    id: id.clone(),
                                    r#type: "function".to_string(),
                                    function: OaiFunction {
                                        name: name.clone(),
                                        arguments: serde_json::to_string(input)
                                            .unwrap_or_default(),
                                    },
                                });
                            }
                            _ => {}
                        }
                    }

                    out.push(OaiMessage {
                        role: "assistant".to_string(),
                        content: if text_parts.is_empty() {
                            None
                        } else {
                            Some(text_parts.join("\n"))
                        },
                        tool_calls: if tool_calls.is_empty() {
                            None
                        } else {
                            Some(tool_calls)
                        },
                        tool_call_id: None,
                    });
                }
            }
        }

        out
    }

    /// Convert hex-core tool definitions to OpenAI function format.
    fn to_openai_tools(tools: &[ToolDefinition]) -> Vec<OaiToolDef> {
        tools
            .iter()
            .map(|t| OaiToolDef {
                r#type: "function".to_string(),
                function: OaiToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.input_schema.properties.clone(),
                },
            })
            .collect()
    }

    /// Strip `<think>` tags from content (MiniMax and Qwen thinking output).
    /// Returns (cleaned_text, thinking_text).
    fn strip_thinking(text: &str) -> (String, Option<String>) {
        if let Some(start) = text.find("<think>") {
            if let Some(end) = text.find("</think>") {
                let thinking = text[start + 7..end].trim().to_string();
                let cleaned = format!("{}{}", text[..start].trim(), text[end + 8..].trim());
                return (cleaned.trim().to_string(), Some(thinking));
            }
        }
        (text.to_string(), None)
    }

    /// Build the chat-completions request body.
    fn build_body(&self, request: &InferenceRequest) -> serde_json::Value {
        let model = self.resolve_model(&request.model);
        let oai_messages = Self::to_openai_messages(&request.system_prompt, &request.messages);

        let mut body = serde_json::json!({
            "model": model,
            "messages": oai_messages,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
        });

        if !request.tools.is_empty() {
            body["tools"] =
                serde_json::to_value(Self::to_openai_tools(&request.tools)).unwrap_or_default();
        }

        // OpenRouter routing preferences (ADR-2026-03-23-1600)
        if self.is_openrouter() {
            body["provider"] = serde_json::json!({
                "order": ["Together", "Lambda", "Fireworks"],
                "allow_fallbacks": true
            });
            body["route"] = serde_json::json!("fallback");
        }

        body
    }
}

#[async_trait]
impl IInferencePort for OpenAiCompatAdapter {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let started = Instant::now();
        let body = self.build_body(&request);

        let mut http = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json");

        if !self.api_key.is_empty() {
            http = http.header("Authorization", format!("Bearer {}", self.api_key));
        }

        // OpenRouter-specific headers (ADR-2026-03-23-1600)
        if self.is_openrouter() {
            http = http
                .header("HTTP-Referer", "https://github.com/hex-intf")
                .header("X-Title", "hex");
        }

        let response = http
            .json(&body)
            .send()
            .await
            .map_err(|e| InferenceError::ProviderUnavailable(format!("{}: {}", self.base_url, e)))?;

        let status = response.status().as_u16();

        if status == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(5);
            return Err(InferenceError::RateLimited(format!(
                "retry after {}s",
                retry_after
            )));
        }
        if status == 402 {
            return Err(InferenceError::ApiError {
                status,
                body: "insufficient credits (OpenRouter: top up at https://openrouter.ai/credits)"
                    .to_string(),
            });
        }
        if status == 404 {
            // Same convention as the Ollama adapter: 404 means the backend has
            // never heard of this model, which is distinct from being down.
            return Err(InferenceError::UnknownProvider(format!(
                "{} has no model '{}'",
                self.base_url,
                self.resolve_model(&request.model)
            )));
        }
        if status >= 400 {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".into());
            return Err(InferenceError::ApiError { status, body: text });
        }

        let oai_resp: OaiChatResponse = response.json().await.map_err(|e| InferenceError::ApiError {
            status,
            body: format!("malformed chat-completions response: {}", e),
        })?;

        let choice = oai_resp
            .choices
            .first()
            .ok_or_else(|| InferenceError::ApiError {
                status,
                body: "no choices in response".into(),
            })?;

        let mut content = Vec::new();

        // Text content — strip thinking tags
        if let Some(ref text) = choice.message.content {
            let (cleaned, thinking) = Self::strip_thinking(text);
            if let Some(think) = thinking {
                tracing::debug!(thinking_len = think.len(), "Stripped thinking content");
            }
            if !cleaned.is_empty() {
                content.push(ContentBlock::Text { text: cleaned });
            }
        }

        // Tool calls
        if let Some(ref tool_calls) = choice.message.tool_calls {
            for tc in tool_calls {
                let input: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                content.push(ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    input,
                });
            }
        }

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        // Log OpenRouter actual cost if present
        if let Some(cost) = oai_resp.usage.cost {
            tracing::info!(
                openrouter_cost_usd = cost,
                model = %oai_resp.model,
                "OpenRouter actual cost"
            );
        }

        Ok(InferenceResponse {
            content,
            model_used: oai_resp.model,
            stop_reason,
            input_tokens: oai_resp.usage.prompt_tokens as u64,
            output_tokens: oai_resp.usage.completion_tokens as u64,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        request: InferenceRequest,
    ) -> Result<Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>, InferenceError>
    {
        // This adapter does not yet parse SSE. It completes the request and
        // replays the answer as chunks so callers get one shape either way.
        // Callers that need real token-by-token output should use the Ollama
        // adapter, which streams NDJSON for real.
        let resp = self.complete(request).await?;

        let mut chunks = Vec::new();
        for block in &resp.content {
            match block {
                ContentBlock::Text { text } => {
                    chunks.push(StreamChunk::TextDelta(text.clone()));
                }
                ContentBlock::ToolUse { id, name, input } => {
                    chunks.push(StreamChunk::ToolUseStart {
                        id: id.clone(),
                        name: name.clone(),
                    });
                    chunks.push(StreamChunk::InputJsonDelta(
                        serde_json::to_string(input).unwrap_or_default(),
                    ));
                }
                _ => {}
            }
        }
        chunks.push(StreamChunk::Usage {
            input_tokens: resp.input_tokens,
            output_tokens: resp.output_tokens,
        });
        chunks.push(StreamChunk::MessageStop(resp.stop_reason));

        Ok(Box::new(VecStream::new(chunks)))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        let mut http = self.client.get(format!("{}/models", self.base_url));
        if !self.api_key.is_empty() {
            http = http.header("Authorization", format!("Bearer {}", self.api_key));
        }

        let response = match http.send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(HealthStatus::Unreachable {
                    reason: format!("{}: {}", self.base_url, e),
                })
            }
        };

        let status = response.status().as_u16();
        if status >= 400 {
            return Ok(HealthStatus::Degraded {
                reason: format!("GET {}/models returned {}", self.base_url, status),
            });
        }

        let listing: OaiModelListing = match response.json().await {
            Ok(l) => l,
            // A reachable endpoint that cannot enumerate models is still
            // usable for completions — many OpenAI-compatible shims omit
            // /models entirely.
            Err(_) => return Ok(HealthStatus::Ok { models: vec![] }),
        };

        Ok(HealthStatus::Ok {
            models: listing.data.into_iter().map(|m| m.id).collect(),
        })
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![ModelInfo {
                id: self.model.clone(),
                provider: "openai_compat".to_string(),
                tier: ModelTier::Local,
                context_window: 0, // unknown without a provider-specific probe
            }],
            supports_tool_use: true,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false, // replayed, not incremental — see `stream`
            max_context_tokens: 0,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

// --- OpenAI API types (private, for serialization only) ---

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiToolCall {
    id: String,
    r#type: String,
    function: OaiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OaiToolDef {
    r#type: String,
    function: OaiToolFunction,
}

#[derive(Debug, Serialize)]
struct OaiToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OaiChatResponse {
    model: String,
    choices: Vec<OaiChoice>,
    usage: OaiUsage,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiChoiceMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Debug, Deserialize, Default)]
struct OaiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    /// Actual cost in USD (OpenRouter-specific, absent for other providers)
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct OaiModelListing {
    #[serde(default)]
    data: Vec<OaiModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OaiModelEntry {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::tools::ToolInputSchema;
    use hex_core::ports::inference::Priority;

    fn req(model: &str) -> InferenceRequest {
        InferenceRequest {
            model: model.to_string(),
            system_prompt: "You are helpful.".into(),
            messages: vec![Message::user("Hello")],
            tools: vec![],
            max_tokens: 256,
            temperature: 0.3,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Normal,
            grammar: None,
        }
    }

    #[test]
    fn strip_thinking_with_tags() {
        let input = "<think>Let me reason about this...</think>The answer is 42.";
        let (cleaned, thinking) = OpenAiCompatAdapter::strip_thinking(input);
        assert_eq!(cleaned, "The answer is 42.");
        assert_eq!(thinking.unwrap(), "Let me reason about this...");
    }

    #[test]
    fn strip_thinking_no_tags() {
        let input = "Just a plain response.";
        let (cleaned, thinking) = OpenAiCompatAdapter::strip_thinking(input);
        assert_eq!(cleaned, "Just a plain response.");
        assert!(thinking.is_none());
    }

    #[test]
    fn to_openai_messages_basic() {
        let messages = vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "Hello".to_string(),
                }],
            },
            Message {
                role: Role::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Hi there".to_string(),
                }],
            },
        ];

        let oai = OpenAiCompatAdapter::to_openai_messages("You are helpful.", &messages);
        assert_eq!(oai.len(), 3); // system + user + assistant
        assert_eq!(oai[0].role, "system");
        assert_eq!(oai[1].role, "user");
        assert_eq!(oai[2].role, "assistant");
    }

    #[test]
    fn tool_result_becomes_a_tool_role_message() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".into(),
                content: "42".into(),
                is_error: None,
            }],
        }];
        let oai = OpenAiCompatAdapter::to_openai_messages("sys", &messages);
        assert_eq!(oai.len(), 2); // system + tool
        assert_eq!(oai[1].role, "tool");
        assert_eq!(oai[1].tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn openrouter_constructor_sets_correct_url() {
        let adapter = OpenAiCompatAdapter::openrouter(
            "sk-or-test".to_string(),
            "meta-llama/llama-4-maverick".to_string(),
        );
        assert!(adapter.is_openrouter());
        assert_eq!(adapter.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(adapter.model, "meta-llama/llama-4-maverick");
    }

    #[test]
    fn non_openrouter_detected_correctly() {
        let adapter = OpenAiCompatAdapter::minimax("key".to_string());
        assert!(!adapter.is_openrouter());
    }

    #[test]
    fn to_openai_tools_conversion() {
        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: ToolInputSchema {
                schema_type: "object".to_string(),
                properties: serde_json::json!({
                    "path": {"type": "string"}
                }),
                required: vec!["path".to_string()],
            },
        }];

        let oai_tools = OpenAiCompatAdapter::to_openai_tools(&tools);
        assert_eq!(oai_tools.len(), 1);
        assert_eq!(oai_tools[0].function.name, "read_file");
    }

    #[test]
    fn base_url_trailing_slash_is_trimmed() {
        let a = OpenAiCompatAdapter::new("k".into(), "http://host:8000/v1/".into(), "m".into());
        assert_eq!(a.base_url, "http://host:8000/v1");
    }

    #[test]
    fn empty_request_model_falls_back_to_adapter_default() {
        let a = OpenAiCompatAdapter::minimax("k".into());
        assert_eq!(a.resolve_model(""), "MiniMax-M2.7");
        assert_eq!(a.resolve_model("   "), "MiniMax-M2.7");
        assert_eq!(a.resolve_model("other"), "other");
    }

    /// Regression guard for the defect this move fixes: the hex-agent
    /// original had nowhere to put `temperature`, so it never reached the
    /// provider.
    #[test]
    fn body_forwards_temperature_and_max_tokens() {
        let a = OpenAiCompatAdapter::minimax("k".into());
        let mut r = req("m");
        r.temperature = 0.7;
        r.max_tokens = 1234;
        let body = a.build_body(&r);
        assert_eq!(body["temperature"].as_f64().unwrap(), 0.7_f32 as f64);
        assert_eq!(body["max_tokens"].as_u64().unwrap(), 1234);
        assert_eq!(body["model"].as_str().unwrap(), "m");
    }

    #[test]
    fn openrouter_body_carries_routing_preferences() {
        let a = OpenAiCompatAdapter::openrouter("k".into(), "m".into());
        let body = a.build_body(&req(""));
        assert_eq!(body["route"].as_str().unwrap(), "fallback");
        assert!(body["provider"]["allow_fallbacks"].as_bool().unwrap());
        assert_eq!(body["model"].as_str().unwrap(), "m");
    }

    #[test]
    fn non_openrouter_body_has_no_routing_preferences() {
        let a = OpenAiCompatAdapter::minimax("k".into());
        let body = a.build_body(&req("m"));
        assert!(body.get("route").is_none());
        assert!(body.get("provider").is_none());
    }
}
