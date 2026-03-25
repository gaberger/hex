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

use crate::ports::secret_grant::ISecretGrantPort;
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
    // If a model is requested, find the provider that serves it; otherwise use first provider.
    let endpoint: Option<crate::routes::secrets::InferenceEndpointEntry> =
        if let Some(ref stdb) = state.inference_stdb {
            match stdb.list_providers().await {
                Ok(providers) if !providers.is_empty() => {
                    // Find the provider that matches the requested model
                    let matched = if let Some(ref requested_model) = body.model {
                        // 1. Exact model match
                        providers.iter().find(|p| {
                            p.models_json.contains(requested_model.as_str())
                        })
                        // 2. For OpenRouter-format IDs (e.g. "google/gemini-2.0-flash-001"),
                        //    route through any registered openrouter provider with a key.
                        .or_else(|| {
                            if requested_model.contains('/') {
                                providers.iter().find(|p| {
                                    p.provider_type == "openrouter" && !p.api_key_ref.is_empty()
                                })
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    };
                    // Fall back to first provider if no model match
                    let p = matched.unwrap_or(&providers[0]);
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
        // Resolve secret key reference to actual value from vault
        if ep.requires_auth && !ep.secret_key.is_empty() && !ep.secret_key.starts_with("sk-") {
            let key_ref = ep.secret_key.clone();
            tracing::debug!(key_ref = %key_ref, "resolving secret key reference");
            // Try environment variable first, then vault
            if let Ok(val) = std::env::var(&key_ref) {
                tracing::debug!("resolved from env var");
                ep.secret_key = val;
            } else if let Some(ref stdb) = state.spacetime_secrets {
                match stdb.vault_get(&key_ref).await {
                    Ok(Some(val)) => {
                        tracing::debug!("resolved from vault");
                        ep.secret_key = val;
                    }
                    Ok(None) => {
                        tracing::warn!(key = %key_ref, "secret not found in vault");
                    }
                    Err(e) => {
                        tracing::warn!(key = %key_ref, error = %e, "vault_get failed");
                    }
                }
            } else {
                tracing::warn!("spacetime_secrets not available for vault resolution");
            }
        }
        match super::chat::call_inference_endpoint(&ep, &messages).await {
            Ok(resp) => Ok(resp),
            Err(ref e) if e.contains("insufficient credits") || e.contains("402") || e.contains("rate limited") || e.contains("429") => {
                // OpenRouter out of credits — retry with a registered :free provider.
                tracing::warn!(provider = %ep.provider, model = %ep.model, "insufficient credits — retrying with registered :free provider");
                let free_providers: Vec<_> = if let Some(ref stdb) = state.inference_stdb {
                    stdb.list_providers().await.ok()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|p| p.provider_type == "openrouter" && p.models_json.contains(":free"))
                        .collect()
                } else {
                    vec![]
                };
                // Try each :free provider in order until one succeeds.
                let mut fallback_result: Result<_, String> = Err("no :free providers registered".to_string());
                for fp in &free_providers {
                    let free_model = fp.models_json
                        .trim_start_matches('[').trim_end_matches(']')
                        .split(',').next().unwrap_or(&fp.models_json)
                        .trim().trim_matches('"').to_string();
                    // Resolve the secret key ref (same logic as the main path).
                    let resolved_key = if fp.api_key_ref.starts_with("sk-") {
                        fp.api_key_ref.clone()
                    } else {
                        let key_ref = &fp.api_key_ref;
                        if let Ok(val) = std::env::var(key_ref) {
                            val
                        } else if let Some(ref stdb) = state.spacetime_secrets {
                            stdb.vault_get(key_ref).await.ok()
                                .flatten()
                                .unwrap_or_else(|| fp.api_key_ref.clone())
                        } else {
                            fp.api_key_ref.clone()
                        }
                    };
                    let free_ep = crate::routes::secrets::InferenceEndpointEntry {
                        id: fp.provider_id.clone(),
                        url: fp.base_url.clone(),
                        provider: fp.provider_type.clone(),
                        model: free_model.clone(),
                        status: "unknown".into(),
                        requires_auth: !fp.api_key_ref.is_empty(),
                        secret_key: resolved_key,
                        health_checked_at: fp.last_health_check.clone(),
                    };
                    match super::chat::call_inference_endpoint(&free_ep, &messages).await {
                        Ok(resp) => { fallback_result = Ok(resp); break; }
                        Err(e2) => {
                            tracing::warn!(model = %free_model, error = %e2, ":free provider failed — trying next");
                            fallback_result = Err(format!("{} failed: {}", free_model, e2));
                        }
                    }
                }
                fallback_result.map_err(|e| e)
            }
            Err(e) => {
                tracing::warn!(provider = %ep.provider, error = %e, "Inference endpoint failed, trying Anthropic fallback");
                // Fallback to Anthropic only if the key looks like a real Anthropic key.
                if let Some(ref api_key) = state.anthropic_api_key {
                    if api_key.starts_with("sk-ant-") {
                        super::chat::call_anthropic(api_key, &messages).await
                    } else {
                        Err(format!(
                            "{} failed: {}; Anthropic key is not configured (got non-Anthropic key in ANTHROPIC_API_KEY)",
                            ep.provider, e
                        ))
                    }
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
