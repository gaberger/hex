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

    // Resolve a synthetic OpenRouter endpoint from a key that may have been placed
    // in ANTHROPIC_API_KEY (sk-or-v1- prefix) or OPENROUTER_API_KEY.
    let openrouter_key: Option<String> = std::env::var("OPENROUTER_API_KEY").ok().or_else(|| {
        state.anthropic_api_key.as_ref()
            .filter(|k| k.starts_with("sk-or-v1-"))
            .cloned()
    });

    // Map the pseudo-model "openrouter/free" to a real, consistently-available free model.
    // openai/gpt-4o-mini is preferred — it respects OpenRouter privacy settings.
    let resolve_free_model = |requested: Option<&str>| -> String {
        match requested {
            Some(m) if m == "openrouter/free" || m.is_empty() => {
                "openai/gpt-4o-mini".to_string()
            }
            Some(m) => m.to_string(),
            None => "openai/gpt-4o-mini".to_string(),
        }
    };

    // Normalize bare Anthropic model IDs to OpenRouter vendor-namespaced format.
    // OpenRouter requires "anthropic/claude-sonnet-4-6", not "claude-sonnet-4-6".
    let normalize_for_openrouter = |model: &str| -> String {
        if model.starts_with("claude-") && !model.contains('/') {
            format!("anthropic/{}", model)
        } else {
            model.to_string()
        }
    };

    let result = if let Some(mut ep) = endpoint {
        // Apply requested model override, normalizing bare Anthropic IDs for OpenRouter.
        if let Some(ref model) = body.model {
            ep.model = if ep.provider == "openrouter" {
                normalize_for_openrouter(model)
            } else {
                model.clone()
            };
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
            Err(ref e) if e.contains("insufficient credits") || e.contains("402")
                || e.contains("rate limited") || e.contains("429")
                || e.contains("parse:") || e.contains("500") || e.contains("503")
                || e.contains("404") || e.contains("No endpoints") || e.contains("data policy") => {
                // OpenRouter transient failure (credits, rate limit, parse/server error),
                // or permanent failure (404 model-not-found / data policy).
                // For transient errors (parse/5xx), first retry the same endpoint after a
                // brief sleep — these are often momentary network hiccups.
                // For 404/policy errors, skip retry and go straight to fallback.
                let is_transient = (e.contains("parse:") || e.contains("500") || e.contains("503"))
                    && !e.contains("404") && !e.contains("No endpoints") && !e.contains("data policy");
                if is_transient {
                    tracing::warn!(provider = %ep.provider, model = %ep.model, error = %e,
                        "transient endpoint error — sleeping 5s then retrying same endpoint");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    match super::chat::call_inference_endpoint(&ep, &messages).await {
                        Ok(resp) => return (StatusCode::OK, Json(json!({
                            "content": resp.0, "model": resp.1,
                            "input_tokens": resp.2, "output_tokens": resp.3,
                        }))),
                        Err(ref e2) => tracing::warn!(error = %e2, "retry also failed — falling through to :free providers"),
                    }
                }
                // For rate-limit errors, back off before trying :free providers.
                let is_rate_limit = e.contains("rate limited") || e.contains("429");
                if is_rate_limit {
                    tracing::warn!(provider = %ep.provider, model = %ep.model,
                        "rate limited — sleeping 10s before :free retry");
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                }
                tracing::warn!(provider = %ep.provider, model = %ep.model, "retrying with registered :free provider");
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
                // Skip policy-blocked models immediately; back off on rate-limited ones.
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
                            // For rate-limited models, back off briefly before the next attempt.
                            // Skip this delay for policy/404 errors — they won't recover with time.
                            let is_rate_limit = e2.contains("rate limited") || e2.contains("429");
                            let is_permanent = e2.contains("data policy") || e2.contains("guardrail")
                                || (e2.contains("404") && !e2.contains("rate"));
                            if is_rate_limit && !is_permanent {
                                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                            }
                        }
                    }
                }
                // If no registered :free provider worked, try a chain of free models with the OR key.
                if fallback_result.is_err() {
                    if let Some(ref or_key) = openrouter_key {
                        // Models confirmed to support this account's privacy settings.
                        let free_candidates = [
                            "openai/gpt-4o-mini",
                            "arcee-ai/trinity-mini:free",
                            "mistralai/mistral-small-3.1-24b-instruct:free",
                            "meta-llama/llama-3.3-70b-instruct:free",
                            "meta-llama/llama-3.2-3b-instruct:free",
                        ];
                        for free_model in free_candidates {
                            tracing::info!(model = %free_model, "all :free providers exhausted — retrying with OpenRouter key + free model");
                            let synth = crate::routes::secrets::InferenceEndpointEntry {
                                id: "openrouter-key-free-fallback".into(),
                                url: "https://openrouter.ai/api/v1".into(),
                                provider: "openrouter".into(),
                                model: free_model.to_string(),
                                status: "unknown".into(),
                                requires_auth: true,
                                secret_key: or_key.clone(),
                                health_checked_at: String::new(),
                            };
                            fallback_result = super::chat::call_inference_endpoint(&synth, &messages).await;
                            if fallback_result.is_ok() {
                                break;
                            }
                            tracing::warn!(model = %free_model, "free model fallback failed — trying next");
                        }
                    }
                }
                // If all free models failed, try Anthropic direct as final fallback.
                if fallback_result.is_err() {
                    if let Some(ref api_key) = state.anthropic_api_key {
                        if api_key.starts_with("sk-ant-") {
                            tracing::info!("all free providers exhausted — falling back to Anthropic direct");
                            fallback_result = super::chat::call_anthropic(api_key, &messages).await;
                        }
                    }
                }
                fallback_result.map_err(|e| e)
            }
            Err(e) => {
                tracing::warn!(provider = %ep.provider, error = %e, "Inference endpoint failed, trying fallback");
                // Fallback hierarchy: Anthropic key → OpenRouter key → error.
                if let Some(ref api_key) = state.anthropic_api_key {
                    if api_key.starts_with("sk-ant-") {
                        super::chat::call_anthropic(api_key, &messages).await
                    } else if let Some(ref or_key) = openrouter_key {
                        // OpenRouter key available (either from OPENROUTER_API_KEY or
                        // detected sk-or-v1- prefix in ANTHROPIC_API_KEY).
                        let model = normalize_for_openrouter(&resolve_free_model(body.model.as_deref()));
                        tracing::info!(model = %model, "retrying via OpenRouter key fallback");
                        let synth = crate::routes::secrets::InferenceEndpointEntry {
                            id: "openrouter-key-fallback".into(),
                            url: "https://openrouter.ai/api/v1".into(),
                            provider: "openrouter".into(),
                            model,
                            status: "unknown".into(),
                            requires_auth: true,
                            secret_key: or_key.clone(),
                            health_checked_at: String::new(),
                        };
                        super::chat::call_inference_endpoint(&synth, &messages).await
                    } else {
                        Err(format!(
                            "{} failed: {}; no valid fallback key (ANTHROPIC_API_KEY contains a non-Anthropic key and OPENROUTER_API_KEY is not set)",
                            ep.provider, e
                        ))
                    }
                } else if let Some(ref or_key) = openrouter_key {
                    let model = normalize_for_openrouter(&resolve_free_model(body.model.as_deref()));
                    tracing::info!(model = %model, "retrying via OpenRouter key (no Anthropic key)");
                    let synth = crate::routes::secrets::InferenceEndpointEntry {
                        id: "openrouter-key-fallback".into(),
                        url: "https://openrouter.ai/api/v1".into(),
                        provider: "openrouter".into(),
                        model,
                        status: "unknown".into(),
                        requires_auth: true,
                        secret_key: or_key.clone(),
                        health_checked_at: String::new(),
                    };
                    super::chat::call_inference_endpoint(&synth, &messages).await
                } else {
                    Err(format!(
                        "{} failed: {}; no Anthropic fallback configured",
                        ep.provider, e
                    ))
                }
            }
        }
    } else if let Some(ref api_key) = state.anthropic_api_key {
        if api_key.starts_with("sk-ant-") {
            super::chat::call_anthropic(api_key, &messages).await
        } else if let Some(ref or_key) = openrouter_key {
            let model = normalize_for_openrouter(&resolve_free_model(body.model.as_deref()));
            tracing::info!(model = %model, "no registered providers — using OpenRouter key fallback");
            let synth = crate::routes::secrets::InferenceEndpointEntry {
                id: "openrouter-key-fallback".into(),
                url: "https://openrouter.ai/api/v1".into(),
                provider: "openrouter".into(),
                model,
                status: "unknown".into(),
                requires_auth: true,
                secret_key: or_key.clone(),
                health_checked_at: String::new(),
            };
            super::chat::call_inference_endpoint(&synth, &messages).await
        } else {
            Err("No inference endpoints registered and ANTHROPIC_API_KEY contains a non-Anthropic key (set OPENROUTER_API_KEY for OpenRouter)".into())
        }
    } else if let Some(ref or_key) = openrouter_key {
        let model = normalize_for_openrouter(&resolve_free_model(body.model.as_deref()));
        tracing::info!(model = %model, "no registered providers and no ANTHROPIC_API_KEY — using OPENROUTER_API_KEY");
        let synth = crate::routes::secrets::InferenceEndpointEntry {
            id: "openrouter-key-fallback".into(),
            url: "https://openrouter.ai/api/v1".into(),
            provider: "openrouter".into(),
            model,
            status: "unknown".into(),
            requires_auth: true,
            secret_key: or_key.clone(),
            health_checked_at: String::new(),
        };
        super::chat::call_inference_endpoint(&synth, &messages).await
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
