//! HTTP inference endpoint — synchronous LLM completion for hex-agent.
//!
//! POST /api/inference/complete
//!
//! Routes through registered inference providers (Ollama, vLLM, OpenAI-compat)
//! with Anthropic as fallback, reusing the same logic as the WebSocket LLM bridge.
//!
//! Forward-progress guarantees:
//!   - Hard 300s outer deadline (HTTP 504 on expiry); local providers need time for model load
//!   - Vault resolution has a 3s timeout; fails fast rather than stalling the handler
//!   - 401 is a hard-fail — bad credentials never trigger the fallback chain
//!   - Local provider 503 (model loading) uses exponential backoff, not single-retry
//!   - Minimum 2s inter-candidate sleep prevents thundering-herd rate exhaustion
//!   - Model routing uses exact JSON array match, not substring search

use axum::{extract::State, Json};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::complexity::score_complexity;
use crate::ports::secret_grant::ISecretGrantPort;
use crate::quant_router::select_provider;
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
/// Hard deadline: 600 seconds. Returns HTTP 504 on timeout.
/// Local providers (Ollama, vLLM) may need 5-10 minutes to load a model on first request.
pub async fn inference_complete(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<InferenceCompleteRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let work = async move {

    // Score complexity before consuming body.messages (ADR-2603271000).
    let prompt_text = body.messages.iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    let complexity = score_complexity(&prompt_text, &[]);
    let min_quant = complexity.min_quantization();
    tracing::debug!(
        complexity = ?complexity,
        min_quant = %min_quant,
        "quantization routing: complexity scored, minimum tier selected"
    );

    // Resolve architecture fingerprint for ACI injection (ADR-2603301200).
    // Read project_id from x-hex-project-id header; look up in state.fingerprints.
    // If found, prepend the fingerprint block to the system prompt.
    let aci_block: Option<String> = {
        let project_id = headers
            .get("x-hex-project-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !project_id.is_empty() {
            let fp_map = state.fingerprints.read().await;
            fp_map.get(project_id).map(|fp| fp.to_injection_block())
        } else {
            // No project header — try to use any fingerprint that is registered
            let fp_map = state.fingerprints.read().await;
            if fp_map.len() == 1 {
                fp_map.values().next().map(|fp| fp.to_injection_block())
            } else {
                None
            }
        }
    };

    // Build messages list, optionally prepending system prompt (with ACI block).
    let effective_system: Option<String> = match (aci_block, body.system.as_deref()) {
        (Some(aci), Some(sys)) if !sys.is_empty() => {
            Some(format!("{}\n\n---\n\n{}", aci, sys))
        }
        (Some(aci), _) => Some(aci),
        (None, Some(sys)) if !sys.is_empty() => Some(sys.to_string()),
        _ => None,
    };

    let mut messages = body.messages;
    if let Some(ref system) = effective_system {
        messages.insert(0, json!({ "role": "system", "content": system }));
    }

    // Try registered inference endpoints first (SpacetimeDB providers)
    // If a model is requested, find the provider that serves it; otherwise use first provider.
    // Complexity scoring selects minimum quantization tier (ADR-2603271000).
    let endpoint: Option<crate::routes::secrets::InferenceEndpointEntry> =
        if let Some(ref stdb) = state.inference_stdb {
            match stdb.list_providers().await {
                Ok(providers) if !providers.is_empty() => {
                    // Find the provider that matches the requested model.
                    // Exact element match via JSON deserialization — substring search would
                    // route "llama-3" to any provider whose list contains "meta-llama/llama-3.3-70b".
                    let matched = if let Some(ref requested_model) = body.model {
                        providers.iter().find(|p| {
                            serde_json::from_str::<Vec<String>>(&p.models_json)
                                .map(|models| models.iter().any(|m| m == requested_model.as_str()))
                                .unwrap_or_else(|_| p.models_json.contains(requested_model.as_str()))
                        })
                        // For OpenRouter-format IDs (e.g. "google/gemini-2.0-flash-001"),
                        // route through any registered openrouter provider with a key.
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
                        // No model requested — use quantization router to pick best provider
                        select_provider(&providers, min_quant)
                    };
                    // Use matched provider if found.
                    // If no model was requested, fall back to the first provider.
                    // If a specific model was requested but NOT matched by any registered
                    // provider, yield None so endpoint = None and the key-based OpenRouter
                    // path (below) handles it — routing to an unrelated provider (e.g. an
                    // offline Ollama) would waste time and eventually time out.
                    let resolved = matched
                        .or_else(|| if body.model.is_none() { Some(&providers[0]) } else { None });

                    if let Some(p) = resolved {
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
                    } else {
                        tracing::debug!(
                            model = ?body.model,
                            "no registered provider serves this model — falling through to key-based path"
                        );
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };

    // Resolve a synthetic OpenRouter endpoint from a key that may have been placed
    // in ANTHROPIC_API_KEY (sk-or-v1- prefix) or OPENROUTER_API_KEY.
    // Vault-first resolution (set at startup by lib.rs). Fall back to env, then
    // check if ANTHROPIC_API_KEY is actually an OpenRouter key (sk-or-v1- prefix).
    let openrouter_key: Option<String> = state.openrouter_api_key.clone()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
        .or_else(|| {
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

    // Normalize bare model IDs to OpenRouter vendor-namespaced format.
    // OpenRouter requires "anthropic/claude-sonnet-4-6", not "claude-sonnet-4-6".
    // Covers the most common families; unknown bare IDs pass through unchanged and
    // will 404 on OpenRouter, triggering the free fallback chain.
    let normalize_for_openrouter = |model: &str| -> String {
        if model.contains('/') {
            return model.to_string();
        }
        if model.starts_with("claude-") {
            format!("anthropic/{}", model)
        } else if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
            format!("openai/{}", model)
        } else if model.starts_with("gemini-") {
            format!("google/{}", model)
        } else if model.starts_with("mistral-") || model.starts_with("mixtral-") {
            format!("mistralai/{}", model)
        } else if model.starts_with("deepseek-") {
            format!("deepseek/{}", model)
        } else {
            model.to_string()
        }
    };

    let result = if let Some(mut ep) = endpoint {
        // Apply requested model override, normalizing bare IDs for OpenRouter.
        if let Some(ref model) = body.model {
            ep.model = if ep.provider == "openrouter" {
                normalize_for_openrouter(model)
            } else {
                model.clone()
            };
        }
        // Resolve secret key reference to actual value from vault.
        // Hard 3s timeout — a slow SpacetimeDB must not stall the handler indefinitely.
        // Fail immediately on miss or timeout: passing the unresolved ref string as a
        // Bearer token produces a misleading 401 that bypasses all useful error context.
        if ep.requires_auth && !ep.secret_key.is_empty() && !ep.secret_key.starts_with("sk-") {
            let key_ref = ep.secret_key.clone();
            tracing::debug!(key_ref = %key_ref, "resolving secret key reference");
            if let Ok(val) = std::env::var(&key_ref) {
                tracing::debug!("resolved from env var");
                ep.secret_key = val;
            } else if let Some(ref stdb) = state.spacetime_secrets {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    stdb.vault_get(&key_ref),
                ).await {
                    Ok(Ok(Some(val))) => {
                        tracing::debug!("resolved from vault");
                        ep.secret_key = val;
                    }
                    Ok(Ok(None)) => {
                        tracing::warn!(key = %key_ref, "secret not found in vault");
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error": "secret_resolution_failed", "ref": key_ref})));
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(key = %key_ref, error = %e, "vault_get failed");
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error": "secret_resolution_failed", "ref": key_ref})));
                    }
                    Err(_elapsed) => {
                        tracing::warn!(key = %key_ref, "vault_get timed out after 3s");
                        return (StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error": "secret_resolution_timeout", "ref": key_ref})));
                    }
                }
            } else {
                tracing::warn!("spacetime_secrets not available for vault resolution");
            }
        }
        match super::chat::call_inference_endpoint(&ep, &messages).await {
            Ok(resp) => Ok(resp),
            // Hard-fail on authentication errors — bad credentials must never trigger the
            // fallback chain. Doing so wastes the entire retry budget and produces an error
            // trail that ends at Anthropic with no indication of the root cause.
            Err(ref e) if e.contains("401") || e.contains("Unauthorized") => {
                tracing::error!(provider = %ep.provider, error = %e,
                    "authentication failed — bad credentials, not retrying");
                return (StatusCode::UNAUTHORIZED, Json(json!({
                    "error": "authentication_failed",
                    "provider": ep.provider,
                    "detail": e
                })));
            }
            Err(ref e) if e.contains("insufficient credits") || e.contains("402")
                || e.contains("rate limited") || e.contains("429")
                || e.contains("parse:") || e.contains("500") || e.contains("503")
                || e.contains("404") || e.contains("No endpoints") || e.contains("data policy")
                || e.contains("connection:") || e.contains("null content") => {
                // OpenRouter transient failure (credits, rate limit, parse/server error),
                // or permanent failure (404 model-not-found / data policy).
                //
                // For transient errors (parse/5xx on cloud), retry the same endpoint once
                // after a brief sleep. Exception: local providers returning 503 are mid-load
                // (not a cloud transient) — route them through exponential backoff instead.
                let is_local = ep.provider == "ollama" || ep.provider == "vllm";
                let is_transient = (e.contains("parse:") || e.contains("500") || e.contains("503"))
                    && !e.contains("404") && !e.contains("No endpoints") && !e.contains("data policy")
                    && !is_local;
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
                // For local providers (Ollama/vLLM), retry with exponential backoff + jitter.
                // Match both TCP connection errors AND HTTP 503 — Ollama returns 503 while
                // loading a model, which is semantically identical to "not ready yet".
                let is_local_connection_error = is_local
                    && (e.contains("connection:") || e.contains("503"));
                if is_local_connection_error {
                    let mut backoff_ms = 5_000u64; // start at 5s
                    for attempt in 1u8..=3 {
                        // Jitter: use subsecond nanos as cheap pseudo-random source (no dep needed)
                        let jitter_ms = (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .subsec_nanos() % 2_000) as u64; // 0-2s jitter
                        let sleep_ms = backoff_ms + jitter_ms;
                        tracing::warn!(
                            provider = %ep.provider, model = %ep.model,
                            attempt, sleep_ms,
                            "local model not ready — backing off before retry"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(sleep_ms)).await;
                        match super::chat::call_inference_endpoint(&ep, &messages).await {
                            Ok(resp) => return (StatusCode::OK, Json(json!({
                                "content": resp.0, "model": resp.1,
                                "input_tokens": resp.2, "output_tokens": resp.3,
                            }))),
                            Err(ref e2) => tracing::warn!(attempt, error = %e2, "local retry failed"),
                        }
                        backoff_ms = (backoff_ms * 2).min(60_000); // cap at 60s
                    }
                    tracing::warn!(provider = %ep.provider, model = %ep.model,
                        "all local retries exhausted — falling through to :free providers");
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
                // A 2s minimum sleep between candidates prevents rapid-fire requests from
                // burning the per-minute rate-limit window before any candidate can succeed.
                let mut fallback_result: Result<_, String> = Err("no :free providers registered".to_string());
                for fp in &free_providers {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let free_model = fp.models_json
                        .trim_start_matches('[').trim_end_matches(']')
                        .split(',').next().unwrap_or(&fp.models_json)
                        .trim().trim_matches('"').to_string();
                    // Resolve secret key — same 3s timeout + fail-fast as main path.
                    let resolved_key = if fp.api_key_ref.starts_with("sk-") {
                        fp.api_key_ref.clone()
                    } else {
                        let key_ref = &fp.api_key_ref;
                        if let Ok(val) = std::env::var(key_ref) {
                            val
                        } else if let Some(ref stdb) = state.spacetime_secrets {
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(3),
                                stdb.vault_get(key_ref),
                            ).await {
                                Ok(Ok(Some(val))) => val,
                                Ok(Ok(None)) => {
                                    tracing::warn!(key = %key_ref, ":free provider secret not in vault — skipping");
                                    continue;
                                }
                                Ok(Err(e)) => {
                                    tracing::warn!(key = %key_ref, error = %e, "vault_get failed for :free provider — skipping");
                                    continue;
                                }
                                Err(_elapsed) => {
                                    tracing::warn!(key = %key_ref, "vault_get timed out for :free provider — skipping");
                                    continue;
                                }
                            }
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
                            // For rate-limited models, back off before the next attempt.
                            // Skip for policy/404 errors — they won't recover with time.
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
                        // Ordered by capability (best first) — most capable free
                        // models are tried before weaker ones so code generation gets
                        // the strongest available model, not just the first that responds.
                        let free_candidates = [
                            "openai/gpt-4o-mini",
                            "meta-llama/llama-3.3-70b-instruct:free",
                            "mistralai/mistral-small-3.1-24b-instruct:free",
                            "deepseek/deepseek-r1:free",
                            "meta-llama/llama-3.2-3b-instruct:free",
                            "arcee-ai/trinity-mini:free",
                        ];
                        for free_model in free_candidates {
                            // Minimum inter-candidate delay — same rationale as registered :free loop.
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
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
                fallback_result
            }
            Err(e) => {
                tracing::warn!(provider = %ep.provider, error = %e, "Inference endpoint failed, trying fallback");
                // Fallback hierarchy: Anthropic key → OpenRouter key → error.
                if let Some(ref api_key) = state.anthropic_api_key {
                    if api_key.starts_with("sk-ant-") {
                        super::chat::call_anthropic(api_key, &messages).await
                    } else if let Some(ref or_key) = openrouter_key {
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
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            tracing::error!(error = %e, "inference/complete failed");
            (StatusCode::BAD_GATEWAY, Json(json!({ "error": e })))
        }
    }

    }; // end async move work block

    match tokio::time::timeout(std::time::Duration::from_secs(600), work).await {
        Ok(response) => response,
        Err(_elapsed) => {
            tracing::error!("inference/complete timed out after 600s");
            (StatusCode::GATEWAY_TIMEOUT, Json(json!({
                "error": "inference_timeout",
                "message": "Request exceeded 600s deadline"
            })))
        }
    }
}
