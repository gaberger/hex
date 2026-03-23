//! HTTP inference endpoint — synchronous LLM completion for hex-agent.
//!
//! POST /api/inference/complete
//!
//! Routes through registered inference providers (Ollama, vLLM, OpenAI-compat)
//! with Anthropic as fallback, reusing the same logic as the WebSocket LLM bridge.

use axum::{extract::State, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct InferenceCompleteRequest {
    /// Model identifier (e.g. "llama3", "claude-sonnet-4-20250514").
    /// If omitted, the first registered provider's default model is used.
    pub model: Option<String>,
    /// Messages in OpenAI-compatible format: [{role, content}]
    pub messages: Vec<serde_json::Value>,
    /// System prompt (prepended as a system message if the provider supports it).
    #[serde(default)]
    pub system: Option<String>,
    /// Maximum tokens to generate.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    4096
}

#[derive(Debug, Serialize)]
pub struct InferenceCompleteResponse {
    pub content: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// POST /api/inference/complete — synchronous LLM completion.
///
/// Picks the best available inference provider (registered endpoints first,
/// then Anthropic fallback) and returns the full response.
pub async fn inference_complete(
    State(state): State<SharedState>,
    Json(body): Json<InferenceCompleteRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Build messages list, optionally prepending system prompt
    let mut messages = body.messages;
    if let Some(ref system) = body.system {
        if !system.is_empty() {
            messages.insert(0, json!({ "role": "system", "content": system }));
        }
    }

    // Try registered inference endpoints first (SpacetimeDB providers)
    let endpoint: Option<crate::routes::secrets::InferenceEndpointEntry> =
        if let Some(ref stdb) = state.inference_stdb {
            match stdb.list_providers().await {
                Ok(providers) if !providers.is_empty() => {
                    let p = &providers[0];
                    let first_model = p
                        .models_json
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .split(',')
                        .next()
                        .unwrap_or(&p.models_json)
                        .trim()
                        .trim_matches('"')
                        .to_string();
                    Some(crate::routes::secrets::InferenceEndpointEntry {
                        id: p.provider_id.clone(),
                        url: p.base_url.clone(),
                        provider: p.provider_type.clone(),
                        model: first_model,
                        status: if p.healthy == 1 {
                            "healthy".into()
                        } else {
                            "unknown".into()
                        },
                        requires_auth: !p.api_key_ref.is_empty(),
                        secret_key: p.api_key_ref.clone(),
                        health_checked_at: p.last_health_check.clone(),
                    })
                }
                _ => None,
            }
        } else {
            None
        };

    let result = if let Some(mut ep) = endpoint {
        // Apply requested model override
        if let Some(ref model) = body.model {
            ep.model = model.clone();
        }
        match super::chat::call_inference_endpoint(&ep, &messages).await {
            Ok(resp) => Ok(resp),
            Err(e) => {
                tracing::warn!(provider = %ep.provider, error = %e, "Inference endpoint failed, trying Anthropic fallback");
                // Fallback to Anthropic
                if let Some(ref api_key) = state.anthropic_api_key {
                    super::chat::call_anthropic(api_key, &messages).await
                } else {
                    Err(format!(
                        "{} failed: {}; no Anthropic fallback configured",
                        ep.provider, e
                    ))
                }
            }
        }
    } else if let Some(ref api_key) = state.anthropic_api_key {
        super::chat::call_anthropic(api_key, &messages).await
    } else {
        Err("No inference endpoints registered and no ANTHROPIC_API_KEY set".into())
    };

    match result {
        Ok((content, model, input_tokens, output_tokens, openrouter_cost)) => {
            tracing::info!(model = %model, input_tokens, output_tokens, "inference/complete OK");
            let mut resp = json!({
                "content": content,
                "model": model,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
            });
            if !openrouter_cost.is_empty() {
                resp["openrouter_cost_usd"] = json!(openrouter_cost);
            }
            (
                StatusCode::OK,
                Json(resp),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "inference/complete failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": e })),
            )
        }
    }
}
