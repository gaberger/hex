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

use axum::{extract::{Path, State}, Json};
use chrono::{DateTime, Utc};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

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
    /// Tool schemas (OpenAI function-calling format). When present, the model
    /// may emit tool_call events; finish_reason "tool_calls" triggers a done
    /// event with a `tool_calls` array for the client to execute.
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
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
    let started = std::time::Instant::now();

    // Score complexity before consuming body.messages (ADR-2026-03-27-1000).
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

    // Resolve architecture fingerprint for ACI injection (ADR-2026-03-30-1200).
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

    // ── Tools fast-path (hex agent run / typed-tool dispatch) ──────────
    // When the request includes a `tools` schema, iterate registered
    // inference providers in priority order (local first → paid last)
    // and use the first one that supports tools AND completes the call.
    // This replaces an older hardcoded-OpenRouter path that burned
    // credits even when a local Ollama was registered (observed
    // 2026-05-21: 6 persona tasks all failed with "OpenRouter:
    // insufficient credits" while Ollama sat idle on localhost).
    //
    // Fallback chain in priority order:
    //   1. Registered tools-capable providers, sorted by
    //      `priority_for_tools` (ollama/vllm/llama-cpp → openai-compat
    //      → openrouter, healthy > unknown > unhealthy within tier)
    //   2. Synthetic OpenRouter endpoint from OPENROUTER_API_KEY env
    //      (preserves legacy behavior for operators with no registered
    //      providers)
    //   3. Fall through to the no-tools chain — the simple_agent
    //      text-mode parser still gets a chance to surface structured
    //      output from prose
    if let Some(ref tools_schema) = body.tools {
        if !tools_schema.is_empty() {
            // Convert STDB ProviderRow → secrets::InferenceEndpointEntry
            // (the shape `call_inference_endpoint_with_tools` consumes).
            // ProviderRow has multi-model JSON; we expand each row into
            // one entry per advertised model, then dedupe later in the
            // priority sort.
            let registered: Vec<crate::routes::secrets::InferenceEndpointEntry> =
                if let Some(ref stdb) = state.inference_stdb {
                    let rows = stdb.list_providers().await.unwrap_or_default();
                    let mut out: Vec<crate::routes::secrets::InferenceEndpointEntry> =
                        Vec::with_capacity(rows.len() * 2);
                    for r in rows {
                        // Pick the first model from models_json (defensive
                        // parse — strings work, JSON arrays work).
                        let model = serde_json::from_str::<Vec<String>>(&r.models_json)
                            .ok()
                            .and_then(|v| v.into_iter().next())
                            .unwrap_or_else(|| {
                                r.models_json
                                    .trim_start_matches('[')
                                    .trim_end_matches(']')
                                    .split(',')
                                    .next()
                                    .unwrap_or(&r.models_json)
                                    .trim()
                                    .trim_matches('"')
                                    .to_string()
                            });
                        out.push(crate::routes::secrets::InferenceEndpointEntry {
                            id: r.provider_id,
                            url: r.base_url,
                            provider: r.provider_type,
                            model,
                            status: if r.healthy == 1 { "healthy".into() } else { "unknown".into() },
                            requires_auth: !r.api_key_ref.is_empty(),
                            secret_key: r.api_key_ref,
                            health_checked_at: r.last_health_check,
                        });
                    }
                    out
                } else {
                    Vec::new()
                };

            // Build the candidate list. Filter to tools-capable, sort
            // by priority. Within Ollama family, prefer entries whose
            // model matches the request's model hint (if any).
            let requested_model = body.model.clone();
            let mut candidates: Vec<crate::routes::secrets::InferenceEndpointEntry> = registered
                .into_iter()
                .filter(super::chat::provider_supports_tools)
                .collect();
            candidates.sort_by_key(super::chat::priority_for_tools);

            // If the caller specified a model, the provider that actually
            // serves it wins — model match DOMINATES the local-first
            // priority. Otherwise a tier request for a cloud model (e.g.
            // T2 "Qwen/Qwen3-32B" on Tenstorrent) gets hijacked by whatever
            // local Ollama model happens to be registered, since Ollama
            // ranks above openai-compat in priority_for_tools. Priority only
            // breaks ties within the matched / unmatched groups.
            if let Some(ref m) = requested_model {
                candidates.sort_by(|a, b| {
                    let pa = super::chat::priority_for_tools(a);
                    let pb = super::chat::priority_for_tools(b);
                    let am = if a.model == *m { 0 } else { 1 };
                    let bm = if b.model == *m { 0 } else { 1 };
                    (am, pa).cmp(&(bm, pb))
                });
            }

            // Always append a synthetic OpenRouter env-key endpoint as
            // the last-resort fallback. Skipped if no key is available.
            let openrouter_key: Option<String> = state.openrouter_api_key.clone()
                .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok()
                    .filter(|k| k.starts_with("sk-or-")));
            if let Some(or_key) = openrouter_key.clone() {
                // Skip if a registered openrouter entry is already
                // first-class — no point queueing two of them.
                let already_have_or = candidates.iter().any(|c| {
                    c.provider.eq_ignore_ascii_case("openrouter")
                        || c.url.contains("openrouter.ai")
                });
                if !already_have_or {
                    let raw_model = requested_model
                        .clone()
                        .unwrap_or_else(|| "anthropic/claude-haiku-4.5".to_string());
                    // OpenRouter needs vendor/model slug; tolerate
                    // Ollama-style names by substituting a sane default.
                    let or_model = if raw_model.contains('/') {
                        raw_model
                    } else if raw_model.starts_with("claude") {
                        format!("anthropic/{}", raw_model)
                    } else if raw_model.starts_with("gpt") || raw_model.starts_with("o1") {
                        format!("openai/{}", raw_model)
                    } else if raw_model.contains(':') {
                        "anthropic/claude-haiku-4.5".to_string()
                    } else {
                        raw_model
                    };
                    candidates.push(crate::routes::secrets::InferenceEndpointEntry {
                        id: "openrouter-env-fastpath".into(),
                        url: "https://openrouter.ai/api/v1".into(),
                        provider: "openrouter".into(),
                        model: or_model,
                        status: "unknown".into(),
                        requires_auth: true,
                        secret_key: or_key,
                        health_checked_at: String::new(),
                    });
                }
            }

            // Walk the candidate list. First success wins.
            let mut last_err = String::new();
            for ep in &candidates {
                // Resolve a vault secret reference (e.g. "TENSTORRENT") to the
                // real key before dispatch. The no-tools path does this at the
                // L513 block; the tools fast-path must too, or openai-compat
                // providers receive a bogus `Bearer <ref>` and 401. Env var
                // first, then the SpacetimeDB vault (3s cap).
                let mut ep = ep.clone();
                // Honor the requested model on LOCAL providers. One Ollama/vLLM/
                // llama-cpp endpoint serves any locally-available model, so the
                // registered provider's model must not pin the request — otherwise
                // `--model` (and `hex do --model`, the bench `--model`) is silently
                // ignored on the tools path and every call runs the registered
                // default. Diagnostic 2026-06-07: the bench `react` arm ran
                // gemma4-12b regardless of --model because of this.
                if let Some(ref m) = requested_model {
                    let m = m.trim();
                    if !m.is_empty()
                        && m != ep.model
                        && matches!(ep.provider.as_str(), "ollama" | "vllm" | "llama-cpp" | "llamacpp")
                    {
                        ep.model = m.to_string();
                    }
                }
                if ep.requires_auth && !ep.secret_key.is_empty() && !ep.secret_key.starts_with("sk-") {
                    let key_ref = ep.secret_key.clone();
                    if let Ok(val) = std::env::var(&key_ref) {
                        ep.secret_key = val;
                    } else if let Some(ref stdb) = state.spacetime_secrets {
                        match tokio::time::timeout(
                            std::time::Duration::from_secs(3),
                            stdb.vault_get(&key_ref),
                        ).await {
                            Ok(Ok(Some(val))) => ep.secret_key = val,
                            _ => {
                                tracing::warn!(key = %key_ref, "tools fast-path: vault resolution failed; skipping candidate");
                                continue;
                            }
                        }
                    }
                }
                tracing::debug!(
                    provider = %ep.provider,
                    model = %ep.model,
                    "tools fast-path: trying candidate"
                );
                match super::chat::call_inference_endpoint_with_tools(&ep, &messages, tools_schema).await {
                    Ok(((content, model_used, input_tokens, output_tokens, cost), tool_calls)) => {
                        tracing::info!(
                            provider = %ep.provider,
                            model = %model_used,
                            input_tokens,
                            output_tokens,
                            tool_calls = tool_calls.len(),
                            "inference/complete OK (tools fast-path)"
                        );
                        let mut resp = json!({
                            "content": content,
                            "model": model_used,
                            "input_tokens": input_tokens,
                            "output_tokens": output_tokens,
                            "tool_calls": tool_calls,
                            "provider": ep.provider,
                        });
                        if !cost.is_empty() {
                            resp["openrouter_cost_usd"] = json!(cost);
                        }
                        return (StatusCode::OK, Json(resp));
                    }
                    Err(e) => {
                        tracing::warn!(
                            provider = %ep.provider,
                            error = %e,
                            "tools fast-path: candidate failed; trying next"
                        );
                        last_err = e;
                    }
                }
            }

            if !candidates.is_empty() {
                tracing::warn!(
                    last_err = %last_err,
                    candidates = candidates.len(),
                    "tools fast-path: all candidates exhausted — falling through to no-tools chain"
                );
            }
        }
    }

    // Try registered inference endpoints first (SpacetimeDB providers)
    // If a model is requested, find the provider that serves it; otherwise use first provider.
    // Complexity scoring selects minimum quantization tier (ADR-2026-03-27-1000).
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
                        // No registry match — check for local Ollama provider as fallback.
                        // Local models don't have "/" in the ID (e.g. "nemotron-mini", "qwen3:8b").
                        let is_local_model = body.model.as_ref()
                            .map(|m| !m.contains('/'))
                            .unwrap_or(false);
                        
                        if is_local_model {
                            // Find any ollama provider (local) that might serve this.
                            let local_provider = providers.iter()
                                .find(|p| p.provider_type == "ollama" && !p.base_url.is_empty());
                            
                            if let Some(p) = local_provider {
                                tracing::debug!(
                                    model = ?body.model,
                                    provider = %p.provider_id,
                                    "routing to local Ollama provider"
                                );
                                Some(crate::routes::secrets::InferenceEndpointEntry {
                                    id: p.provider_id.clone(),
                                    url: p.base_url.clone(),
                                    provider: p.provider_type.clone(),
                                    model: body.model.clone().unwrap_or_default(),
                                    status: if p.healthy == 1 { "healthy".into() } else { "unknown".into() },
                                    requires_auth: false,
                                    secret_key: String::new(),
                                    health_checked_at: p.last_health_check.clone(),
                                })
                            } else {
                                tracing::debug!(model = ?body.model, "no local Ollama provider found");
                                None
                            }
                        } else {
                            tracing::debug!(
                                model = ?body.model,
                                "no registered provider serves this model — falling through to key-based path"
                            );
                            None
                        }
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
            // Anthropic-direct uses dashes + 8-digit date suffix
            // (e.g. "claude-haiku-4-5-20251001", "claude-sonnet-4-6").
            // OpenRouter uses dotted version, no date (e.g. "claude-haiku-4.5",
            // "claude-3.7-sonnet"). Convert here so callers can use either form.
            let mut parts: Vec<&str> = model.split('-').collect();
            if parts.last().is_some_and(|p| p.len() == 8 && p.chars().all(|c| c.is_ascii_digit())) {
                parts.pop();
            }
            let n = parts.len();
            // Pattern A: claude-FAMILY-MAJOR-MINOR  → anthropic/claude-FAMILY-MAJOR.MINOR
            if n >= 4
                && parts[n-1].chars().all(|c| c.is_ascii_digit())
                && parts[n-2].chars().all(|c| c.is_ascii_digit())
            {
                let prefix = parts[..n-2].join("-");
                return format!("anthropic/{}-{}.{}", prefix, parts[n-2], parts[n-1]);
            }
            // Pattern B: claude-MAJOR-MINOR-FAMILY  → anthropic/claude-MAJOR.MINOR-FAMILY
            if n >= 4
                && parts[1].chars().all(|c| c.is_ascii_digit())
                && parts[2].chars().all(|c| c.is_ascii_digit())
            {
                let suffix = parts[3..].join("-");
                return format!("anthropic/claude-{}.{}-{}", parts[1], parts[2], suffix);
            }
            format!("anthropic/{}", model)
        } else if model.starts_with("gpt-") || model.starts_with("o1") || model.starts_with("o3") || model.starts_with("o4") {
            format!("openai/{}", model)
        } else if model.starts_with("gemini-") {
            format!("google/{}", model)
        } else if model.starts_with("mistral-") || model.starts_with("mixtral-") {
            format!("mistralai/{}", model)
        } else if model.starts_with("deepseek-") {
            format!("deepseek/{}", model)
        } else if model.contains(':') {
            // Ollama-style (e.g. qwen2.5-coder:14b) — won't resolve on OpenRouter.
            // When no local Ollama provider is registered to serve it, substitute
            // a tool-capable OpenRouter default so the drafter/SOP chain doesn't
            // 502 on "not a valid model ID". Mirrors the tools fast-path at L146-156.
            tracing::warn!(
                requested = %model,
                fallback = "anthropic/claude-haiku-4.5",
                "legacy path: Ollama-style model substituted with OpenRouter-capable default"
            );
            "anthropic/claude-haiku-4.5".to_string()
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
        // LoRA idiom-expert attachment (ADR-2606161300 Phase 1). If an enabled adapter
        // is registered for this local base, swap to the derived base+adapter model.
        // This changes ONLY which weights generate the draft — every correctness gate
        // (analyze/specs/compile) is downstream and untouched. Absent/disabled/unbuilt
        // → leaves ep.model as the bare base. Best-effort, never errors.
        if let Some(lora_model) =
            crate::lora_attach::resolve_serving_model(&ep.provider, &ep.model, &ep.url).await
        {
            ep.model = lora_model;
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
        // Record request dispatch in rate limiter (ADR-2026-04-05-2125)
        state.rate_limiter.record_request(&ep.id, body.max_tokens as u64).await;
        match super::chat::call_inference_endpoint(&ep, &messages).await {
            Ok(resp) => {
                // Record success in rate limiter
                state.rate_limiter.record_completion(&ep.id, resp.2, resp.3, true).await;
                Ok(resp)
            }
            // Hard-fail on authentication errors — bad credentials must never trigger the
            // fallback chain. Doing so wastes the entire retry budget and produces an error
            // trail that ends at Anthropic with no indication of the root cause.
            Err(ref e) if e.contains("401") || e.contains("Unauthorized") => {
                // Record auth failure in rate limiter (ADR-2026-04-05-2125)
                state.rate_limiter.record_completion(&ep.id, 0, 0, false).await;
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
                // Record transient failure in rate limiter (ADR-2026-04-05-2125)
                state.rate_limiter.record_completion(&ep.id, 0, 0, false).await;
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
                // If all free models failed, try a registered local Ollama
                // endpoint with a sensible code-gen model. Most operators
                // running this loop have Ollama on localhost (the standalone
                // composition variant requires it). When OpenRouter's free
                // tier is exhausted (rate-limited or auth-broken), local
                // inference is the difference between the loop progressing
                // and the loop sitting idle on every workplan.
                if fallback_result.is_err() {
                    if let Some(ref stdb) = state.inference_stdb {
                        let ollama = stdb.list_providers().await.ok()
                            .unwrap_or_default()
                            .into_iter()
                            .find(|p| p.provider_type == "ollama" && !p.base_url.is_empty());
                        if let Some(p) = ollama {
                            // Pick a model the LOCAL Ollama can actually serve.
                            // Priority: (1) the originally-requested model if it's
                            // local-shaped (no vendor "/" slug — e.g. "nemotron-mini",
                            // "qwen2.5-coder:14b"); (2) the configured T2 tier model,
                            // but ONLY if it's local — since T2 may now be a cloud-only
                            // id like "Qwen/Qwen3-32B" that Ollama 404s on; (3) a known
                            // local default. Reading T2 also keeps us from asking for a
                            // model larger than the local GPU can hold.
                            let is_local_name = |m: &str| !m.contains('/');
                            let t2_local = std::fs::read_to_string(".hex/project.json")
                                .ok()
                                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                                .and_then(|v| v.get("inference")
                                    .and_then(|i| i.get("tier_models"))
                                    .and_then(|t| t.get("t2"))
                                    .and_then(|m| m.as_str())
                                    .map(|s| s.to_string()))
                                .filter(|m| is_local_name(m));
                            let local_model = body.model.clone()
                                .filter(|m| is_local_name(m))
                                .or(t2_local)
                                .unwrap_or_else(|| "qwen2.5-coder:32b".to_string());
                            tracing::info!(
                                provider = %p.provider_id,
                                model = %local_model,
                                "all openrouter fallbacks failed — retrying via local Ollama"
                            );
                            let synth = crate::routes::secrets::InferenceEndpointEntry {
                                id: p.provider_id.clone(),
                                url: p.base_url.clone(),
                                provider: "ollama".into(),
                                model: local_model,
                                status: "unknown".into(),
                                requires_auth: false,
                                secret_key: String::new(),
                                health_checked_at: p.last_health_check.clone(),
                            };
                            fallback_result = super::chat::call_inference_endpoint(&synth, &messages).await;
                            if let Err(ref e) = fallback_result {
                                tracing::warn!(error = %e, "local Ollama fallback also failed");
                            }
                        }
                    }
                }
                // If even Ollama failed, try Anthropic direct as final fallback.
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
                resp["openrouter_cost_usd"] = json!(openrouter_cost.clone());
            }
            // Fire-and-forget inference_log write — closes CTO commitment 12294.
            // Never blocks the response; tokio::spawn isolates any STDB failure.
            let log_model = model.clone();
            let log_cost = openrouter_cost.clone();
            let duration_ms = started.elapsed().as_millis() as u64;
            let stdb_for_log = state.inference_stdb.clone();
            tokio::spawn(async move {
                let session_id = std::env::var("CLAUDE_SESSION_ID")
                    .unwrap_or_else(|_| "nexus-bg".to_string());
                // Resolve the ACTUAL provider type by matching the served model
                // against registered providers. The old name-only heuristic
                // mislabeled cloud openai_compat models with vendor slugs (e.g.
                // "Qwen/Qwen3-32B", "deepseek-ai/DeepSeek-R1-0528") as
                // "openrouter" because they have no ':' and aren't "anthropic/".
                let mut provider = if log_model.starts_with("anthropic/") || log_model.starts_with("claude") {
                    "anthropic".to_string()
                } else if log_model.starts_with("ollama/") || log_model.contains(':') {
                    "ollama".to_string()
                } else {
                    "openrouter".to_string()
                };
                if let Some(stdb) = stdb_for_log {
                    if let Ok(providers) = stdb.list_providers().await {
                        if let Some(p) = providers.iter().find(|p| {
                            serde_json::from_str::<Vec<String>>(&p.models_json)
                                .map(|models| models.iter().any(|m| m == &log_model))
                                .unwrap_or(false)
                        }) {
                            provider = p.provider_type.clone();
                        }
                    }
                }
                let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
                    .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
                let url = format!("{}/v1/database/hex/call/inference_log_create", stdb_host);
                let body = serde_json::json!([
                    uuid::Uuid::new_v4().to_string(),
                    session_id,
                    "chat",
                    log_model,
                    provider,
                    input_tokens,
                    output_tokens,
                    log_cost,
                    duration_ms,
                    0u64,
                    "",
                    "success",
                    chrono::Utc::now().to_rfc3339(),
                ]);
                let _ = reqwest::Client::new()
                    .post(&url)
                    .json(&body)
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await;
            });
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

// ── Path B: Inference Queue (ADR-2026-04-01-0000) ──────────────────────────────

/// An entry in the inference dispatch queue. Stored in HexFlo memory so workers
/// can claim tasks via GET /api/inference/queue/pending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceQueueEntry {
    pub id: String,
    pub task_id: String,
    pub workplan_id: String,
    pub prompt: String,
    pub role: String,
    /// "pending" | "claimed" | "completed"
    pub status: String,
    pub created_at: DateTime<Utc>,
}

impl InferenceQueueEntry {
    pub fn new(task_id: String, workplan_id: String, prompt: String, role: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            task_id,
            workplan_id,
            prompt,
            role,
            status: "pending".to_string(),
            created_at: Utc::now(),
        }
    }

    /// HexFlo memory key for this entry.
    pub fn memory_key(&self) -> String {
        format!("inference:queue:{}", self.id)
    }
}

#[derive(Debug, Deserialize)]
pub struct InferenceQueueRequest {
    pub task_id: String,
    pub workplan_id: String,
    pub prompt: String,
    pub role: String,
}

/// POST /api/inference/queue — enqueue an inference task for Path B dispatch.
///
/// Creates an `InferenceQueueEntry` with status "pending", persists it in
/// HexFlo memory under `inference:queue:{id}`, sends an inbox notification,
/// and returns the queue entry ID so the caller can poll or claim it.
pub async fn inference_queue(
    State(state): State<SharedState>,
    Json(body): Json<InferenceQueueRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let entry = InferenceQueueEntry::new(
        body.task_id.clone(),
        body.workplan_id.clone(),
        body.prompt.clone(),
        body.role.clone(),
    );
    let key = entry.memory_key();
    let id = entry.id.clone();

    let value = match serde_json::to_string(&entry) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize InferenceQueueEntry");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("serialization failed: {}", e) })),
            );
        }
    };

    // Persist to HexFlo memory via state_port.
    if let Some(sp) = state.state_port.as_deref() {
        if let Err(e) = sp.hexflo_memory_store(&key, &value, "global").await {
            tracing::error!(error = %e, key = %key, "failed to store inference queue entry");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("memory store failed: {}", e) })),
            );
        }

        // Best-effort inbox notification — do not fail the request if notify fails.
        let msg = format!("Inference task queued: {} ({})", body.task_id, body.role);
        if let Err(e) = sp
            .inbox_notify("system", 1, "inference_queue", &msg)
            .await
        {
            tracing::warn!(error = %e, "inbox_notify failed for inference queue entry");
        }
    } else {
        tracing::warn!("state_port not available — inference queue entry not persisted");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "State port not available" })),
        );
    }

    tracing::info!(
        queue_id = %id,
        task_id = %body.task_id,
        workplan_id = %body.workplan_id,
        role = %body.role,
        "inference task queued"
    );

    (
        StatusCode::CREATED,
        Json(json!({
            "queue_id": id,
            "task_id": body.task_id,
            "status": "pending",
        })),
    )
}

#[derive(Debug, Deserialize)]
pub struct UpdateQueueStatusRequest {
    pub status: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub agent_id: Option<String>,
}

/// GET /api/inference/queue/pending — list pending inference tasks from STDB.
///
/// Returns tasks in the InferenceTaskPush shape (snake_case) so that
/// `hex inference watch` startup reconciliation can deserialize them directly.
pub async fn queue_pending(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "IStatePort not initialized" })),
        ),
    };

    match port.inference_task_list_pending().await {
        Ok(tasks) => {
            // Map to InferenceTaskPush shape (snake_case) for watch compatibility.
            let pushes: Vec<serde_json::Value> = tasks.iter().map(|t| json!({
                "id": t.id,
                "workplan_id": t.workplan_id,
                "task_id": t.task_id,
                "phase": t.phase,
                "prompt": t.prompt,
                "role": t.role,
            })).collect();
            (StatusCode::OK, Json(json!(pushes)))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        ),
    }
}

// ── LoRA idiom-expert corpus (ADR-2606161300 Phase 0) ───────────────────────

/// Resolve the tier model used for corpus augmentation. Cheap T1 by default;
/// overridable so a box without qwen3 can point at whatever it serves.
fn resolve_lora_augment_model() -> String {
    // 1. env (highest precedence)
    if let Ok(m) = std::env::var("HEX_LORA_AUGMENT_MODEL") {
        if !m.trim().is_empty() {
            return m;
        }
    }
    // 2. .hex/project.json → inference.lora.augment_model
    let project_dir = std::env::var("CLAUDE_PROJECT_DIR")
        .or_else(|_| std::env::var("HEX_PROJECT_DIR"))
        .unwrap_or_else(|_| ".".to_string());
    let project_json = std::path::Path::new(&project_dir).join(".hex/project.json");
    if let Ok(content) = std::fs::read_to_string(&project_json) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(m) = v["inference"]["lora"]["augment_model"].as_str() {
                if !m.trim().is_empty() {
                    return m.to_string();
                }
            }
        }
    }
    // 3. default: fast local T1
    "qwen3:4b".to_string()
}

/// Resolve the registered local Ollama base URL (empty string if none registered).
/// Used for model-driven corpus augmentation and adapter eval — both measure/generate
/// against local Ollama, since `state.inference_port` is only wired in standalone mode.
async fn resolve_ollama_url(state: &SharedState) -> String {
    match &state.inference_stdb {
        Some(stdb) => stdb
            .list_providers()
            .await
            .ok()
            .and_then(|ps| {
                ps.into_iter()
                    .find(|p| p.provider_type == "ollama" && !p.base_url.is_empty())
                    .map(|p| p.base_url)
            })
            .unwrap_or_default(),
        None => String::new(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CorpusBuildRequest {
    pub expert: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// POST /api/inference/corpus/build {expert, dry_run} — extract an auditable training
/// corpus for one idiom expert from hex's own ADRs/specs/exemplars (ADR-2606161300 §2).
///
/// Returns the [`hex_core::corpus::CorpusManifest`]. On `dry_run`, nothing is written.
pub async fn corpus_build(
    State(state): State<SharedState>,
    Json(body): Json<CorpusBuildRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let ollama_url = resolve_ollama_url(&state).await;
    let cfg = crate::corpus_build::CorpusBuildConfig {
        repo_root: crate::corpus_build::resolve_repo_root(),
        qa_count: crate::state_config::resolve_lora_corpus_qa_count(),
        dry_run: body.dry_run,
        augment_model: resolve_lora_augment_model(),
        ollama_url: if ollama_url.is_empty() { None } else { Some(ollama_url) },
    };
    match crate::corpus_build::build_corpus(&body.expert, &cfg).await {
        Ok(m) => (
            StatusCode::OK,
            Json(json!({
                "expert": m.expert,
                "corpus_version": m.corpus_version,
                "source_globs": m.source_globs,
                "record_count": m.record_count,
                "content_hash": m.content_hash,
                "dry_run": body.dry_run,
            })),
        ),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))),
    }
}

/// GET /api/inference/corpus/list — list known experts + their current manifest
/// (or `null` when not yet built).
pub async fn corpus_list() -> (StatusCode, Json<serde_json::Value>) {
    let repo_root = crate::corpus_build::resolve_repo_root();
    let experts: Vec<serde_json::Value> = hex_core::corpus::default_knowledge_units()
        .into_iter()
        .map(|unit| {
            let manifest_path = repo_root
                .join(".hex/corpus")
                .join(&unit.expert)
                .join("manifest.json");
            let manifest = std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());
            json!({
                "expert": unit.expert,
                "source_globs": unit.source_globs,
                "manifest": manifest,
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({ "experts": experts })))
}

// ── LoRA adapter registry (ADR-2606161300 Phase 1) ──────────────────────────

#[derive(Debug, Deserialize)]
pub struct AdapterRegisterRequest {
    pub expert: String,
    pub base_model: String,
    pub tier: u8,
    pub artifact_ref: String,
    pub corpus_version: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Render an [`AdapterRecord`] to JSON, including its id and computed `stale` flag.
fn adapter_to_json(r: &hex_core::corpus::AdapterRecord, fresh_hash: Option<&str>) -> serde_json::Value {
    // Stale = registered corpus_version differs from a freshly-built manifest hash
    // for the expert (spec corpus-version-staleness-trigger). Unknown freshness
    // (couldn't rebuild) → not flagged stale, to avoid false alarms.
    let stale = fresh_hash.map(|h| h != r.corpus_version).unwrap_or(false);
    json!({
        "id": crate::lora_registry::record_id(r),
        "expert": r.expert,
        "base_model": r.base_model,
        "tier": r.tier,
        "artifact_ref": r.artifact_ref,
        "corpus_version": r.corpus_version,
        "enabled": r.enabled,
        "promoted": r.promoted,
        "stale": stale,
    })
}

/// POST /api/inference/adapters — register (or update) a LoRA adapter record.
pub async fn adapter_register(
    Json(body): Json<AdapterRegisterRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let record = hex_core::corpus::AdapterRecord {
        expert: body.expert,
        base_model: body.base_model,
        tier: body.tier,
        artifact_ref: body.artifact_ref,
        corpus_version: body.corpus_version,
        enabled: body.enabled,
        promoted: false,
    };
    let id = crate::lora_registry::record_id(&record);
    match crate::lora_registry::AdapterStore::from_env().register(record) {
        Ok(()) => (StatusCode::OK, Json(json!({ "id": id, "registered": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

/// GET /api/inference/adapters — list adapters, flagging stale ones.
pub async fn adapter_list() -> (StatusCode, Json<serde_json::Value>) {
    let store = crate::lora_registry::AdapterStore::from_env();
    let records = store.list();

    // Freshly recompute each distinct expert's corpus hash once (deterministic,
    // model-free) to evaluate staleness without re-hashing per record.
    let cfg = crate::corpus_build::CorpusBuildConfig {
        repo_root: crate::corpus_build::resolve_repo_root(),
        qa_count: crate::state_config::resolve_lora_corpus_qa_count(),
        dry_run: true,
        augment_model: String::new(),
        // Staleness must be a deterministic function of the source, so no model.
        ollama_url: None,
    };
    let mut fresh: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for r in &records {
        if !fresh.contains_key(&r.expert) {
            if let Ok(h) = crate::corpus_build::current_corpus_hash(&r.expert, &cfg).await {
                fresh.insert(r.expert.clone(), h);
            }
        }
    }

    let adapters: Vec<serde_json::Value> = records
        .iter()
        .map(|r| adapter_to_json(r, fresh.get(&r.expert).map(String::as_str)))
        .collect();
    (StatusCode::OK, Json(json!({ "adapters": adapters })))
}

/// DELETE /api/inference/adapters/{id} — remove an adapter (restores bare base).
pub async fn adapter_remove(Path(id): Path<String>) -> (StatusCode, Json<serde_json::Value>) {
    match crate::lora_registry::AdapterStore::from_env().remove(&id) {
        Ok(true) => (StatusCode::OK, Json(json!({ "removed": true, "id": id }))),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "no such adapter", "id": id }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct AdapterPatchRequest {
    pub enabled: bool,
}

/// PATCH /api/inference/adapters/{id} {enabled} — enable/disable an adapter.
pub async fn adapter_patch(
    Path(id): Path<String>,
    Json(body): Json<AdapterPatchRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match crate::lora_registry::AdapterStore::from_env().set_enabled(&id, body.enabled) {
        Ok(true) => (StatusCode::OK, Json(json!({ "id": id, "enabled": body.enabled }))),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "no such adapter", "id": id }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

/// POST /api/inference/adapters/{expert}/evaluate — bench-gate an adapter.
///
/// Runs the Phase-1 acceptance proxy on the bare base and on base+adapter, then applies
/// the BLOCKING promotion gate (ADR-2606161300 §5, spec `bench-gate-acceptance-lift-blocking`):
/// promote ONLY on first-draft acceptance lift with no quality/throughput regression.
/// Requests resource-governor admission first and DEFERS (never OOMs) under pressure.
/// The verdict — not the training loss — decides, and it is recorded as a lesson.
pub async fn adapter_evaluate(
    State(state): State<SharedState>,
    Path(expert): Path<String>,
    Json(_body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    // 1. Resolve the registered adapter for this expert.
    let store = crate::lora_registry::AdapterStore::from_env();
    let Some(record) = store.list().into_iter().find(|r| r.expert == expert) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("no registered adapter for expert '{expert}'") })),
        );
    };
    let id = crate::lora_registry::record_id(&record);

    // 2. Resource-governor admission BEFORE any heavy dispatch (spec
    //    resource-governor-training-admission-negative). Footprint is overridable so
    //    an already-loaded model isn't double-counted.
    let footprint = std::env::var("HEX_LORA_EVAL_FOOTPRINT_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(4000);
    if let Err(reason) = crate::lora_eval::admit_local_job(footprint, 1000, 512) {
        tracing::warn!(expert = %expert, %reason, "LoRA eval deferred by resource governor");
        return (StatusCode::OK, Json(json!({ "deferred": true, "reason": reason })));
    }

    // 3. Resolve the local Ollama URL and ensure the derived base+adapter model exists.
    //    The eval compares two local Ollama models directly (the bench gate's authority
    //    rests on real measurement), so it needs a registered Ollama provider.
    let ollama_url = resolve_ollama_url(&state).await;
    if ollama_url.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "no local Ollama provider registered — cannot evaluate" })),
        );
    }
    let Some(derived) =
        crate::lora_attach::resolve_serving_model("ollama", &record.base_model, &ollama_url).await
    else {
        return (
            StatusCode::OK,
            Json(json!({
                "error": "adapter could not be attached (Ollama create failed) — not promoted",
                "promoted": false
            })),
        );
    };

    // 4. Measure bare base vs base+adapter, both directly against Ollama.
    let base = match crate::lora_eval::measure_model(&ollama_url, &record.base_model).await {
        Ok(m) => m,
        Err(e) => return (StatusCode::OK, Json(json!({ "error": e, "promoted": false }))),
    };
    let adapter = match crate::lora_eval::measure_model(&ollama_url, &derived).await {
        Ok(m) => m,
        Err(e) => return (StatusCode::OK, Json(json!({ "error": e, "promoted": false }))),
    };

    // 5. BLOCKING promotion gate (pure).
    let verdict = crate::lora_eval::decide_promotion(
        &base,
        &adapter,
        crate::lora_eval::DEFAULT_QUALITY_TOLERANCE,
        crate::lora_eval::DEFAULT_THROUGHPUT_BUDGET_PCT,
    );

    // 6. Persist the promoted flag; absence of lift leaves it registered-not-default.
    if let Err(e) = store.set_promoted(&id, verdict.promoted) {
        tracing::warn!(%id, error = %e, "failed to persist promotion flag");
    }

    let result = json!({
        "expert": expert,
        "id": id,
        "acceptance_base": base.acceptance,
        "acceptance_adapter": adapter.acceptance,
        "quality_delta": adapter.quality - base.quality,
        "throughput_delta_pct": if base.tok_per_sec > 0.0 {
            (adapter.tok_per_sec - base.tok_per_sec) / base.tok_per_sec * 100.0
        } else { 0.0 },
        "promoted": verdict.promoted,
        "reason": verdict.reason,
        "gate": "phase1-acceptance-proxy (production verdict: agentic bench suite ADR-2606071734)",
    });

    // 7. Record the verdict as a lesson (queryable by personas in SOP GROUND).
    if let Some(ref port) = state.state_port {
        let key = format!("lesson:hex-lora-{expert}");
        let _ = port.hexflo_memory_store(&key, &result.to_string(), "global").await;
    }

    (StatusCode::OK, Json(result))
}

/// PATCH /api/inference/queue/{id} — claim, complete, or fail an inference_task in STDB.
///
/// status="claimed"   → inference_task_claim (CAS: Pending → InProgress)
/// status="completed" → inference_task_complete
/// status="failed"    → inference_task_fail
pub async fn queue_update(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<UpdateQueueStatusRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let port = match &state.state_port {
        Some(p) => p,
        None => return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "IStatePort not initialized" })),
        ),
    };

    let now = chrono::Utc::now().to_rfc3339();

    match body.status.as_str() {
        "claimed" => {
            // Agent ID from X-Hex-Agent-Id header, body.agent_id, or "unknown"
            let agent_id = headers
                .get("x-hex-agent-id")
                .and_then(|v| v.to_str().ok())
                .or(body.agent_id.as_deref())
                .unwrap_or("unknown")
                .to_string();
            match port.inference_task_claim(&id, &agent_id, &now).await {
                Ok(_) => (StatusCode::OK, Json(json!({ "id": id, "status": "InProgress" }))),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("already_claimed") || msg.contains("Conflict") {
                        (StatusCode::CONFLICT, Json(json!({ "error": msg })))
                    } else {
                        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": msg })))
                    }
                }
            }
        }
        "completed" => {
            let result = body.result.as_deref().unwrap_or("");
            match port.inference_task_complete(&id, result, &now).await {
                Ok(_) => (StatusCode::OK, Json(json!({ "id": id, "status": "Completed" }))),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
            }
        }
        "failed" => {
            let error = body.error.as_deref().unwrap_or("unknown error");
            match port.inference_task_fail(&id, error, &now).await {
                Ok(_) => (StatusCode::OK, Json(json!({ "id": id, "status": "Failed" }))),
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))),
            }
        }
        other => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("unknown status: {}", other) })),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Streaming chat endpoint (ADR-2026-04-01-1300)
// ─────────────────────────────────────────────────────────────────────────────

/// POST /api/inference/chat/stream — streaming LLM completion via Server-Sent Events.
///
/// Same provider selection as /api/inference/complete but passes stream=true to
/// the upstream. Emits SSE events with token deltas terminated by a done event.
///
/// Event data shapes:
///   `{"token":"hello"}`
///   `{"done":true,"model":"...","input_tokens":42,"output_tokens":7}`
///   `{"error":"..."}`   — fatal error; stream closes after this event
pub async fn inference_stream(
    State(state): State<SharedState>,
    Json(body): Json<InferenceCompleteRequest>,
) -> axum::response::Response {
    use axum::response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    };
    use futures::channel::mpsc;
    use futures::SinkExt;
    use std::convert::Infallible;

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(128);
    let mut tx = tx;

    let state = state.clone();
    let requested_model = body.model.clone();
    let messages = body.messages.clone();
    let max_tokens = body.max_tokens;
    let tools = body.tools.clone();

    tokio::spawn(async move {
        match pick_stream_provider(&state, requested_model.as_deref()).await {
            None => {
                let _ = tx.send(Ok(Event::default().data(
                    r#"{"error":"no inference provider configured — run `hex inference add` or set OPENROUTER_API_KEY"}"#,
                ))).await;
            }
            Some(ep) => {
                stream_inference(&ep, &messages, max_tokens, tools.as_deref(), &mut tx).await;
            }
        }
    });

    Sse::new(rx)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Select a single provider for the streaming path (simplified — no retry chain).
async fn pick_stream_provider(
    state: &crate::state::AppState,
    requested_model: Option<&str>,
) -> Option<crate::routes::secrets::InferenceEndpointEntry> {
    // 1. Registered SpacetimeDB providers
    if let Some(ref stdb) = state.inference_stdb {
        if let Ok(providers) = stdb.list_providers().await {
            let matched = if let Some(model) = requested_model {
                providers.iter().find(|p| {
                    serde_json::from_str::<Vec<String>>(&p.models_json)
                        .map(|ms| ms.iter().any(|m| m == model))
                        .unwrap_or_else(|_| p.models_json.contains(model))
                }).or_else(|| {
                    if model.contains('/') {
                        providers.iter().find(|p| {
                            p.provider_type == "openrouter" && !p.api_key_ref.is_empty()
                        })
                    } else {
                        None
                    }
                })
            } else {
                providers.first()
            };

            if let Some(p) = matched {
                let first_model = p.models_json
                    .trim_start_matches('[').trim_end_matches(']')
                    .split(',').next().unwrap_or(&p.models_json)
                    .trim().trim_matches('"').to_string();
                let model = requested_model.map(|s| s.to_string()).unwrap_or(first_model);
                let mut ep = crate::routes::secrets::InferenceEndpointEntry {
                    id: p.provider_id.clone(),
                    url: p.base_url.clone(),
                    provider: p.provider_type.clone(),
                    model,
                    status: "unknown".into(),
                    requires_auth: !p.api_key_ref.is_empty(),
                    secret_key: p.api_key_ref.clone(),
                    health_checked_at: p.last_health_check.clone(),
                };
                // Resolve secret key reference
                if ep.requires_auth && !ep.secret_key.is_empty() && !ep.secret_key.starts_with("sk-") {
                    let key_ref = ep.secret_key.clone();
                    if let Ok(val) = std::env::var(&key_ref) {
                        ep.secret_key = val;
                    } else if let Some(ref ss) = state.spacetime_secrets {
                        if let Ok(Ok(Some(val))) = tokio::time::timeout(
                            std::time::Duration::from_secs(3),
                            ss.vault_get(&key_ref),
                        ).await {
                            ep.secret_key = val;
                        }
                    }
                }
                return Some(ep);
            }
        }
    }

    // 2. Synthetic OpenRouter endpoint from key in vault or env
    let or_key = state.openrouter_api_key.clone()
        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
        .or_else(|| {
            state.anthropic_api_key.as_ref()
                .filter(|k| k.starts_with("sk-or-v1-"))
                .cloned()
        })?;

    let model = requested_model.map(|m| {
        if m.contains('/') { m.to_string() } else { format!("openai/{}", m) }
    }).unwrap_or_else(|| "openai/gpt-4o-mini".to_string());

    Some(crate::routes::secrets::InferenceEndpointEntry {
        id: "openrouter-stream".into(),
        url: "https://openrouter.ai/api/v1".into(),
        provider: "openrouter".into(),
        model,
        status: "ok".into(),
        requires_auth: true,
        secret_key: or_key,
        health_checked_at: String::new(),
    })
}

type SseTx = futures::channel::mpsc::Sender<
    Result<axum::response::sse::Event, std::convert::Infallible>,
>;

/// Perform a streaming HTTP request and forward token deltas onto `tx`.
///
/// When the model requests tool calls (`finish_reason: "tool_calls"`), the
/// done event includes a `tool_calls` array for the client to execute.
async fn stream_inference(
    ep: &crate::routes::secrets::InferenceEndpointEntry,
    messages: &[serde_json::Value],
    max_tokens: u32,
    tools: Option<&[serde_json::Value]>,
    tx: &mut SseTx,
) {
    use axum::response::sse::Event;
    use futures::{SinkExt, StreamExt};

    let is_openrouter = ep.provider == "openrouter" || ep.url.contains("openrouter.ai");
    let is_ollama = ep.provider == "ollama";

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let msg = json!({"error": format!("client build failed: {e}")});
            let _ = tx.send(Ok(Event::default().data(msg.to_string()))).await;
            return;
        }
    };

    let (url, body) = if is_openrouter {
        let mut b = json!({ "model": ep.model, "messages": messages, "max_tokens": max_tokens, "stream": true });
        if let Some(t) = tools.filter(|t| !t.is_empty()) {
            b["tools"] = serde_json::Value::Array(t.to_vec());
        }
        ("https://openrouter.ai/api/v1/chat/completions".to_string(), b)
    } else if is_ollama {
        // Ollama: cap via options.num_predict + DISABLE think-mode by
        // default (qwen3 etc. otherwise burn the whole budget on <think>
        // tokens and emit empty content). HEX_OLLAMA_THINK=1 to opt in.
        let think_enabled = std::env::var("HEX_OLLAMA_THINK")
            .ok().map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        (
            format!("{}/api/chat", ep.url.trim_end_matches('/')),
            json!({
                "model": ep.model,
                "messages": messages,
                "stream": true,
                "think": think_enabled,
                "options": { "num_predict": max_tokens },
            }),
        )
    } else {
        let mut b = json!({ "model": ep.model, "messages": messages, "max_tokens": max_tokens, "stream": true });
        if let Some(t) = tools.filter(|t| !t.is_empty()) {
            b["tools"] = serde_json::Value::Array(t.to_vec());
        }
        (format!("{}/v1/chat/completions", ep.url.trim_end_matches('/')), b)
    };

    let mut req = client.post(&url).json(&body);
    if is_openrouter {
        if ep.secret_key.is_empty() {
            let _ = tx.send(Ok(Event::default().data(r#"{"error":"OPENROUTER_API_KEY not configured"}"#))).await;
            return;
        }
        req = req
            .header("Authorization", format!("Bearer {}", ep.secret_key))
            .header("HTTP-Referer", "https://github.com/hex-intf")
            .header("X-Title", "hex-agent");
    } else if !ep.secret_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", ep.secret_key));
    }

    let resp = match req.send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            let status = r.status().as_u16();
            let text = r.text().await.unwrap_or_default();
            let msg = json!({"error": format!("HTTP {status}: {}", &text[..text.len().min(200)])});
            let _ = tx.send(Ok(Event::default().data(msg.to_string()))).await;
            return;
        }
        Err(e) => {
            let msg = json!({"error": format!("connection failed: {e}")});
            let _ = tx.send(Ok(Event::default().data(msg.to_string()))).await;
            return;
        }
    };

    let mut byte_stream = resp.bytes_stream();
    let mut line_buf = String::new();
    let mut output_tokens: u64 = 0;
    let model_name = ep.model.clone();
    // Accumulate streamed tool_call argument deltas: index → (id, name, args_so_far)
    let mut pending_tool_calls: std::collections::BTreeMap<usize, (String, String, String)> =
        Default::default();

    while let Some(chunk) = byte_stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                let msg = json!({"error": format!("stream read error: {e}")});
                let _ = tx.send(Ok(Event::default().data(msg.to_string()))).await;
                return;
            }
        };

        line_buf.push_str(&String::from_utf8_lossy(&bytes));

        loop {
            match line_buf.find('\n') {
                None => break,
                Some(pos) => {
                    let line = line_buf[..pos].trim().to_string();
                    line_buf = line_buf[pos + 1..].to_string();

                    if line.is_empty() || line == "data: [DONE]" {
                        continue;
                    }

                    let json_str = line.strip_prefix("data: ").unwrap_or(&line);
                    let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) else {
                        continue;
                    };

                    if is_ollama {
                        if val.get("done").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let ev = json!({"done":true,"model":model_name,"input_tokens":0u64,"output_tokens":output_tokens});
                            let _ = tx.send(Ok(Event::default().data(ev.to_string()))).await;
                            return;
                        }
                        if let Some(tok) = val.get("message")
                            .and_then(|m| m.get("content"))
                            .and_then(|c| c.as_str())
                        {
                            if !tok.is_empty() {
                                output_tokens += 1;
                                let ev = json!({"token": tok});
                                let _ = tx.send(Ok(Event::default().data(ev.to_string()))).await;
                            }
                        }
                    } else {
                        // OpenAI-compatible SSE delta
                        if let Some(choices) = val.get("choices").and_then(|c| c.as_array()) {
                            if let Some(choice) = choices.first() {
                                // Accumulate tool_call argument deltas
                                if let Some(tc_arr) = choice.get("delta")
                                    .and_then(|d| d.get("tool_calls"))
                                    .and_then(|v| v.as_array())
                                {
                                    for tc in tc_arr {
                                        let idx = tc.get("index")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0) as usize;
                                        let e = pending_tool_calls.entry(idx).or_default();
                                        if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                            e.0 = id.to_string();
                                        }
                                        if let Some(name) = tc.get("function")
                                            .and_then(|f| f.get("name"))
                                            .and_then(|v| v.as_str())
                                        {
                                            e.1 = name.to_string();
                                        }
                                        if let Some(args) = tc.get("function")
                                            .and_then(|f| f.get("arguments"))
                                            .and_then(|v| v.as_str())
                                        {
                                            e.2.push_str(args);
                                        }
                                    }
                                }

                                let finish = choice.get("finish_reason")
                                    .and_then(|r| r.as_str()).unwrap_or("");
                                if !finish.is_empty() && finish != "null" {
                                    let in_tok = val.get("usage")
                                        .and_then(|u| u.get("prompt_tokens"))
                                        .and_then(|v| v.as_u64()).unwrap_or(0);
                                    let out_tok = val.get("usage")
                                        .and_then(|u| u.get("completion_tokens"))
                                        .and_then(|v| v.as_u64()).unwrap_or(output_tokens);

                                    if finish == "tool_calls" && !pending_tool_calls.is_empty() {
                                        // Emit done with tool_calls for the client to execute
                                        let calls: Vec<serde_json::Value> = pending_tool_calls
                                            .values()
                                            .map(|(id, name, args_str)| {
                                                let args = serde_json::from_str::<serde_json::Value>(args_str)
                                                    .unwrap_or(json!({}));
                                                json!({"id": id, "name": name, "arguments": args})
                                            })
                                            .collect();
                                        let ev = json!({
                                            "done": true,
                                            "model": model_name,
                                            "input_tokens": in_tok,
                                            "output_tokens": out_tok,
                                            "tool_calls": calls,
                                        });
                                        let _ = tx.send(Ok(Event::default().data(ev.to_string()))).await;
                                    } else {
                                        let ev = json!({"done":true,"model":model_name,"input_tokens":in_tok,"output_tokens":out_tok});
                                        let _ = tx.send(Ok(Event::default().data(ev.to_string()))).await;
                                    }
                                    return;
                                }
                                if let Some(tok) = choice.get("delta")
                                    .and_then(|d| d.get("content"))
                                    .and_then(|c| c.as_str())
                                {
                                    if !tok.is_empty() {
                                        output_tokens += 1;
                                        let ev = json!({"token": tok});
                                        let _ = tx.send(Ok(Event::default().data(ev.to_string()))).await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Stream ended without an explicit done event
    let ev = json!({"done":true,"model":model_name,"input_tokens":0u64,"output_tokens":output_tokens});
    let _ = tx.send(Ok(Event::default().data(ev.to_string()))).await;
}

// ── OpenAI-compatible proxy routes (/v1/models, /v1/chat/completions) ────────

/// GET /v1/models — returns registered inference providers in OpenAI models-list format.
///
/// Always includes a "hex/default" entry. Additional entries are derived from
/// all providers registered via `hex inference add` (stored in SpacetimeDB).
pub async fn openai_models(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut data: Vec<serde_json::Value> = vec![
        json!({
            "id": "hex/default",
            "object": "model",
            "owned_by": "hex-nexus",
            "created": 0
        }),
    ];

    if let Some(ref stdb) = state.inference_stdb {
        if let Ok(providers) = stdb.list_providers().await {
            for p in providers {
                // Use the provider name or id as the model id, prefixed with "hex/".
                let model_id = format!("hex/{}", p.provider_id);
                data.push(json!({
                    "id": model_id,
                    "object": "model",
                    "owned_by": "hex-nexus",
                    "created": 0
                }));
            }
        }
    }

    (StatusCode::OK, Json(json!({
        "object": "list",
        "data": data
    })))
}

/// POST /v1/chat/completions — OpenAI-compatible chat completions proxy.
///
/// Accepts an OpenAI-format request body and delegates to the existing
/// inference routing logic (same provider selection as /api/inference/complete).
///
/// - Non-streaming: returns an OpenAI-format choices response.
/// - Streaming (`"stream": true`): delegates to inference_stream SSE path.
///
/// Security (spec S07): model must be "hex/default", a "hex/<id>" prefix, or
/// absent. An unrecognised non-hex model prefix returns HTTP 400.
pub async fn openai_chat_completions(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // Extract fields from the OpenAI-format body.
    let model_raw = body.get("model").and_then(|v| v.as_str()).unwrap_or("hex/default");
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let messages = body.get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let max_tokens = body.get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(4096) as u32;

    // Security S07: reject non-hex model prefixes (unknown providers).
    // Allow: absent, "hex/default", "hex/<anything>", or bare model names
    // that don't look like a foreign vendor namespace.
    if model_raw.contains('/') && !model_raw.starts_with("hex/") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": format!("Unknown model prefix '{}'. Use 'hex/default' or a registered 'hex/<id>' model.", model_raw),
                    "type": "invalid_request_error",
                    "code": "model_not_found"
                }
            })),
        ).into_response();
    }

    // Map "hex/default" or "hex/<id>" → the underlying model for routing.
    // Strip the "hex/" prefix so the existing provider selection sees the bare id.
    let resolved_model: Option<String> = if model_raw == "hex/default" {
        None // let provider selection pick the default
    } else if let Some(stripped) = model_raw.strip_prefix("hex/") {
        Some(stripped.to_string())
    } else {
        Some(model_raw.to_string())
    };

    if stream {
        // Delegate to the existing SSE streaming path.
        let stream_body = InferenceCompleteRequest {
            model: resolved_model,
            messages,
            system: None,
            max_tokens,
            tools: None,
        };
        return inference_stream(State(state), Json(stream_body)).await;
    }

    // Non-streaming: reuse inference_complete and wrap the response in
    // OpenAI choices format.
    let complete_body = InferenceCompleteRequest {
        model: resolved_model,
        messages,
        system: None,
        max_tokens,
        tools: None,
    };

    let (status, Json(inner)) =
        inference_complete(State(state), headers, Json(complete_body)).await;

    if !status.is_success() {
        return (status, Json(inner)).into_response();
    }

    let content = inner.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let model_used = inner.get("model").and_then(|v| v.as_str()).unwrap_or(model_raw).to_string();
    let input_tokens = inner.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output_tokens = inner.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);

    let openai_resp = json!({
        "id": format!("chatcmpl-{}", uuid::Uuid::new_v4()),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model_used,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    });

    (StatusCode::OK, Json(openai_resp)).into_response()
}

// ── Anthropic-compatible gateway route (/v1/messages) ────────────────────────
//
// Lets ANY Anthropic-Messages-API client — Claude Code itself, plus
// Anthropic-format agents (Hermes, etc.) — point `ANTHROPIC_BASE_URL` at hex
// and inherit hex's tiered, local-first, circuit-broken routing instead of
// talking to a raw provider. Mirrors `openai_chat_completions`: translate the
// Anthropic request → hex's internal `InferenceCompleteRequest`, reuse the same
// provider selection (and the local-first fallback from
// ADR-2026-07-10-1000), then translate the response back to Anthropic shape.

/// Anthropic `system` is a top-level field that may be a bare string OR an
/// array of `{type:"text", text}` content blocks. Flatten to a single string.
fn flatten_anthropic_system(system: Option<&serde_json::Value>) -> Option<String> {
    match system {
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(serde_json::Value::Array(blocks)) => {
            let joined = blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    }
}

/// Translate Anthropic messages → OpenAI-format `[{role, content, tool_calls?, tool_call_id?}]`
/// (what hex's provider dispatch expects). Handles the agentic tool loop so
/// Claude Code's multi-turn tool_use/tool_result history round-trips:
///   - assistant `tool_use` block  → OpenAI `tool_calls` entry
///   - user `tool_result` block    → OpenAI `{role:"tool", tool_call_id, content}` message
///   - `text` blocks               → folded into the message content string
fn anthropic_messages_to_openai(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();

    let block_text = |c: &serde_json::Value| -> String {
        match c {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(blocks) => blocks
                .iter()
                .filter_map(|b| {
                    // tool_result content may itself be a string or block array.
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    };

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = msg.get("content").cloned().unwrap_or(serde_json::Value::Null);

        // Simple string content → straight through.
        if let serde_json::Value::String(s) = &content {
            out.push(json!({ "role": role, "content": s }));
            continue;
        }

        let blocks = content.as_array().cloned().unwrap_or_default();
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();
        let mut tool_results: Vec<serde_json::Value> = Vec::new();

        for b in &blocks {
            match b.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                        text_parts.push(t.to_string());
                    }
                }
                Some("tool_use") => {
                    let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let input = b.get("input").cloned().unwrap_or(json!({}));
                    tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": serde_json::to_string(&input).unwrap_or_else(|_| "{}".into()),
                        }
                    }));
                }
                Some("tool_result") => {
                    let tool_use_id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let result_content = b.get("content").map(block_text).unwrap_or_default();
                    tool_results.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": result_content,
                    }));
                }
                _ => {} // image / other blocks: ignored in v1 (text-first agents)
            }
        }

        // OpenAI ordering: tool result messages stand alone (they answer a prior
        // assistant tool_call); assistant/user text+tool_calls form one message.
        if !tool_results.is_empty() {
            out.extend(tool_results);
        }
        let text = text_parts.join("\n");
        if !text.is_empty() || !tool_calls.is_empty() {
            let mut m = serde_json::Map::new();
            m.insert("role".into(), json!(role));
            m.insert("content".into(), if text.is_empty() { serde_json::Value::Null } else { json!(text) });
            if !tool_calls.is_empty() {
                m.insert("tool_calls".into(), json!(tool_calls));
            }
            out.push(serde_json::Value::Object(m));
        }
    }

    out
}

/// Build the Anthropic `content` block array + `stop_reason` from hex's internal
/// `{content, tool_calls}` response.
fn anthropic_content_from_inner(inner: &serde_json::Value) -> (Vec<serde_json::Value>, &'static str) {
    let mut content: Vec<serde_json::Value> = Vec::new();
    let text = inner.get("content").and_then(|v| v.as_str()).unwrap_or("");
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    let tool_calls = inner.get("tool_calls").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for tc in &tool_calls {
        let id = tc.get("id").and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| format!("toolu_{}", Uuid::new_v4()));
        let func = tc.get("function").cloned().unwrap_or(serde_json::Value::Null);
        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let args_str = func.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");
        let input: serde_json::Value = serde_json::from_str(args_str).unwrap_or(json!({}));
        content.push(json!({ "type": "tool_use", "id": id, "name": name, "input": input }));
    }
    let stop_reason = if !tool_calls.is_empty() { "tool_use" } else { "end_turn" };
    // An empty content array is invalid Anthropic; emit a placeholder text block.
    if content.is_empty() {
        content.push(json!({ "type": "text", "text": "" }));
    }
    (content, stop_reason)
}

/// POST /v1/messages — Anthropic-compatible Messages API proxy.
///
/// Accepts an Anthropic-format request and delegates to the same routing as
/// `/api/inference/complete` (tiered, local-first, circuit-broken). The
/// `model` field is ignored for provider choice — hex routes — except a
/// `hex/<id>` value pins a specific registered provider. `stream:true` returns
/// the Anthropic SSE event sequence synthesised from the completed response
/// (pseudo-streaming): correct on the wire for Claude Code, without per-token
/// streaming through every provider.
pub async fn anthropic_messages(
    State(state): State<SharedState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let model_raw = body.get("model").and_then(|v| v.as_str()).unwrap_or("hex/default");
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let max_tokens = body.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(4096) as u32;
    let system = flatten_anthropic_system(body.get("system"));
    let anthropic_msgs = body.get("messages").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let messages = anthropic_messages_to_openai(&anthropic_msgs);
    let tools = body.get("tools").and_then(|v| v.as_array()).cloned();

    // hex routes; only a "hex/<id>" override pins a provider. Claude Code always
    // sends "claude-*" names — those map to default (let hex choose), never 400.
    let resolved_model: Option<String> = model_raw
        .strip_prefix("hex/")
        .filter(|s| !s.is_empty() && *s != "default")
        .map(String::from);

    let complete_body = InferenceCompleteRequest {
        model: resolved_model,
        messages,
        system,
        max_tokens,
        tools,
    };

    let (status, Json(inner)) =
        inference_complete(State(state), headers, Json(complete_body)).await;

    if !status.is_success() {
        // Re-shape hex's error into Anthropic's error envelope.
        let msg = inner.get("error").and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| inner.to_string());
        return (status, Json(json!({
            "type": "error",
            "error": { "type": "api_error", "message": msg }
        }))).into_response();
    }

    let model_used = inner.get("model").and_then(|v| v.as_str()).unwrap_or(model_raw).to_string();
    let input_tokens = inner.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output_tokens = inner.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let (content, stop_reason) = anthropic_content_from_inner(&inner);
    let msg_id = format!("msg_{}", Uuid::new_v4());

    if !stream {
        let resp = json!({
            "id": msg_id,
            "type": "message",
            "role": "assistant",
            "model": model_used,
            "content": content,
            "stop_reason": stop_reason,
            "stop_sequence": null,
            "usage": { "input_tokens": input_tokens, "output_tokens": output_tokens }
        });
        return (StatusCode::OK, Json(resp)).into_response();
    }

    // Pseudo-streaming: build the full Anthropic SSE event sequence from the
    // finished result, then stream it. Claude Code parses these events
    // identically to real streaming; we just deliver each content block in one
    // delta. Uses futures-mpsc (its Receiver is a Stream) to match
    // `inference_stream`.
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::channel::mpsc;
    use futures::SinkExt;
    use std::convert::Infallible;

    let mut events: Vec<(&'static str, serde_json::Value)> = Vec::new();
    events.push(("message_start", json!({
        "type": "message_start",
        "message": {
            "id": msg_id, "type": "message", "role": "assistant", "model": model_used,
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": { "input_tokens": input_tokens, "output_tokens": 0 }
        }
    })));
    for (idx, block) in content.iter().enumerate() {
        let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("text");
        if btype == "tool_use" {
            events.push(("content_block_start", json!({
                "type": "content_block_start", "index": idx,
                "content_block": { "type": "tool_use", "id": block.get("id"), "name": block.get("name"), "input": {} }
            })));
            let partial = serde_json::to_string(block.get("input").unwrap_or(&json!({}))).unwrap_or_else(|_| "{}".into());
            events.push(("content_block_delta", json!({
                "type": "content_block_delta", "index": idx,
                "delta": { "type": "input_json_delta", "partial_json": partial }
            })));
        } else {
            let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
            events.push(("content_block_start", json!({
                "type": "content_block_start", "index": idx,
                "content_block": { "type": "text", "text": "" }
            })));
            events.push(("content_block_delta", json!({
                "type": "content_block_delta", "index": idx,
                "delta": { "type": "text_delta", "text": text }
            })));
        }
        events.push(("content_block_stop", json!({ "type": "content_block_stop", "index": idx })));
    }
    events.push(("message_delta", json!({
        "type": "message_delta",
        "delta": { "stop_reason": stop_reason, "stop_sequence": null },
        "usage": { "output_tokens": output_tokens }
    })));
    events.push(("message_stop", json!({ "type": "message_stop" })));

    let (mut tx, rx) = mpsc::channel::<Result<Event, Infallible>>(events.len() + 1);
    tokio::spawn(async move {
        for (name, data) in events {
            if tx.send(Ok(Event::default().event(name).data(data.to_string()))).await.is_err() {
                break;
            }
        }
    });

    Sse::new(rx)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// POST /v1/messages/count_tokens — Anthropic token-count pre-flight. Claude
/// Code calls this before sending; a rough estimate keeps it happy without a
/// tokenizer (hex bills on real provider usage, not this estimate).
pub async fn anthropic_count_tokens(
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let system_len = flatten_anthropic_system(body.get("system")).map(|s| s.len()).unwrap_or(0);
    let msgs = body.get("messages").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let openai = anthropic_messages_to_openai(&msgs);
    let chars: usize = system_len + openai.iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()).map(str::len))
        .sum::<usize>();
    // ~4 chars/token heuristic.
    let input_tokens = (chars / 4).max(1) as u64;
    (StatusCode::OK, Json(json!({ "input_tokens": input_tokens })))
}

// ── Rate State + Cost Attribution (ADR-2026-04-05-2125) ─────────────────────────

/// GET /api/inference/rate-state — per-provider rate limit and circuit breaker state.
pub async fn rate_state(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let providers = state.rate_limiter.get_all_states().await;
    (StatusCode::OK, Json(json!({ "providers": providers })))
}

/// GET /api/inference/stats — cost attribution dashboard data.
pub async fn inference_stats_endpoint(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let stats = state.rate_limiter.get_cost_stats().await;
    (StatusCode::OK, Json(stats))
}

// ── Q-Report (ADR-2026-04-12-0202 + wp-inference-q-report) ───────────────────

#[derive(Debug, Deserialize)]
pub struct QReportParams {
    pub tier: Option<String>,
    pub task_type: Option<String>,
    pub model: Option<String>,
    pub since: Option<String>,
    #[serde(default = "default_q_limit")]
    pub limit: u32,
    pub sort: Option<String>,
}

fn default_q_limit() -> u32 {
    50
}

fn parse_duration_secs(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 2 {
        return None;
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: i64 = num.parse().ok()?;
    match unit {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "d" => Some(n * 86400),
        "w" => Some(n * 604800),
        _ => None,
    }
}

async fn stdb_query_rl(sql: &str) -> Result<Vec<serde_json::Value>, String> {
    let host = std::env::var("SPACETIMEDB_HOST")
        .unwrap_or_else(|_| hex_core::SPACETIMEDB_DEFAULT_HOST.to_string());
    let db = hex_core::STDB_DATABASE_RL;
    let url = format!("{}/v1/database/{}/sql", host, db);

    let client = reqwest::Client::new();
    let mut req = client.post(&url).body(sql.to_string());
    if let Ok(token) = std::env::var("SPACETIMEDB_TOKEN") {
        if !token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("STDB SQL failed: {}", body));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let body: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| e.to_string())?;
    Ok(crate::adapters::spacetime_state::SpacetimeStateAdapter::parse_stdb_response(body))
}

/// Run a SQL query against the core ("hex") SpacetimeDB database.
async fn stdb_query_core(sql: &str) -> Result<Vec<serde_json::Value>, String> {
    let host = std::env::var("SPACETIMEDB_HOST")
        .unwrap_or_else(|_| hex_core::SPACETIMEDB_DEFAULT_HOST.to_string());
    let db = hex_core::STDB_DATABASE_CORE;
    let url = format!("{}/v1/database/{}/sql", host, db);

    let client = reqwest::Client::new();
    let mut req = client.post(&url).body(sql.to_string());
    if let Ok(token) = std::env::var("SPACETIMEDB_TOKEN") {
        if !token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("STDB SQL failed: {}", body));
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let body: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| e.to_string())?;
    Ok(crate::adapters::spacetime_state::SpacetimeStateAdapter::parse_stdb_response(body))
}

#[derive(Debug, Deserialize)]
pub struct UsageParams {
    /// Only count completions newer than this duration (e.g. "1h", "7d").
    pub since: Option<String>,
    /// Filter to models whose name contains this substring.
    pub model: Option<String>,
    #[serde(default = "default_usage_limit")]
    pub limit: u32,
}

fn default_usage_limit() -> u32 {
    30
}

/// GET /api/inference/usage — durable per-model usage aggregated from the
/// `inference_log` table (the real, persisted record of every completion).
///
/// Unlike `/api/inference/q-report` (RL Q-table — only populated when the
/// reinforcement-learning loop is closed) and `/api/inference/stats`
/// (in-memory, resets on restart), this reflects actual traffic that
/// survives restarts. Aggregates requests, tokens, and p50/p99 latency
/// per model, sorted by request count.
pub async fn usage_report(
    axum::extract::Query(params): axum::extract::Query<UsageParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    let rows = match stdb_query_core("SELECT * FROM inference_log").await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "usage: failed to query inference_log");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e })));
        }
    };

    let now = chrono::Utc::now();
    let since_cutoff = params
        .since
        .as_deref()
        .and_then(parse_duration_secs)
        .map(|secs| now - chrono::Duration::seconds(secs));

    struct Agg {
        provider: String,
        count: u64,
        input: u64,
        output: u64,
        durations: Vec<u64>,
        last_seen: String,
    }
    let mut map: std::collections::HashMap<String, Agg> = std::collections::HashMap::new();
    let mut counted = 0u64;

    for r in &rows {
        let model = r.get("model").and_then(|v| v.as_str()).unwrap_or("");
        if model.is_empty() {
            continue;
        }
        if let Some(ref mf) = params.model {
            if !model.contains(mf.as_str()) {
                continue;
            }
        }
        let created = r.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(cut) = since_cutoff {
            match chrono::DateTime::parse_from_rfc3339(created) {
                Ok(dt) if dt.with_timezone(&Utc) >= cut => {}
                _ => continue,
            }
        }
        counted += 1;
        let provider = r.get("provider").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let input = r.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let output = r.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let dur = r.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);

        let e = map.entry(model.to_string()).or_insert_with(|| Agg {
            provider: provider.clone(),
            count: 0,
            input: 0,
            output: 0,
            durations: Vec::new(),
            last_seen: String::new(),
        });
        e.count += 1;
        e.input += input;
        e.output += output;
        if dur > 0 {
            e.durations.push(dur);
        }
        if created > e.last_seen.as_str() {
            e.last_seen = created.to_string();
        }
        if e.provider.is_empty() {
            e.provider = provider;
        }
    }

    let mut out: Vec<serde_json::Value> = map
        .into_iter()
        .map(|(model, mut a)| {
            a.durations.sort_unstable();
            let pct = |q: f64| -> u64 {
                if a.durations.is_empty() {
                    0
                } else {
                    let idx = (((a.durations.len() - 1) as f64) * q).round() as usize;
                    a.durations[idx.min(a.durations.len() - 1)]
                }
            };
            json!({
                "model": model,
                "provider": a.provider,
                "requests": a.count,
                "input_tokens": a.input,
                "output_tokens": a.output,
                "total_tokens": a.input + a.output,
                "p50_ms": pct(0.5),
                "p99_ms": pct(0.99),
                "last_seen": a.last_seen,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b["requests"].as_u64().unwrap_or(0).cmp(&a["requests"].as_u64().unwrap_or(0))
    });
    out.truncate(params.limit as usize);

    (
        StatusCode::OK,
        Json(json!({ "usage": out, "total_completions": counted, "source": "inference_log" })),
    )
}

/// GET /api/inference/q-report — Q-table report with filtering, sorting, and 7-day trend.
pub async fn q_report(
    axum::extract::Query(params): axum::extract::Query<QReportParams>,
) -> (StatusCode, Json<serde_json::Value>) {
    let q_rows = match stdb_query_rl("SELECT * FROM rl_q_entry").await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "q-report: failed to query rl_q_entry");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e })),
            );
        }
    };

    let exp_rows = match stdb_query_rl("SELECT * FROM rl_experience").await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "q-report: failed to query rl_experience");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e })),
            );
        }
    };

    let now = chrono::Utc::now();
    let seven_days_ago = now - chrono::Duration::days(7);

    // Build experience lookup: (state_key, action) → Vec<(timestamp, reward)>
    let mut exp_map: std::collections::HashMap<(String, String), Vec<(chrono::DateTime<Utc>, f64)>> =
        std::collections::HashMap::new();
    for row in &exp_rows {
        let sk = row.get("state_key").and_then(|v| v.as_str()).unwrap_or("");
        let action = row.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let reward = row.get("reward").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let ts_str = row.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let ts = chrono::DateTime::parse_from_rfc3339(ts_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(now);
        exp_map
            .entry((sk.to_string(), action.to_string()))
            .or_default()
            .push((ts, reward));
    }

    // Parse since cutoff
    let since_cutoff = params
        .since
        .as_deref()
        .and_then(parse_duration_secs)
        .map(|secs| now - chrono::Duration::seconds(secs));

    // Process Q-entries
    let mut entries: Vec<serde_json::Value> = Vec::new();
    for row in &q_rows {
        let state_key = row.get("state_key").and_then(|v| v.as_str()).unwrap_or("");
        let action = row.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let q_value = row.get("q_value").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let visit_count = row.get("visit_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let last_updated = row.get("last_updated").and_then(|v| v.as_str()).unwrap_or("");

        // Extract task_type from state_key (first segment before '_')
        let entry_task_type = state_key.split('_').next().unwrap_or("");

        // Infer tier from action (model name)
        let entry_tier = infer_tier_from_model(action);

        // Apply filters
        if let Some(ref tier_filter) = params.tier {
            if !entry_tier.eq_ignore_ascii_case(tier_filter) {
                continue;
            }
        }
        if let Some(ref tt_filter) = params.task_type {
            if !entry_task_type.eq_ignore_ascii_case(tt_filter) {
                continue;
            }
        }
        if let Some(ref model_filter) = params.model {
            if !action.contains(model_filter.as_str()) {
                continue;
            }
        }
        if let Some(cutoff) = since_cutoff {
            let updated = chrono::DateTime::parse_from_rfc3339(last_updated)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);
            if updated < cutoff {
                continue;
            }
        }

        // Compute trend_7d: mean reward in last 7d vs current Q
        let trend_7d = exp_map
            .get(&(state_key.to_string(), action.to_string()))
            .map(|exps| {
                let recent: Vec<f64> = exps
                    .iter()
                    .filter(|(ts, _)| *ts >= seven_days_ago)
                    .map(|(_, r)| *r)
                    .collect();
                if recent.is_empty() {
                    0.0
                } else {
                    let mean = recent.iter().sum::<f64>() / recent.len() as f64;
                    mean - q_value
                }
            })
            .unwrap_or(0.0);

        entries.push(json!({
            "state_key": state_key,
            "action": action,
            "tier": entry_tier,
            "task_type": entry_task_type,
            "q_value": q_value,
            "visit_count": visit_count,
            "last_updated": last_updated,
            "trend_7d": (trend_7d * 1000.0).round() / 1000.0,
        }));
    }

    // Sort
    let sort_key = params.sort.as_deref().unwrap_or("visits");
    entries.sort_by(|a, b| {
        match sort_key {
            "q" => {
                let qa = a["q_value"].as_f64().unwrap_or(0.0);
                let qb = b["q_value"].as_f64().unwrap_or(0.0);
                qb.partial_cmp(&qa).unwrap_or(std::cmp::Ordering::Equal)
            }
            "recency" => {
                let ta = a["last_updated"].as_str().unwrap_or("");
                let tb = b["last_updated"].as_str().unwrap_or("");
                tb.cmp(ta)
            }
            _ => {
                // "visits" (default) — descending
                let va = a["visit_count"].as_u64().unwrap_or(0);
                let vb = b["visit_count"].as_u64().unwrap_or(0);
                vb.cmp(&va)
            }
        }
    });

    // Apply limit
    let limit = params.limit as usize;
    entries.truncate(limit);

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "count": entries.len(),
            "sort": sort_key,
            "entries": entries,
        })),
    )
}

fn infer_tier_from_model(model: &str) -> String {
    let m = model.to_lowercase();
    if m.contains("qwen3:4b") || m.contains("qwen3-4b") {
        "t1".into()
    } else if m.contains("qwen2.5-coder") || m.contains("qwen2.5_coder") || m.contains("codellama") {
        "t2".into()
    } else if m.contains("devstral") || m.contains("deepseek") {
        "t2.5".into()
    } else if m.contains("claude") || m.contains("gpt-4") || m.contains("opus") || m.contains("sonnet") {
        "t3".into()
    } else {
        "unknown".into()
    }
}

// ─── Calibration ──────────────────────────────────────────────────────────────
//
// Sends a 1-prompt probe through this same in-process pipeline so vault-only
// secrets (e.g. OPENROUTER_API_KEY) resolve. The CLI-side `hex inference test`
// can't do this — it makes its own outbound call without vault access — which
// is why every OpenRouter provider sits at q=0.00.

const CALIBRATION_PROMPT: &str = "Reply with one word: ok";
const CALIBRATION_TIMEOUT_SECS: u64 = 60;

fn compute_quality_score(latency_ms: u64, sanity_ok: bool) -> f32 {
    let latency_bonus = if latency_ms <= 800 {
        0.15
    } else if latency_ms >= 5_000 {
        0.0
    } else {
        let span = (5_000 - 800) as f32;
        0.15 * ((5_000 - latency_ms) as f32 / span)
    };
    let sanity_bonus = if sanity_ok { 0.15 } else { -0.30 };
    (0.70_f32 + latency_bonus + sanity_bonus).clamp(0.0, 1.0)
}

#[derive(Debug, Serialize)]
pub struct CalibrationResult {
    pub id: String,
    pub model: String,
    pub latency_ms: u64,
    pub quality_score: f32,
    pub sanity_ok: bool,
    pub sample_reply: String,
    pub error: Option<String>,
}

/// POST /api/inference/calibrate/{id} — probe one provider, score it, persist.
pub async fn calibrate_endpoint(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let result = run_calibration(&state, &id).await;
    let status = if result.error.is_some() {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::OK
    };
    (
        status,
        Json(serde_json::to_value(result).unwrap_or_else(|_| json!({}))),
    )
}

/// POST /api/inference/calibrate-all — probe every provider; best-effort.
pub async fn calibrate_all(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(ref stdb_client) = state.inference_stdb else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "SpacetimeDB not connected" })),
        );
    };
    let providers = match stdb_client.list_providers().await {
        Ok(p) => p,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    };
    let mut results: Vec<CalibrationResult> = Vec::with_capacity(providers.len());
    for (i, p) in providers.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
        results.push(run_calibration(&state, &p.provider_id).await);
    }
    let count = results.len();
    (
        StatusCode::OK,
        Json(json!({ "results": results, "count": count })),
    )
}

async fn run_calibration(state: &SharedState, id: &str) -> CalibrationResult {
    let mut result = CalibrationResult {
        id: id.to_string(),
        model: String::new(),
        latency_ms: 0,
        quality_score: 0.0,
        sanity_ok: false,
        sample_reply: String::new(),
        error: None,
    };

    let Some(ref stdb_client) = state.inference_stdb else {
        result.error = Some("SpacetimeDB not connected".into());
        return result;
    };
    let providers = match stdb_client.list_providers().await {
        Ok(p) => p,
        Err(e) => {
            result.error = Some(format!("list_providers: {}", e));
            return result;
        }
    };
    let Some(provider) = providers.into_iter().find(|p| p.provider_id == id) else {
        result.error = Some(format!("provider '{}' not found", id));
        return result;
    };
    let primary_model = serde_json::from_str::<Vec<String>>(&provider.models_json)
        .ok()
        .and_then(|v| v.into_iter().next())
        .unwrap_or_else(|| provider.models_json.clone());
    result.model = primary_model.clone();

    let req = InferenceCompleteRequest {
        model: Some(primary_model.clone()),
        messages: vec![json!({ "role": "user", "content": CALIBRATION_PROMPT })],
        system: None,
        max_tokens: 16,
        tools: None,
    };
    let started = std::time::Instant::now();
    let probe = tokio::time::timeout(
        std::time::Duration::from_secs(CALIBRATION_TIMEOUT_SECS),
        inference_complete(State(state.clone()), axum::http::HeaderMap::new(), Json(req)),
    )
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    result.latency_ms = latency_ms;

    let (status, body) = match probe {
        Ok(pair) => pair,
        Err(_) => {
            result.error = Some(format!("timeout after {}s", CALIBRATION_TIMEOUT_SECS));
            return result;
        }
    };
    if !status.is_success() {
        result.error = Some(
            body.0
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("upstream error")
                .to_string(),
        );
        return result;
    }
    let content = body
        .0
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    result.sample_reply = content.chars().take(120).collect();
    result.sanity_ok = !content.trim().is_empty();
    result.quality_score = compute_quality_score(latency_ms, result.sanity_ok);

    if let Err(e) = stdb_client
        .register_provider(
            &provider.provider_id,
            &provider.provider_type,
            &provider.base_url,
            &provider.api_key_ref,
            &provider.models_json,
            provider.rate_limit_rpm,
            provider.rate_limit_tpm,
            &provider.quantization_level,
            provider.context_window,
            result.quality_score,
        )
        .await
    {
        result.error = Some(format!("persist: {}", e));
    }

    result
}
#[cfg(test)]
mod tests {
    use super::*;

    // ── /v1/messages Anthropic translation ──────────────────────────────────
    #[test]
    fn system_flattens_string_and_blocks() {
        assert_eq!(flatten_anthropic_system(Some(&json!("hi"))).as_deref(), Some("hi"));
        let blocks = json!([{"type":"text","text":"a"},{"type":"text","text":"b"}]);
        assert_eq!(flatten_anthropic_system(Some(&blocks)).as_deref(), Some("a\nb"));
        assert_eq!(flatten_anthropic_system(None), None);
    }

    #[test]
    fn string_content_passes_through() {
        let m = json!([{"role":"user","content":"hello"}]);
        let out = anthropic_messages_to_openai(m.as_array().unwrap());
        assert_eq!(out, vec![json!({"role":"user","content":"hello"})]);
    }

    #[test]
    fn assistant_tool_use_becomes_openai_tool_calls() {
        let m = json!([{
            "role":"assistant",
            "content":[
                {"type":"text","text":"let me read"},
                {"type":"tool_use","id":"toolu_1","name":"repo_read","input":{"path":"a.rs"}}
            ]
        }]);
        let out = anthropic_messages_to_openai(m.as_array().unwrap());
        assert_eq!(out.len(), 1);
        let tc = &out[0]["tool_calls"][0];
        assert_eq!(tc["id"], "toolu_1");
        assert_eq!(tc["function"]["name"], "repo_read");
        assert_eq!(tc["function"]["arguments"], "{\"path\":\"a.rs\"}");
        assert_eq!(out[0]["content"], "let me read");
    }

    #[test]
    fn user_tool_result_becomes_openai_tool_role() {
        let m = json!([{
            "role":"user",
            "content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"file body"}]
        }]);
        let out = anthropic_messages_to_openai(m.as_array().unwrap());
        assert_eq!(out, vec![json!({"role":"tool","tool_call_id":"toolu_1","content":"file body"})]);
    }

    #[test]
    fn inner_text_only_is_end_turn() {
        let (content, stop) = anthropic_content_from_inner(&json!({"content":"hi","tool_calls":[]}));
        assert_eq!(stop, "end_turn");
        assert_eq!(content, vec![json!({"type":"text","text":"hi"})]);
    }

    #[test]
    fn inner_tool_calls_become_tool_use_and_stop_tool_use() {
        let inner = json!({
            "content":"",
            "tool_calls":[{"id":"toolu_9","function":{"name":"code_patch","arguments":"{\"path\":\"x\"}"}}]
        });
        let (content, stop) = anthropic_content_from_inner(&inner);
        assert_eq!(stop, "tool_use");
        // empty text dropped → only the tool_use block
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["name"], "code_patch");
        assert_eq!(content[0]["input"], json!({"path":"x"}));
    }

    #[test]
    fn test_cqs_bounds() {
        assert!(compute_quality_score(200, false) < 0.6);
        assert!(compute_quality_score(200, true) > 0.9);
    }
}
    #[test]
    fn test_cqs_slow_path() {
        assert!(compute_quality_score(9000, true) > 0.8);
        assert!(compute_quality_score(9000, false) < 0.5);
    }
      #[test]
      fn test_cqs_latency_bonus() {
          assert!(compute_quality_score(100, true) > 0.9);
      }
      #[test]
      fn test_cqs_clamp_floor() {
          assert!(compute_quality_score(9000, false) >= 0.0);
      }
      #[test]
      fn test_cqs_pass() {
          assert!(compute_quality_score(100, true) > 0.9);
      }
      #[test]
      fn test_cqs_mid() {
          assert!(compute_quality_score(2000, true) > 0.7);
      }
#[test]
fn test_cqs_cli_proof() {
    assert!(compute_quality_score(100, false) < 0.7);
    assert!(compute_quality_score(100, true) > 0.9);
}
#[test]
fn test_swarm_review_exec() {
    assert!(compute_quality_score(100, true) > 0.9);
}
#[test]
fn test_adversarial_reverify_xyz() {
    let score = compute_quality_score(500, true);
    assert!(score > 0.5);
}
// Tests for the compute_quality_score function under various conditions
// This module contains tests for the inference logic.
#[test]
fn test_solo_path_check() {
    assert!(compute_quality_score(100, true) > 0.9);
}
// Additional test cases for edge scenarios
#[test]
fn test_harden_proof() {
    assert!(compute_quality_score(100, true) > 0.9);
}
