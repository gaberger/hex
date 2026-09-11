//! Provider transport — the raw HTTP calls behind every inference request.
//!
//! Lifted verbatim out of `hex-nexus/src/routes/chat.rs` per ADR-2608241500
//! P2.4. These functions were already pure: they take an endpoint descriptor
//! and a message array, and they touch no daemon state. Only their endpoint
//! type changed, from `routes::secrets::InferenceEndpointEntry` to
//! [`crate::endpoint::Endpoint`].
//!
//! This is the low-level layer. It knows how to speak to one endpoint and
//! nothing about which endpoint to pick — that is [`crate::routing`] — or
//! what to do when one fails — that is [`crate::complete`].
//!
//! # Why these speak JSON rather than the typed port
//!
//! `IInferencePort` (and the adapters in [`crate::providers`]) is the typed
//! contract for a *known* provider. This path serves the opposite case: an
//! endpoint discovered at runtime from `~/.hex/inference-servers.json`, whose
//! family is a string. The two coexist deliberately; P2.4 does not merge them.

use std::time::Duration;

use serde_json::json;

use crate::endpoint::Endpoint;

pub type InferenceResult = (String, String, u64, u64, String);

/// Translate an Anthropic-shaped tool schema entry
/// `{name, description, input_schema}` to the OpenAI function-calling
/// shape `{type: "function", function: {name, description, parameters}}`.
///
/// Used by `call_inference_endpoint_with_tools` so the same
/// ToolRegistry::anthropic_schema() that simple_agent emits also flows
/// through OpenRouter (which speaks OpenAI-compat).
/// Build the OpenAI-compatible chat/completions URL from a provider's
/// configured base URL. Used by both `call_inference_endpoint` and
/// `call_inference_endpoint_with_tools`. Recognizes the common compat
/// roots so we don't double-append `/v1`:
///
///   base                                                  | result suffix
///   ------------------------------------------------------|---------------
///   https://api.openai.com/v1                             | /chat/completions
///   https://generativelanguage.googleapis.com/v1beta/openai | /chat/completions
///   https://example.com/openai                            | /chat/completions
///   http://localhost:8000   (vLLM bare host)              | /v1/chat/completions
///
/// Without the suffix recognition, the Google endpoint expands to
/// `.../v1beta/openai/v1/chat/completions` (404) — the operator-reported
/// "url does not work" failure mode from 2026-05-21.
pub fn openai_compat_chat_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let already_compat_root = base.ends_with("/v1")
        || base.ends_with("/openai")
        || base.ends_with("/v1beta/openai");
    if already_compat_root {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    }
}

fn anthropic_tool_to_openai(t: &serde_json::Value) -> serde_json::Value {
    let name = t.get("name").cloned().unwrap_or(serde_json::Value::Null);
    let description = t.get("description").cloned().unwrap_or(serde_json::Value::Null);
    let parameters = t
        .get("input_schema")
        .cloned()
        .or_else(|| t.get("parameters").cloned())
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
    serde_json::json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    })
}


/// Pick Ollama's `num_ctx` (input context window). Ollama's default is only
/// ~2-8k, which SILENTLY truncates large prompts — a Claude Code request is
/// ~36k tokens (big system prompt + tool schemas), so the prompt fills the
/// window and the model emits ~1 token. We size `num_ctx` to the actual prompt
/// (≈chars/4 + generation headroom), bucketed to powers of two so same-size
/// workloads reuse one loaded model (Ollama reloads when num_ctx changes).
/// `HEX_OLLAMA_NUM_CTX` is a hard override for operators who want it fixed.
fn pick_num_ctx(payload_chars: usize, num_predict: u32) -> u32 {
    if let Ok(n) = std::env::var("HEX_OLLAMA_NUM_CTX").ok().and_then(|v| v.parse::<u32>().ok()).ok_or(()) {
        return n;
    }
    let est_prompt_tokens = (payload_chars / 4) as u32;
    // prompt + output + 25% headroom for tokenizer drift / chat-template overhead.
    let needed = est_prompt_tokens
        .saturating_add(num_predict)
        .saturating_add(est_prompt_tokens / 4)
        .saturating_add(1024);
    // Smallest bucket that fits; clamp to a sane floor/ceiling.
    for bucket in [8192u32, 16384, 32768, 65536, 131072] {
        if needed <= bucket {
            return bucket;
        }
    }
    131072
}

/// Like `call_inference_endpoint` but propagates a tools schema to the
/// provider and extracts `tool_calls` from the response. Returns
/// (InferenceResult, tool_calls_array). When the model emits no tool
/// calls the second element is an empty Vec.
///
/// Supported provider families (OpenAI-compatible tools schema):
///   - openrouter (HTTP POST {url}/chat/completions with OpenAI shape)
///   - openai / openai-compat / vllm (HTTP POST {url}/v1/chat/completions)
///   - ollama (HTTP POST {url}/api/chat with `tools` field)
///
/// Anything else: delegates to the no-tools `call_inference_endpoint`
/// path and returns Vec::new() for tool_calls — caller's text-mode
/// parser is the fallback.
/// Translate Anthropic-style messages (string OR array content with text/
/// tool_use/tool_result blocks) into Ollama's `/api/chat` shape:
///   - string content passes through unchanged;
///   - an assistant array turn → `{role:"assistant", content:<text>, tool_calls:[{function:{name,arguments}}]}`;
///   - a user array turn's `tool_result` blocks → separate `{role:"tool", content:<string>}` messages.
///
/// Without this, the react loop's multi-turn turns (which carry array content
/// after the first tool call) make Ollama 400 with
/// "cannot unmarshal array into ... content of type string" — so no local model
/// can complete an agentic loop (diagnostic 2026-06-07).
fn anthropic_messages_to_ollama(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        match m.get("content") {
            Some(serde_json::Value::String(s)) => {
                out.push(json!({ "role": role, "content": s }));
            }
            Some(serde_json::Value::Array(blocks)) => {
                let mut text = String::new();
                let mut tool_calls: Vec<serde_json::Value> = Vec::new();
                let mut tool_results: Vec<String> = Vec::new();
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                            }
                        }
                        Some("tool_use") => {
                            tool_calls.push(json!({
                                "function": {
                                    "name": b.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                                    "arguments": b.get("input").cloned().unwrap_or_else(|| json!({})),
                                }
                            }));
                        }
                        Some("tool_result") => {
                            let s = match b.get("content") {
                                Some(serde_json::Value::String(s)) => s.clone(),
                                Some(v) => serde_json::to_string(v).unwrap_or_default(),
                                None => String::new(),
                            };
                            tool_results.push(s);
                        }
                        _ => {}
                    }
                }
                if role == "assistant" {
                    let mut msg = json!({ "role": "assistant", "content": text });
                    if !tool_calls.is_empty() {
                        msg["tool_calls"] = json!(tool_calls);
                    }
                    out.push(msg);
                } else {
                    if !text.is_empty() {
                        out.push(json!({ "role": "user", "content": text }));
                    }
                    for tr in tool_results {
                        out.push(json!({ "role": "tool", "content": tr }));
                    }
                }
            }
            _ => out.push(json!({ "role": role, "content": "" })),
        }
    }
    out
}

pub async fn call_inference_endpoint_with_tools(
    ep: &Endpoint,
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
) -> Result<(InferenceResult, Vec<serde_json::Value>), String> {
    if !ep.supports_tools() {
        // Unknown provider — fall back to no-tools. Caller's text-mode
        // parser still gets a chance to surface structured output.
        let r = call_inference_endpoint(ep, messages).await?;
        return Ok((r, Vec::new()));
    }

    let p = ep.provider.to_ascii_lowercase();
    let is_openrouter = p == "openrouter" || ep.url.contains("openrouter.ai");
    let is_ollama = matches!(p.as_str(), "ollama");

    // Local providers (Ollama / vLLM) may need 5-10 min to cold-load
    // a quantized weight on first use. Remote APIs are capped at 4 min.
    let inference_timeout = if matches!(p.as_str(), "ollama" | "vllm" | "llama-cpp" | "llamacpp") {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(240)
    };

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(inference_timeout)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let model = if ep.model.is_empty() { "default".to_string() } else { ep.model.clone() };

    let openai_tools: Vec<serde_json::Value> = tools
        .iter()
        .map(anthropic_tool_to_openai)
        .collect();

    // Build URL + body per provider family.
    let (url, body) = if is_openrouter {
        // ep.url typically `https://openrouter.ai/api/v1`. Append
        // /chat/completions. Tolerate trailing slash.
        let base = ep.url.trim_end_matches('/');
        (
            format!("{base}/chat/completions"),
            json!({
                "model": model,
                "messages": messages,
                "max_tokens": 4096,
                "tools": openai_tools,
            }),
        )
    } else if is_ollama {
        // Ollama exposes /api/chat with native tool-calling since 0.3.x.
        // Response shape differs from OpenAI: tool_calls live under
        // `message.tool_calls`, not `choices[0].message.tool_calls`.
        let base = ep.url.trim_end_matches('/');
        let num_predict: u32 = std::env::var("HEX_OLLAMA_NUM_PREDICT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2048);
        // Translate Anthropic content-blocks → Ollama's string-content + tool
        // message shape, or multi-turn agentic turns 400 (see fn doc).
        let ollama_messages = anthropic_messages_to_ollama(messages);
        // num_ctx sized to the actual prompt (messages + tool schemas) so large
        // agentic clients like Claude Code aren't silently truncated. See
        // pick_num_ctx.
        let payload_chars = serde_json::to_string(&ollama_messages).map(|s| s.len()).unwrap_or(0)
            + serde_json::to_string(&openai_tools).map(|s| s.len()).unwrap_or(0);
        let num_ctx = pick_num_ctx(payload_chars, num_predict);
        let think_enabled = std::env::var("HEX_OLLAMA_THINK")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        (
            format!("{base}/api/chat"),
            json!({
                "model": model,
                "messages": ollama_messages,
                "stream": false,
                "think": think_enabled,
                "tools": openai_tools,
                "options": { "num_predict": num_predict, "num_ctx": num_ctx },
            }),
        )
    } else {
        // openai / openai-compat / vllm — append /chat/completions.
        // Different vendors use different OpenAI-compat root paths;
        // openai_compat_chat_url handles all of them.
        let url = openai_compat_chat_url(&ep.url);
        // 16384 ceiling for reasoning models (see call_inference_endpoint).
        let max_tokens: u32 = std::env::var("HEX_OPENAI_MAX_TOKENS")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(16384);
        (
            url,
            json!({
                "model": model,
                "messages": messages,
                "max_tokens": max_tokens,
                "tools": openai_tools,
            }),
        )
    };

    let mut req = client.post(&url).json(&body);
    if is_openrouter {
        if ep.secret_key.is_empty() {
            return Err("openrouter: API key not configured".into());
        }
        req = req
            .bearer_auth(&ep.secret_key)
            .header("HTTP-Referer", "https://hex.local")
            .header("X-Title", "hex-nexus");
    } else if !ep.secret_key.is_empty() {
        req = req.bearer_auth(&ep.secret_key);
    }
    // Ollama doesn't need auth by default — leave header off.

    let resp = req.send().await.map_err(|e| format!("connection: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let err = resp.text().await.unwrap_or_default();
        // Preserve the specific OpenRouter 402 message so credit
        // exhaustion is visible in logs + dashboard.
        if is_openrouter {
            return match status.as_u16() {
                402 => Err("openrouter: insufficient credits — top up at https://openrouter.ai/credits".into()),
                429 => Err("openrouter: rate limited".into()),
                _ => Err(format!("openrouter HTTP {status}: {}", truncate_str(&err, 200))),
            };
        }
        return Err(format!("{p} HTTP {status}: {}", truncate_str(&err, 200)));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    if is_ollama {
        // Ollama shape: { "message": { "content": "...", "tool_calls": [...] }, "model": "...", "eval_count": N, "prompt_eval_count": N }
        let content = data["message"]["content"].as_str().unwrap_or("").to_string();
        let model_used = data["model"].as_str().unwrap_or(&model).to_string();
        let input_tokens = data["prompt_eval_count"].as_u64().unwrap_or(0);
        let output_tokens = data["eval_count"].as_u64().unwrap_or(0);
        // Ollama doesn't return cost (it's free); leave empty.
        let cost = String::new();
        let tool_calls: Vec<serde_json::Value> = data["message"]["tool_calls"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        return Ok(((content, model_used, input_tokens, output_tokens, cost), tool_calls));
    }

    // OpenAI-compat shape (OpenRouter, OpenAI, vLLM).
    let content_val = data["choices"][0]["message"]["content"].as_str();
    let reasoning_val = data["choices"][0]["message"]["reasoning"].as_str();
    // When the model emits tool_calls and no text, content may be null.
    let content = content_val.or(reasoning_val).unwrap_or("").to_string();
    let model_used = data["model"].as_str().unwrap_or(&model).to_string();
    let input_tokens = data["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = data["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    let cost = data["usage"]["cost"]
        .as_f64()
        .map(|c| format!("{:.8}", c))
        .unwrap_or_default();
    let tool_calls: Vec<serde_json::Value> = data["choices"][0]["message"]["tool_calls"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(((content, model_used, input_tokens, output_tokens, cost), tool_calls))
}

/// Call a registered inference endpoint (OpenAI-compatible /v1/chat/completions,
/// Ollama /api/chat, or OpenRouter /api/v1/chat/completions).
pub async fn call_inference_endpoint(
    ep: &Endpoint,
    messages: &[serde_json::Value],
) -> Result<InferenceResult, String> {
    let is_openrouter = ep.provider == "openrouter"
        || ep.url.contains("openrouter.ai");

    // Local providers (Ollama, vLLM) need a much longer timeout — cold model
    // load can take 5-10 minutes for large quantised weights. Remote APIs
    // (OpenRouter, Anthropic) are faster and capped at 4 minutes.
    let inference_timeout = if ep.provider == "ollama" || ep.provider == "vllm"
        || (!is_openrouter && !ep.url.contains("anthropic.com") && !ep.url.starts_with("https://api."))
    {
        Duration::from_secs(600)
    } else {
        Duration::from_secs(240)
    };

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(inference_timeout)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let model = if ep.model.is_empty() { "default".to_string() } else { ep.model.clone() };

    let (url, body) = if is_openrouter {
        let url = "https://openrouter.ai/api/v1/chat/completions".to_string();
        // Let OpenRouter choose the upstream provider — forcing specific providers
        // (Together/Fireworks/etc.) blocks free-tier routing which uses the model
        // creator's own infrastructure. Remove deprecated `route` field too.
        let body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": 4096,
        });
        (url, body)
    } else {
        match ep.provider.as_str() {
            "ollama" => {
                let url = format!("{}/api/chat", ep.url);
                let num_predict: u32 = std::env::var("HEX_OLLAMA_NUM_PREDICT")
                    .ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
                // num_ctx sized to the actual prompt (see pick_num_ctx) so large
                // prompts aren't silently truncated by Ollama's small default.
                let payload_chars = serde_json::to_string(messages).map(|s| s.len()).unwrap_or(0);
                let num_ctx = pick_num_ctx(payload_chars, num_predict);
                // CRITICAL: qwen3 family is a *thinking* model — by default
                // it spends all output tokens on <think> reasoning and emits
                // an EMPTY `message.content`. Every responder/drafter/twin
                // call was returning content='' because of this, looking
                // like silent success but landing nothing. Disable thinking
                // unless explicitly opted-in via HEX_OLLAMA_THINK=1.
                let think_enabled = std::env::var("HEX_OLLAMA_THINK")
                    .ok().map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                let body = json!({
                    "model": model,
                    "messages": messages,
                    "stream": false,
                    "think": think_enabled,
                    "options": { "num_predict": num_predict, "num_ctx": num_ctx },
                });
                (url, body)
            }
            _ => {
                // OpenAI-compatible (vLLM, Google Gemini compat layer, etc).
                let url = openai_compat_chat_url(&ep.url);
                // 16384 ceiling (not 4096): reasoning models served via this
                // path (Tenstorrent DeepSeek-R1, Qwen3-32B in thinking mode)
                // spend thousands of tokens reasoning before emitting the
                // answer; a 4096 cap truncated their output mid-string or left
                // it empty. max_tokens is a ceiling — models still stop at
                // natural completion — so a higher bound is safe. Override via
                // HEX_OPENAI_MAX_TOKENS.
                let max_tokens: u32 = std::env::var("HEX_OPENAI_MAX_TOKENS")
                    .ok().and_then(|s| s.parse().ok()).unwrap_or(16384);
                let body = json!({
                    "model": model,
                    "messages": messages,
                    "max_tokens": max_tokens,
                });
                (url, body)
            }
        }
    };

    let mut req = client.post(&url).json(&body);

    if is_openrouter {
        // OpenRouter requires its own API key + identification headers
        if ep.secret_key.is_empty() {
            return Err("OPENROUTER_API_KEY not set — register with `hex secrets set OPENROUTER_API_KEY <key>` then `hex inference add openrouter`".into());
        }
        req = req
            .header("Authorization", format!("Bearer {}", ep.secret_key))
            .header("HTTP-Referer", "https://github.com/hex-intf")
            .header("X-Title", "hex-agent");
    } else if !ep.secret_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", ep.secret_key));
    }

    let resp = req.send().await.map_err(|e| format!("connection: {e}"))?;
    let status = resp.status().as_u16();

    // OpenRouter-specific error handling
    if status >= 400 {
        if is_openrouter {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let err = resp.text().await.unwrap_or_default();
            return match status {
                402 => Err("OpenRouter: insufficient credits — top up at https://openrouter.ai/credits".into()),
                429 => {
                    let msg = if let Some(ra) = retry_after {
                        format!("OpenRouter: rate limited — retry after {ra}s")
                    } else {
                        "OpenRouter: rate limited — back off and retry".into()
                    };
                    Err(msg)
                }
                502 => {
                    // Try to extract which upstream provider failed
                    let detail = if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&err) {
                        parsed["error"]["message"]
                            .as_str()
                            .unwrap_or("upstream provider down")
                            .to_string()
                    } else {
                        "upstream provider down".to_string()
                    };
                    tracing::warn!(status = 502, detail = %detail, "OpenRouter upstream failure");
                    Err(format!("OpenRouter 502: {detail}"))
                }
                _ => Err(format!("OpenRouter HTTP {status}: {}", truncate_str(&err, 200))),
            };
        }
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {}", truncate_str(&err, 200)));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    match ep.provider.as_str() {
        "ollama" => {
            let raw = data["message"]["content"].as_str().unwrap_or("(empty)").to_string();
            // qwen3 / deepseek-r1 family wraps reasoning in <think>...</think>
            // and emits the actual answer afterward. Downstream parsers (the
            // responder's Confirm:/Silent gate, the drafter, the twin) all
            // expect the answer at the start. Strip the think block here so
            // every caller sees clean content.
            let content = strip_think_block(&raw);
            let model_used = data["model"].as_str().unwrap_or(&model).to_string();
            let prompt_tokens = data["prompt_eval_count"].as_u64().unwrap_or(0);
            let eval_tokens = data["eval_count"].as_u64().unwrap_or(0);
            Ok((content, model_used, prompt_tokens, eval_tokens, String::new()))
        }
        _ if is_openrouter => {
            // DeepSeek R1 and other reasoning models may return content=null with the
            // actual answer in the reasoning field when chain-of-thought is enabled.
            let content_val = data["choices"][0]["message"]["content"].as_str();
            let reasoning_val = data["choices"][0]["message"]["reasoning"].as_str();
            if content_val.is_none() && reasoning_val.is_none() {
                tracing::warn!(
                    response = %data.to_string(),
                    "OpenRouter returned null content and null reasoning — full response logged"
                );
                return Err(format!("null content: model '{}' returned null/empty response — triggering fallback", model));
            }
            let content = content_val
                .or(reasoning_val)
                .unwrap_or("(empty)").to_string();
            let model_used = data["model"].as_str().unwrap_or(&model).to_string();
            let input_tokens = data["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
            let output_tokens = data["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            // Extract actual cost from OpenRouter's usage.cost field
            let openrouter_cost = data["usage"]["cost"]
                .as_f64()
                .map(|c| format!("{:.8}", c))
                .unwrap_or_default();
            if !openrouter_cost.is_empty() {
                tracing::info!(
                    openrouter_cost_usd = %openrouter_cost,
                    model = %model_used,
                    "OpenRouter actual cost"
                );
            }
            Ok((content, model_used, input_tokens, output_tokens, openrouter_cost))
        }
        _ => {
            // OpenAI-compatible response format
            let content = data["choices"][0]["message"]["content"]
                .as_str().unwrap_or("(empty)").to_string();
            let model_used = data["model"].as_str().unwrap_or(&model).to_string();
            let input_tokens = data["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
            let output_tokens = data["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            Ok((content, model_used, input_tokens, output_tokens, String::new()))
        }
    }
}

/// Call Anthropic API directly
/// Inject Anthropic prompt-caching breakpoints into messages before sending.
///
/// Marks the system prompt and the first user message's content as cacheable.
/// This eliminates redundant token spend on repeated inference calls within the
/// same TDD feedback loop — the static prefix (system + context files) is sent
/// once and cached; only new turns are charged at full rate.
///
/// The `anthropic-beta: prompt-caching-2024-07-31` header must accompany these requests.
/// For non-Anthropic providers this function is not called.
fn inject_cache_controls(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut out = messages.to_vec();
    // Mark the first user message's content as a cache breakpoint. This is the
    // message that typically carries all static context (port interfaces, files).
    for msg in out.iter_mut() {
        if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
            match msg.get("content") {
                Some(serde_json::Value::String(s)) => {
                    let text = s.clone();
                    msg["content"] = json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": { "type": "ephemeral" }
                    }]);
                }
                Some(serde_json::Value::Array(blocks)) if !blocks.is_empty() => {
                    let mut blocks = blocks.clone();
                    if let Some(last) = blocks.last_mut() {
                        if last.get("cache_control").is_none() {
                            last["cache_control"] = json!({ "type": "ephemeral" });
                        }
                    }
                    msg["content"] = serde_json::Value::Array(blocks);
                }
                _ => {}
            }
            break; // only the first user message
        }
    }
    out
}

pub async fn call_anthropic(
    api_key: &str,
    messages: &[serde_json::Value],
) -> Result<InferenceResult, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    // Extract any role:system messages from the array — Anthropic requires them
    // at the top level `system` parameter, not as messages (HTTP 400 otherwise).
    let is_system = |m: &&serde_json::Value| {
        m.get("role").and_then(|r| r.as_str()) == Some("system")
    };
    let system_text: String = messages.iter()
        .filter(is_system)
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n\n");

    let system_text = if system_text.is_empty() {
        "You are a helpful AI assistant integrated into the hex architecture dashboard. Be concise and helpful.".to_string()
    } else {
        system_text
    };

    let non_system: Vec<serde_json::Value> = messages.iter()
        .filter(|m| !is_system(m))
        .cloned()
        .collect();
    let cached_messages = inject_cache_controls(&non_system);

    // System prompt uses the array format so Anthropic can cache the static prefix.
    let system_block = json!([{
        "type": "text",
        "text": system_text,
        "cache_control": { "type": "ephemeral" }
    }]);

    let body = json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 8192,
        "system": system_block,
        "messages": cached_messages,
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "prompt-caching-2024-07-31")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("connection: {e}"))?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let err = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {}", truncate_str(&err, 200)));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;
    let content = data["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|block| block["text"].as_str())
        .unwrap_or("(empty)")
        .to_string();
    let model = data["model"].as_str().unwrap_or("unknown").to_string();
    let input_tokens = data["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let output_tokens = data["usage"]["output_tokens"].as_u64().unwrap_or(0);
    let cache_read = data["usage"]["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let cache_created = data["usage"]["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    if cache_read > 0 || cache_created > 0 {
        tracing::debug!(cache_read_tokens = cache_read, cache_created_tokens = cache_created, "anthropic prompt cache stats");
    }
    Ok((content, model, input_tokens, output_tokens, String::new()))
}
pub fn strip_think_block(raw: &str) -> String {
    // Fast path: no think tag.
    if !raw.contains("<think>") && !raw.contains("</think>") {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    loop {
        match rest.find("<think>") {
            None => {
                // No more opens — but there may be a stray `</think>` from
                // a truncated reply where the open got eaten by streaming.
                // In that case, take everything AFTER the last `</think>`.
                if let Some(close) = rest.rfind("</think>") {
                    out.push_str(&rest[close + "</think>".len()..]);
                } else {
                    out.push_str(rest);
                }
                break;
            }
            Some(open_idx) => {
                out.push_str(&rest[..open_idx]);
                let after_open = &rest[open_idx + "<think>".len()..];
                match after_open.find("</think>") {
                    None => break, // unterminated — drop everything in/after the think
                    Some(close_idx) => {
                        rest = &after_open[close_idx + "</think>".len()..];
                    }
                }
            }
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod compat_url_tests {
    use super::openai_compat_chat_url;

    #[test]
    fn google_gemini_openai_compat_root() {
        // The 2026-05-21 "url does not work" report — Google's
        // OpenAI-compat root ends with /v1beta/openai; appending /v1
        // produces 404. We must just add /chat/completions.
        assert_eq!(
            openai_compat_chat_url("https://generativelanguage.googleapis.com/v1beta/openai"),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
    }

    #[test]
    fn openai_canonical_v1_root() {
        assert_eq!(
            openai_compat_chat_url("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn vllm_bare_host_gets_v1_prefix() {
        assert_eq!(
            openai_compat_chat_url("http://localhost:8000"),
            "http://localhost:8000/v1/chat/completions"
        );
    }

    #[test]
    fn generic_openai_path() {
        // Any path ending in /openai is treated as a compat root.
        assert_eq!(
            openai_compat_chat_url("https://example.com/openai"),
            "https://example.com/openai/chat/completions"
        );
    }

    #[test]
    fn trailing_slash_tolerated() {
        assert_eq!(
            openai_compat_chat_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            openai_compat_chat_url("https://generativelanguage.googleapis.com/v1beta/openai/"),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
    }
}

#[cfg(test)]
mod num_ctx_tests {
    use super::pick_num_ctx;
    #[test]
    fn small_prompt_uses_small_bucket() {
        // ~350 chars ≈ 90 tokens → smallest bucket.
        assert_eq!(pick_num_ctx(350, 1024), 8192);
    }
    #[test]
    fn claude_code_sized_prompt_gets_large_bucket() {
        // ~36k tokens ≈ 144k chars → needs the 65536 bucket (or larger).
        let n = pick_num_ctx(144_000, 2048);
        assert!(n >= 65536, "got {n}");
    }
    #[test]
    fn buckets_are_monotonic_in_prompt_size() {
        assert!(pick_num_ctx(40_000, 1024) <= pick_num_ctx(400_000, 1024));
    }
}

#[cfg(test)]
mod think_block_tests {
    use super::strip_think_block;
    #[test] fn no_think() { assert_eq!(strip_think_block("answer"), "answer"); }
    #[test] fn full_think() {
        assert_eq!(strip_think_block("<think>reasoning here</think>\nanswer"), "answer");
    }
    #[test] fn stray_close() {
        // Truncated mid-stream where <think> was eaten.
        assert_eq!(strip_think_block("reasoning here</think>\nanswer"), "answer");
    }
    #[test] fn unterminated_think() {
        assert_eq!(strip_think_block("<think>reasoning but no close"), "");
    }
    #[test] fn multiple_blocks() {
        assert_eq!(
            strip_think_block("<think>a</think>line1<think>b</think>line2"),
            "line1line2"
        );
    }
}

pub fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find the last char boundary at or before `max` to avoid panicking on multi-byte UTF-8
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

