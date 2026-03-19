use crate::ports::{ApiRequestOptions, RateLimitHeaders};
use crate::ports::{ContentBlock, Message, StopReason, TokenUsage, ToolDefinition};
use crate::ports::anthropic::{AnthropicError, AnthropicPort, AnthropicResponse, StreamChunk};
use async_trait::async_trait;
use futures::stream::{self, Stream};
use reqwest::Client;
use serde::Deserialize;
use std::sync::Mutex;

/// Adapter for the Anthropic Messages API.
///
/// Uses raw reqwest + SSE parsing — zero SDK dependency.
/// Supports prompt caching (`cache_control`), extended thinking (`thinking`),
/// and rate limit header parsing for proactive throttling.
pub struct AnthropicAdapter {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    /// Whether prompt caching is enabled by default.
    enable_cache: bool,
    /// Last parsed rate limit headers (for the rate limiter adapter to read).
    last_rate_limit_headers: Mutex<Option<RateLimitHeaders>>,
}

impl AnthropicAdapter {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.anthropic.com".into(),
            model,
            enable_cache: false,
            last_rate_limit_headers: Mutex::new(None),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
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

    fn build_request_body(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        stream: bool,
        model_override: Option<&str>,
        options: Option<&ApiRequestOptions>,
    ) -> serde_json::Value {
        let model = model_override.unwrap_or(&self.model);
        let use_cache = options.map(|o| o.enable_cache).unwrap_or(self.enable_cache);

        // Build system block — with cache_control if caching is enabled.
        // The system prompt is the best caching target: it's large, static,
        // and sent on every request in a conversation.
        let system_value = if use_cache {
            serde_json::json!([{
                "type": "text",
                "text": system,
                "cache_control": { "type": "ephemeral" }
            }])
        } else {
            serde_json::json!(system)
        };

        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "system": system_value,
            "messages": messages,
            "stream": stream,
        });

        if !tools.is_empty() {
            if use_cache {
                // Cache tool definitions — they're static and large.
                // Add cache_control to the last tool (Anthropic caches up to that point).
                let mut tools_json = serde_json::to_value(tools).unwrap_or_default();
                if let Some(arr) = tools_json.as_array_mut() {
                    if let Some(last) = arr.last_mut() {
                        last["cache_control"] = serde_json::json!({ "type": "ephemeral" });
                    }
                }
                body["tools"] = tools_json;
            } else {
                body["tools"] = serde_json::to_value(tools).unwrap_or_default();
            }
        }

        // Extended thinking — requires anthropic-beta header and budget_tokens
        if let Some(opts) = options {
            if opts.thinking.enabled && opts.thinking.budget_tokens > 0 {
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": opts.thinking.budget_tokens
                });
            }
        }

        body
    }

    /// Parse rate limit headers from an HTTP response.
    fn parse_rate_limit_headers(headers: &reqwest::header::HeaderMap) -> RateLimitHeaders {
        let get_u32 = |name: &str| -> Option<u32> {
            headers.get(name)?.to_str().ok()?.parse().ok()
        };
        let get_u64 = |name: &str| -> Option<u64> {
            headers.get(name)?.to_str().ok()?.parse().ok()
        };

        RateLimitHeaders {
            rpm_limit: get_u32("anthropic-ratelimit-requests-limit"),
            rpm_remaining: get_u32("anthropic-ratelimit-requests-remaining"),
            input_tpm_limit: get_u64("anthropic-ratelimit-input-tokens-limit"),
            input_tpm_remaining: get_u64("anthropic-ratelimit-input-tokens-remaining"),
            output_tpm_limit: get_u64("anthropic-ratelimit-output-tokens-limit"),
            output_tpm_remaining: get_u64("anthropic-ratelimit-output-tokens-remaining"),
            retry_after_ms: headers.get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|s| s * 1000),
        }
    }

    /// Build the list of extra headers needed for this request.
    fn extra_headers(options: Option<&ApiRequestOptions>) -> Vec<(&'static str, String)> {
        let mut hdrs = vec![];
        if let Some(opts) = options {
            if opts.enable_cache {
                // Prompt caching requires the beta header
                hdrs.push(("anthropic-beta", "prompt-caching-2024-07-31".to_string()));
            }
            if opts.thinking.enabled {
                // Extended thinking also requires a beta header
                hdrs.push(("anthropic-beta", "extended-thinking-2025-04-11".to_string()));
            }
        }
        hdrs
    }
}

#[async_trait]
impl AnthropicPort for AnthropicAdapter {
    async fn send_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        model_override: Option<&str>,
        options: Option<&ApiRequestOptions>,
    ) -> Result<AnthropicResponse, AnthropicError> {
        let body = self.build_request_body(system, messages, tools, max_tokens, false, model_override, options);

        let mut req = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        // Add beta headers for caching / thinking
        for (name, value) in Self::extra_headers(options) {
            req = req.header(name, value);
        }

        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| AnthropicError::Http(e.to_string()))?;

        // Parse and store rate limit headers before consuming the response body
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
            return Err(AnthropicError::RateLimited {
                retry_after_ms: retry_after * 1000,
            });
        }

        if status >= 400 {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".into());
            return Err(AnthropicError::Api {
                status,
                message: text,
            });
        }

        let api_resp: ApiResponse = response
            .json()
            .await
            .map_err(|e| AnthropicError::Deserialize(e.to_string()))?;

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

        let stop_reason = match api_resp.stop_reason.as_deref() {
            Some("end_turn") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some("stop_sequence") => StopReason::StopSequence,
            _ => StopReason::EndTurn,
        };

        // Extract cache token counts from the API response
        let cache_read = api_resp.usage.cache_read_input_tokens.unwrap_or(0);
        let cache_write = api_resp.usage.cache_creation_input_tokens.unwrap_or(0);

        let usage = TokenUsage {
            input_tokens: api_resp.usage.input_tokens,
            output_tokens: api_resp.usage.output_tokens,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            ..Default::default()
        };

        Ok(AnthropicResponse {
            content,
            stop_reason,
            usage,
            model: api_resp.model,
        })
    }

    async fn stream_message(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDefinition],
        max_tokens: u32,
        model_override: Option<&str>,
        options: Option<&ApiRequestOptions>,
    ) -> Result<
        Box<dyn Stream<Item = Result<StreamChunk, AnthropicError>> + Send + Unpin>,
        AnthropicError,
    > {
        let body = self.build_request_body(system, messages, tools, max_tokens, true, model_override, options);

        let mut req = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        for (name, value) in Self::extra_headers(options) {
            req = req.header(name, value);
        }

        let response = req
            .json(&body)
            .send()
            .await
            .map_err(|e| AnthropicError::Http(e.to_string()))?;

        // Store rate limit headers
        let rate_headers = Self::parse_rate_limit_headers(response.headers());
        if let Ok(mut guard) = self.last_rate_limit_headers.lock() {
            *guard = Some(rate_headers);
        }

        let status = response.status().as_u16();
        if status >= 400 {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".into());
            return Err(AnthropicError::Api {
                status,
                message: text,
            });
        }

        // Read the full SSE body and parse events.
        // For a production implementation, this would use async byte streaming.
        // This simplified version buffers the response and yields parsed chunks.
        let text = response
            .text()
            .await
            .map_err(|e| AnthropicError::Stream(e.to_string()))?;

        let chunks = parse_sse_events(&text);
        Ok(Box::new(stream::iter(chunks)))
    }
}

/// Parse SSE event stream text into StreamChunk items.
fn parse_sse_events(raw: &str) -> Vec<Result<StreamChunk, AnthropicError>> {
    let mut chunks = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in raw.lines() {
        if line.starts_with("event: ") {
            current_event = line[7..].to_string();
        } else if line.starts_with("data: ") {
            current_data = line[6..].to_string();
        } else if line.is_empty() && !current_event.is_empty() {
            if let Some(chunk) = parse_sse_event(&current_event, &current_data) {
                chunks.push(Ok(chunk));
            }
            current_event.clear();
            current_data.clear();
        }
    }

    chunks
}

fn parse_sse_event(event: &str, data: &str) -> Option<StreamChunk> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;

    match event {
        "content_block_start" => {
            let block = json.get("content_block")?;
            let block_type = block.get("type")?.as_str()?;
            if block_type == "tool_use" {
                Some(StreamChunk::ToolUseStart {
                    id: block.get("id")?.as_str()?.to_string(),
                    name: block.get("name")?.as_str()?.to_string(),
                })
            } else {
                None
            }
        }
        "content_block_delta" => {
            let delta = json.get("delta")?;
            let delta_type = delta.get("type")?.as_str()?;
            match delta_type {
                "text_delta" => {
                    let text = delta.get("text")?.as_str()?.to_string();
                    Some(StreamChunk::TextDelta(text))
                }
                "input_json_delta" => {
                    let partial = delta.get("partial_json")?.as_str()?.to_string();
                    Some(StreamChunk::InputJsonDelta(partial))
                }
                _ => None,
            }
        }
        "message_delta" => {
            let delta = json.get("delta")?;
            let stop_reason = match delta.get("stop_reason")?.as_str()? {
                "end_turn" => StopReason::EndTurn,
                "tool_use" => StopReason::ToolUse,
                "max_tokens" => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            };
            let usage_obj = json.get("usage")?;
            let usage = TokenUsage {
                output_tokens: usage_obj
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                ..Default::default()
            };
            Some(StreamChunk::MessageStop { stop_reason, usage })
        }
        _ => None,
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

#[derive(Debug, Deserialize)]
struct ApiUsage {
    input_tokens: u32,
    output_tokens: u32,
    /// Tokens read from the prompt cache (present when caching is enabled)
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    /// Tokens written to the prompt cache on first request
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
}
