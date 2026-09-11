//! Anthropic Messages API adapter — raw reqwest, no SDK.
//!
//! Salvaged out of `hex-agent/src/adapters/secondary/anthropic.rs` per
//! ADR-2608241500 P2.3. Prompt caching (`cache_control`), extended thinking
//! (`budget_tokens`), SSE parsing, and rate-limit header capture are carried
//! over. What changed is the contract: the original implemented hex-agent's
//! private `AnthropicPort`; this one implements [`IInferencePort`] so the
//! whole workspace speaks one inference shape and no caller names a provider.
//!
//! # Mapping from `InferenceRequest`
//!
//! | Request field     | Anthropic body                                  |
//! |-------------------|-------------------------------------------------|
//! | `cache_control`   | `cache_control: ephemeral` on system + last tool |
//! | `thinking_budget` | `thinking: { type: enabled, budget_tokens }`     |
//! | `temperature`     | `temperature`                                    |
//! | `grammar`         | ignored — Anthropic has no GBNF equivalent       |

use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hex_core::domain::api_optimization::RateLimitHeaders;
use hex_core::domain::messages::{ContentBlock, StopReason};
use hex_core::domain::tools::ToolDefinition;
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
};
use reqwest::Client;
use serde::Deserialize;

use super::vec_stream::VecStream;

/// Default request timeout. Overridable with `HEX_INFERENCE_TIMEOUT_SECS`.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Adapter for the Anthropic Messages API.
pub struct AnthropicAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    /// Whether prompt caching is enabled by default, when the request does
    /// not ask for it explicitly.
    enable_cache: bool,
    /// Last parsed rate limit headers, for callers that throttle proactively.
    last_rate_limit_headers: Mutex<Option<RateLimitHeaders>>,
}

impl AnthropicAdapter {
    pub fn new(api_key: String, model: String) -> Self {
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
            base_url: "https://api.anthropic.com".into(),
            model,
            enable_cache: false,
            last_rate_limit_headers: Mutex::new(None),
        }
    }

    /// Build from `ANTHROPIC_API_KEY`. Returns `None` when the key is absent,
    /// so the composition step can fall back to another provider instead of
    /// constructing an adapter that will 401 on first use.
    pub fn from_env(model: &str) -> Option<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY").ok().filter(|k| !k.is_empty())?;
        let mut adapter = Self::new(key, model.to_string());
        if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
            adapter.base_url = url.trim_end_matches('/').to_string();
        }
        Some(adapter)
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url.trim_end_matches('/').to_string();
        self
    }

    /// Enable prompt caching by default for all requests.
    pub fn with_cache(mut self, enabled: bool) -> Self {
        self.enable_cache = enabled;
        self
    }

    /// Get the last rate limit headers from the most recent API response.
    pub fn last_rate_limit_headers(&self) -> Option<RateLimitHeaders> {
        self.last_rate_limit_headers.lock().ok()?.clone()
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

    fn build_request_body(&self, request: &InferenceRequest, stream: bool) -> serde_json::Value {
        let model = self.resolve_model(&request.model);
        let use_cache = request.cache_control || self.enable_cache;

        // The system prompt is the best caching target: large, static, and
        // sent on every request in a conversation.
        let system_value = if use_cache {
            serde_json::json!([{
                "type": "text",
                "text": request.system_prompt,
                "cache_control": { "type": "ephemeral" }
            }])
        } else {
            serde_json::json!(request.system_prompt)
        };

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
            "system": system_value,
            "messages": request.messages,
            "stream": stream,
        });

        if !request.tools.is_empty() {
            body["tools"] = Self::tools_json(&request.tools, use_cache);
        }

        // Extended thinking — needs the beta header and a positive budget.
        if let Some(budget) = request.thinking_budget.filter(|b| *b > 0) {
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": budget
            });
        }

        body
    }

    /// Serialize tool definitions, marking the last one for caching when
    /// caching is on — Anthropic caches the prefix up to that marker.
    fn tools_json(tools: &[ToolDefinition], use_cache: bool) -> serde_json::Value {
        let mut tools_json = serde_json::to_value(tools).unwrap_or_default();
        if use_cache {
            if let Some(arr) = tools_json.as_array_mut() {
                if let Some(last) = arr.last_mut() {
                    last["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                }
            }
        }
        tools_json
    }

    /// Parse rate limit headers from an HTTP response.
    fn parse_rate_limit_headers(headers: &reqwest::header::HeaderMap) -> RateLimitHeaders {
        let get_u32 = |name: &str| -> Option<u32> { headers.get(name)?.to_str().ok()?.parse().ok() };
        let get_u64 = |name: &str| -> Option<u64> { headers.get(name)?.to_str().ok()?.parse().ok() };

        RateLimitHeaders {
            rpm_limit: get_u32("anthropic-ratelimit-requests-limit"),
            rpm_remaining: get_u32("anthropic-ratelimit-requests-remaining"),
            input_tpm_limit: get_u64("anthropic-ratelimit-input-tokens-limit"),
            input_tpm_remaining: get_u64("anthropic-ratelimit-input-tokens-remaining"),
            output_tpm_limit: get_u64("anthropic-ratelimit-output-tokens-limit"),
            output_tpm_remaining: get_u64("anthropic-ratelimit-output-tokens-remaining"),
            retry_after_ms: headers
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|s| s * 1000),
        }
    }

    /// Beta headers required by the features this request switches on.
    fn extra_headers(request: &InferenceRequest, default_cache: bool) -> Vec<(&'static str, String)> {
        let mut hdrs = vec![];
        if request.cache_control || default_cache {
            hdrs.push(("anthropic-beta", "prompt-caching-2024-07-31".to_string()));
        }
        if request.thinking_budget.filter(|b| *b > 0).is_some() {
            hdrs.push(("anthropic-beta", "extended-thinking-2025-04-11".to_string()));
        }
        hdrs
    }

    /// POST to `/v1/messages`, capture rate-limit headers, and map HTTP
    /// status onto [`InferenceError`]. Shared by `complete` and `stream`.
    async fn post_messages(
        &self,
        request: &InferenceRequest,
        stream: bool,
    ) -> Result<reqwest::Response, InferenceError> {
        let body = self.build_request_body(request, stream);

        let mut req = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        for (name, value) in Self::extra_headers(request, self.enable_cache) {
            req = req.header(name, value);
        }

        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| InferenceError::ProviderUnavailable(format!("{}: {}", self.base_url, e)))?;

        // Capture rate limit headers before the body is consumed.
        let rate_headers = Self::parse_rate_limit_headers(response.headers());
        if let Ok(mut guard) = self.last_rate_limit_headers.lock() {
            *guard = Some(rate_headers);
        }

        let status = response.status().as_u16();
        if status == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1);
            return Err(InferenceError::RateLimited(format!(
                "retry after {}s",
                retry_after
            )));
        }
        if status == 404 {
            return Err(InferenceError::UnknownProvider(format!(
                "anthropic has no model '{}'",
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

        Ok(response)
    }
}

#[async_trait]
impl IInferencePort for AnthropicAdapter {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let started = Instant::now();
        let response = self.post_messages(&request, false).await?;
        let status = response.status().as_u16();

        let api_resp: ApiResponse = response.json().await.map_err(|e| InferenceError::ApiError {
            status,
            body: format!("malformed messages response: {}", e),
        })?;

        let content = api_resp
            .content
            .into_iter()
            .map(|block| match block {
                ApiContentBlock::Text { text } => ContentBlock::Text { text },
                ApiContentBlock::ToolUse { id, name, input } => {
                    ContentBlock::ToolUse { id, name, input }
                }
            })
            .collect();

        Ok(InferenceResponse {
            content,
            model_used: api_resp.model,
            stop_reason: stop_reason_from(api_resp.stop_reason.as_deref()),
            input_tokens: api_resp.usage.input_tokens as u64,
            output_tokens: api_resp.usage.output_tokens as u64,
            cache_read_tokens: api_resp.usage.cache_read_input_tokens.unwrap_or(0) as u64,
            cache_write_tokens: api_resp.usage.cache_creation_input_tokens.unwrap_or(0) as u64,
            latency_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        request: InferenceRequest,
    ) -> Result<Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>, InferenceError>
    {
        let response = self.post_messages(&request, true).await?;

        // Buffer the SSE body, then parse it into chunks. Incremental byte
        // streaming is a later refinement; the chunk sequence is identical
        // either way, so callers do not have to change when it lands.
        let text = response
            .text()
            .await
            .map_err(|e| InferenceError::Network(e.to_string()))?;

        Ok(Box::new(VecStream::new(parse_sse_events(&text))))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        if self.api_key.is_empty() {
            return Ok(HealthStatus::Unreachable {
                reason: "ANTHROPIC_API_KEY is not set".into(),
            });
        }

        let response = match self
            .client
            .get(format!("{}/v1/models", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(HealthStatus::Unreachable {
                    reason: format!("{}: {}", self.base_url, e),
                })
            }
        };

        let status = response.status().as_u16();
        if status == 401 || status == 403 {
            return Ok(HealthStatus::Unreachable {
                reason: format!("authentication rejected ({})", status),
            });
        }
        if status >= 400 {
            return Ok(HealthStatus::Degraded {
                reason: format!("GET /v1/models returned {}", status),
            });
        }

        let listing: ApiModelListing = match response.json().await {
            Ok(l) => l,
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
                provider: "anthropic".to_string(),
                tier: tier_for(&self.model),
                context_window: 200_000,
            }],
            supports_tool_use: true,
            supports_thinking: true,
            supports_caching: true,
            supports_streaming: true,
            max_context_tokens: 200_000,
            // Left at zero deliberately: per-model pricing belongs in
            // `.hex/project.json`, not hardcoded where it silently goes stale.
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

/// Best-effort tier from a model id. Unknown ids report `Sonnet`, the
/// middle tier, rather than pretending to know.
fn tier_for(model: &str) -> ModelTier {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        ModelTier::Opus
    } else if m.contains("haiku") {
        ModelTier::Haiku
    } else {
        ModelTier::Sonnet
    }
}

fn stop_reason_from(raw: Option<&str>) -> StopReason {
    match raw {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

/// Parse an SSE event stream into `StreamChunk` items.
fn parse_sse_events(raw: &str) -> Vec<StreamChunk> {
    let mut chunks = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            current_event = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("data: ") {
            current_data = rest.to_string();
        } else if line.is_empty() && !current_event.is_empty() {
            chunks.extend(parse_sse_event(&current_event, &current_data));
            current_event.clear();
            current_data.clear();
        }
    }
    // A final event with no trailing blank line still counts.
    if !current_event.is_empty() {
        chunks.extend(parse_sse_event(&current_event, &current_data));
    }

    chunks
}

/// One SSE event may produce zero, one, or two chunks — `message_delta`
/// carries both usage and the stop reason, which are separate variants in
/// `hex_core`'s `StreamChunk`.
fn parse_sse_event(event: &str, data: &str) -> Vec<StreamChunk> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
        return vec![];
    };

    match event {
        "content_block_start" => {
            let Some(block) = json.get("content_block") else {
                return vec![];
            };
            if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                return vec![];
            }
            match (
                block.get("id").and_then(|v| v.as_str()),
                block.get("name").and_then(|v| v.as_str()),
            ) {
                (Some(id), Some(name)) => vec![StreamChunk::ToolUseStart {
                    id: id.to_string(),
                    name: name.to_string(),
                }],
                _ => vec![],
            }
        }
        "content_block_delta" => {
            let Some(delta) = json.get("delta") else {
                return vec![];
            };
            match delta.get("type").and_then(|t| t.as_str()) {
                Some("text_delta") => delta
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|t| vec![StreamChunk::TextDelta(t.to_string())])
                    .unwrap_or_default(),
                Some("input_json_delta") => delta
                    .get("partial_json")
                    .and_then(|v| v.as_str())
                    .map(|t| vec![StreamChunk::InputJsonDelta(t.to_string())])
                    .unwrap_or_default(),
                _ => vec![],
            }
        }
        "message_delta" => {
            let mut out = Vec::new();
            if let Some(usage) = json.get("usage") {
                out.push(StreamChunk::Usage {
                    input_tokens: usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    output_tokens: usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                });
            }
            let stop = json
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(|v| v.as_str());
            out.push(StreamChunk::MessageStop(stop_reason_from(stop)));
            out
        }
        _ => vec![],
    }
}

// --- API response types (private, for deserialization only) ---

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ApiContentBlock>,
    model: String,
    stop_reason: Option<String>,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ApiContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize, Default)]
struct ApiUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    /// Tokens read from the prompt cache (present when caching is enabled)
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    /// Tokens written to the prompt cache on the first request
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ApiModelListing {
    #[serde(default)]
    data: Vec<ApiModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ApiModelEntry {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::messages::Message;
    use hex_core::domain::tools::ToolInputSchema;
    use hex_core::ports::inference::Priority;

    fn req() -> InferenceRequest {
        InferenceRequest {
            model: String::new(),
            system_prompt: "sys".into(),
            messages: vec![Message::user("hi")],
            tools: vec![],
            max_tokens: 512,
            temperature: 0.2,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Normal,
            grammar: None,
        }
    }

    fn tool() -> ToolDefinition {
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a file".into(),
            input_schema: ToolInputSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({"path": {"type": "string"}}),
                required: vec!["path".into()],
            },
        }
    }

    #[test]
    fn empty_request_model_falls_back_to_adapter_default() {
        let a = AnthropicAdapter::new("k".into(), "claude-opus-5".into());
        assert_eq!(a.resolve_model(""), "claude-opus-5");
        assert_eq!(a.resolve_model("claude-haiku-4-5-20251001"), "claude-haiku-4-5-20251001");
    }

    #[test]
    fn cache_off_sends_a_plain_system_string() {
        let a = AnthropicAdapter::new("k".into(), "claude-opus-5".into());
        let body = a.build_request_body(&req(), false);
        assert_eq!(body["system"].as_str().unwrap(), "sys");
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn cache_on_sends_a_system_block_with_cache_control() {
        let a = AnthropicAdapter::new("k".into(), "claude-opus-5".into());
        let mut r = req();
        r.cache_control = true;
        let body = a.build_request_body(&r, false);
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["system"][0]["text"], "sys");
    }

    #[test]
    fn cache_on_marks_only_the_last_tool() {
        let tools = vec![tool(), tool()];
        let json = AnthropicAdapter::tools_json(&tools, true);
        let arr = json.as_array().unwrap();
        assert!(arr[0].get("cache_control").is_none());
        assert_eq!(arr[1]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn thinking_budget_enables_extended_thinking_and_its_beta_header() {
        let a = AnthropicAdapter::new("k".into(), "claude-opus-5".into());
        let mut r = req();
        r.thinking_budget = Some(8000);
        let body = a.build_request_body(&r, false);
        assert_eq!(body["thinking"]["budget_tokens"].as_u64().unwrap(), 8000);
        let hdrs = AnthropicAdapter::extra_headers(&r, false);
        assert!(hdrs.iter().any(|(_, v)| v.contains("extended-thinking")));
    }

    #[test]
    fn zero_thinking_budget_does_not_enable_thinking() {
        let a = AnthropicAdapter::new("k".into(), "claude-opus-5".into());
        let mut r = req();
        r.thinking_budget = Some(0);
        let body = a.build_request_body(&r, false);
        assert!(body.get("thinking").is_none());
        assert!(AnthropicAdapter::extra_headers(&r, false).is_empty());
    }

    /// Regression guard for the defect this move fixes: the hex-agent
    /// original had nowhere to put `temperature`.
    #[test]
    fn body_forwards_temperature() {
        let a = AnthropicAdapter::new("k".into(), "claude-opus-5".into());
        let mut r = req();
        r.temperature = 0.9;
        let body = a.build_request_body(&r, false);
        assert_eq!(body["temperature"].as_f64().unwrap(), 0.9_f32 as f64);
    }

    #[test]
    fn tier_is_read_from_the_model_id() {
        assert!(matches!(tier_for("claude-opus-5"), ModelTier::Opus));
        assert!(matches!(tier_for("claude-haiku-4-5-20251001"), ModelTier::Haiku));
        assert!(matches!(tier_for("claude-sonnet-5"), ModelTier::Sonnet));
        assert!(matches!(tier_for("something-else"), ModelTier::Sonnet));
    }

    #[test]
    fn sse_text_deltas_parse_in_order() {
        let raw = "event: content_block_delta\n\
                   data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"He\"}}\n\
                   \n\
                   event: content_block_delta\n\
                   data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"llo\"}}\n\
                   \n";
        let chunks = parse_sse_events(raw);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], StreamChunk::TextDelta(t) if t == "He"));
        assert!(matches!(&chunks[1], StreamChunk::TextDelta(t) if t == "llo"));
    }

    #[test]
    fn sse_tool_use_start_parses() {
        let raw = "event: content_block_start\n\
                   data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"read_file\"}}\n\
                   \n";
        let chunks = parse_sse_events(raw);
        assert!(
            matches!(&chunks[0], StreamChunk::ToolUseStart { id, name } if id == "t1" && name == "read_file")
        );
    }

    #[test]
    fn sse_message_delta_yields_usage_then_stop() {
        let raw = "event: message_delta\n\
                   data: {\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"input_tokens\":10,\"output_tokens\":7}}\n\
                   \n";
        let chunks = parse_sse_events(raw);
        assert_eq!(chunks.len(), 2);
        assert!(
            matches!(chunks[0], StreamChunk::Usage { input_tokens: 10, output_tokens: 7 })
        );
        assert!(matches!(chunks[1], StreamChunk::MessageStop(StopReason::ToolUse)));
    }

    /// The hex-agent original dropped a trailing event when the stream did
    /// not end with a blank line.
    #[test]
    fn sse_final_event_without_trailing_blank_line_is_not_dropped() {
        let raw = "event: content_block_delta\n\
                   data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"tail\"}}";
        let chunks = parse_sse_events(raw);
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::TextDelta(t) if t == "tail"));
    }

    #[test]
    fn unparseable_sse_data_is_skipped_not_panicked() {
        let raw = "event: content_block_delta\ndata: not json\n\n";
        assert!(parse_sse_events(raw).is_empty());
    }

    #[test]
    fn stop_reason_mapping_covers_every_documented_value() {
        assert!(matches!(stop_reason_from(Some("end_turn")), StopReason::EndTurn));
        assert!(matches!(stop_reason_from(Some("tool_use")), StopReason::ToolUse));
        assert!(matches!(stop_reason_from(Some("max_tokens")), StopReason::MaxTokens));
        assert!(matches!(stop_reason_from(Some("stop_sequence")), StopReason::StopSequence));
        assert!(matches!(stop_reason_from(None), StopReason::EndTurn));
    }

    #[test]
    fn rate_limit_headers_are_parsed_from_the_response() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("anthropic-ratelimit-requests-remaining", "42".parse().unwrap());
        h.insert("anthropic-ratelimit-input-tokens-limit", "1000".parse().unwrap());
        h.insert("retry-after", "3".parse().unwrap());
        let parsed = AnthropicAdapter::parse_rate_limit_headers(&h);
        assert_eq!(parsed.rpm_remaining, Some(42));
        assert_eq!(parsed.input_tpm_limit, Some(1000));
        assert_eq!(parsed.retry_after_ms, Some(3000));
    }

    #[test]
    fn base_url_trailing_slash_is_trimmed() {
        let a = AnthropicAdapter::new("k".into(), "m".into())
            .with_base_url("https://proxy.example/".into());
        assert_eq!(a.base_url, "https://proxy.example");
    }
}
