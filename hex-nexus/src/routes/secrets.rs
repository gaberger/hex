//! Secret broker routes (ADR-026).
//!
//! All operations go through SpacetimeDB via ISecretGrantPort.
//! No in-memory fallback — returns 503 when SpacetimeDB is unavailable.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use crate::ports::secret_grant::ISecretGrantPort;
use crate::state::SharedState;

fn no_backend() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({
        "error": "SpacetimeDB not configured — secrets require distributed storage",
        "hint": "Set HEX_SPACETIMEDB_URL"
    })))
}

// ── Request/Response Types ───────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSetRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimRequest {
    pub agent_id: String,
    pub nonce: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimResponse {
    pub secrets: HashMap<String, String>,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantRequest {
    pub agent_id: String,
    pub secret_key: String,
    pub purpose: String,
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokeRequest {
    pub agent_id: String,
    pub secret_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRegisterRequest {
    pub id: String,
    pub url: String,
    pub provider: String,
    pub model: String,
    #[serde(alias = "models_json")]
    pub models_json: Option<String>,
    pub requires_auth: Option<bool>,
    #[serde(alias = "secret_key")]
    pub secret_key: Option<String>,
    /// Quantization level (e.g. "q2", "q4", "fp16", "cloud").
    /// Auto-detected from Ollama model name if omitted. Defaults to "cloud" for API providers.
    pub quantization: Option<String>,
    /// Context window size in tokens.
    #[serde(alias = "context_window")]
    pub context_window: Option<u32>,
    /// Requests per minute limit (ADR-2604052125).
    #[serde(alias = "rate_limit_rpm")]
    pub rate_limit_rpm: Option<u32>,
    /// Tokens per minute limit (ADR-2604052125).
    #[serde(alias = "rate_limit_tpm")]
    pub rate_limit_tpm: Option<u64>,
    /// Whether this is a free-tier provider (ADR-2604052125).
    #[serde(alias = "is_free_tier")]
    pub is_free_tier: Option<bool>,
    /// Cost per million input tokens (ADR-2604052125).
    #[serde(alias = "cost_per_input_mtok")]
    pub cost_per_input_mtok: Option<f64>,
    /// Cost per million output tokens (ADR-2604052125).
    #[serde(alias = "cost_per_output_mtok")]
    pub cost_per_output_mtok: Option<f64>,
    /// Daily token limit (0 = unlimited) (ADR-2604052125).
    #[serde(alias = "daily_token_limit")]
    pub daily_token_limit: Option<u64>,
    /// Daily request limit (0 = unlimited) (ADR-2604052125).
    #[serde(alias = "daily_request_limit")]
    pub daily_request_limit: Option<u32>,
}

/// In-memory inference endpoint entry.
#[derive(Debug, Clone, Serialize)]
pub struct InferenceEndpointEntry {
    pub id: String,
    pub url: String,
    pub provider: String,
    pub model: String,
    pub status: String,
    pub requires_auth: bool,
    pub secret_key: String,
    pub health_checked_at: String,
}

// ── Vault Handlers ───────────────────────────────────────

pub async fn vault_set(
    State(state): State<SharedState>,
    Json(body): Json<VaultSetRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if body.key.is_empty() || body.value.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "key and value are required" })));
    }
    let stdb = match &state.spacetime_secrets {
        Some(s) => s,
        None => return no_backend(),
    };
    if !stdb.is_healthy().await {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB unreachable" })));
    }
    match stdb.vault_store(&body.key, &body.value).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "stored": body.key, "backend": "spacetimedb" }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Failed: {}", e) }))),
    }
}

pub async fn vault_list(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let stdb = match &state.spacetime_secrets {
        Some(s) => s,
        None => return no_backend(),
    };
    match stdb.vault_list().await {
        Ok(map) => {
            let keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
            (StatusCode::OK, Json(json!({ "keys": keys, "count": keys.len() })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

pub async fn vault_get(
    State(state): State<SharedState>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let stdb = match &state.spacetime_secrets {
        Some(s) => s,
        None => return no_backend(),
    };
    match stdb.vault_get(&key).await {
        Ok(Some(value)) => (StatusCode::OK, Json(json!({ "key": key, "value": value }))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Secret '{}' not found", key) }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

// ── Grant Handlers ───────────────────────────────────────

/// POST /secrets/claim — Resolve secrets for an agent from SpacetimeDB vault.
///
/// SpacetimeDB is the sole secret store. No env var fallback.
/// Any agent connected to the hub can claim all secrets in the vault.
pub async fn claim_secrets(
    State(state): State<SharedState>,
    Json(body): Json<ClaimRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if body.agent_id.is_empty() || body.nonce.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "agent_id and nonce are required" })));
    }

    let stdb = match &state.spacetime_secrets {
        Some(s) => s,
        None => return no_backend(),
    };
    if !stdb.is_healthy().await {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB unreachable" })));
    }

    // Query ALL secrets from SpacetimeDB vault directly via SQL
    let resolved: HashMap<String, String> = match stdb.vault_list().await {
        Ok(secrets) => secrets,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to query vault");
            HashMap::new()
        }
    };

    if resolved.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "No secrets in SpacetimeDB vault. Use 'hex secrets set <key> <value>' to store secrets.",
            })),
        );
    }

    tracing::info!(agent = %body.agent_id, secrets_count = resolved.len(), "Secrets resolved from SpacetimeDB vault");

    (StatusCode::OK, Json(json!(ClaimResponse { secrets: resolved, expires_in: 3600 })))
}

pub async fn grant_secret(
    State(state): State<SharedState>,
    Json(body): Json<GrantRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let stdb = match &state.spacetime_secrets {
        Some(s) => s,
        None => return no_backend(),
    };
    if !stdb.is_healthy().await {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB unreachable" })));
    }
    let ttl = body.ttl_secs.unwrap_or(3600);
    let hub_id = std::env::var("HEX_HUB_ID").unwrap_or_else(|_| "hub-local".to_string());

    match stdb.grant(&body.agent_id, &body.secret_key, &body.purpose, &hub_id, ttl).await {
        Ok(grant) => (StatusCode::CREATED, Json(json!({ "id": grant.id, "expiresAt": grant.expires_at }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("Grant failed: {}", e) }))),
    }
}

pub async fn revoke_secret(
    State(state): State<SharedState>,
    Json(body): Json<RevokeRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let stdb = match &state.spacetime_secrets {
        Some(s) => s,
        None => return no_backend(),
    };
    if !stdb.is_healthy().await {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB unreachable" })));
    }
    if let Some(key) = &body.secret_key {
        match stdb.revoke(&body.agent_id, key).await {
            Ok(()) => (StatusCode::OK, Json(json!({ "revoked": 1 }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
        }
    } else {
        match stdb.revoke_all(&body.agent_id).await {
            Ok(count) => (StatusCode::OK, Json(json!({ "revoked": count }))),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
        }
    }
}

pub async fn list_grants(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let stdb = match &state.spacetime_secrets {
        Some(s) => s,
        None => return no_backend(),
    };
    match stdb.list_grants().await {
        Ok(grants) => {
            let list: Vec<serde_json::Value> = grants.iter().map(|g| {
                json!({
                    "agentId": g.agent_id,
                    "secretKey": g.secret_key,
                    "purpose": g.purpose,
                    "hubId": g.hub_id,
                    "grantedAt": g.granted_at,
                    "expiresAt": g.expires_at,
                    "claimed": g.claimed,
                    "claimedAt": g.claimed_at,
                    "claimHubId": g.claim_hub_id,
                })
            }).collect();
            (StatusCode::OK, Json(json!({ "grants": list })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

pub async fn secrets_health(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    match &state.spacetime_secrets {
        Some(stdb) => {
            let h = stdb.health().await;
            let status = if h.connected { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
            (status, Json(serde_json::to_value(h).unwrap()))
        }
        None => no_backend(),
    }
}

// ── Inference Endpoint Routes ────────────────────────────

pub async fn register_inference(
    State(state): State<SharedState>,
    Json(body): Json<InferenceRegisterRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    match body.provider.as_str() {
        "ollama" | "openai-compatible" | "openai_compat" | "vllm" | "llama-cpp" | "openrouter" => {}
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Unknown provider '{}'", body.provider) }))),
    }

    let Some(ref stdb_client) = state.inference_stdb else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB not connected" })));
    };

    let provider_type = match body.provider.as_str() {
        "ollama" => "ollama",
        "openai-compatible" => "openai_compat",
        "vllm" => "vllm",
        "llama-cpp" => "openai_compat",
        "openrouter" => "openrouter",
        _ => "openai_compat",
    };
    let models_json = body.models_json
        .unwrap_or_else(|| serde_json::json!([body.model]).to_string());

    // Resolve quantization level (ADR-2603271000):
    // 1. Explicit --quantization flag
    // 2. Auto-detect from model name GGUF tag
    // 3. Default: "cloud" for API providers, "q4" for local
    let quantization_level = body.quantization
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            if body.provider == "ollama" || body.provider == "vllm" {
                hex_core::QuantizationLevel::detect_from_model_name(&body.model)
                    .map(|q| q.to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| match body.provider.as_str() {
            "openrouter" => "cloud".to_string(),
            _ => "q4".to_string(),
        });

    let api_key_ref = body.secret_key.clone().unwrap_or_default();

    // Auto-fetch context window from OpenRouter model metadata if not provided.
    let context_window: u64 = if body.provider == "openrouter" && body.context_window.is_none() {
        let first_model: Option<String> = serde_json::from_str::<Vec<String>>(&models_json)
            .ok()
            .and_then(|v| v.into_iter().next());
        if let Some(ref mid) = first_model {
            fetch_openrouter_context_window(mid).await.unwrap_or(0) as u64
        } else {
            0
        }
    } else {
        body.context_window.unwrap_or(0) as u64
    };

    // Extract rate limit and cost metadata from request body (ADR-2604052125)
    let rpm_limit = body.rate_limit_rpm.unwrap_or(60);
    let tpm_limit = body.rate_limit_tpm.unwrap_or(0);
    let is_free_tier = body.is_free_tier.unwrap_or(false);

    match stdb_client.register_provider(
        &body.id, provider_type, &body.url,
        &api_key_ref,
        &models_json, rpm_limit, context_window,
        &quantization_level, 0, -1.0,
    ).await {
        Ok(()) => {
            // Push the actual resolved key to the private inference_api_key table
            // so the execute_inference procedure can use it directly.
            if !api_key_ref.is_empty() {
                if let Err(e) = stdb_client.set_api_key(&body.id, &api_key_ref).await {
                    tracing::warn!(provider = %body.id, error = %e, "set_api_key failed (non-fatal)");
                }
            }
            // Register rate limits in the rate limiter (ADR-2604052125)
            let daily_token_limit = body.daily_token_limit.unwrap_or(0);
            let daily_request_limit = body.daily_request_limit.unwrap_or(0);
            let cost_input = body.cost_per_input_mtok.unwrap_or(0.0);
            let cost_output = body.cost_per_output_mtok.unwrap_or(0.0);
            state.rate_limiter.register_provider(
                &body.id, rpm_limit, tpm_limit,
                daily_token_limit, daily_request_limit,
                is_free_tier, cost_input, cost_output,
            ).await;
            tracing::info!(
                provider = %body.id,
                rpm = rpm_limit,
                free_tier = is_free_tier,
                "provider registered with rate limiter"
            );
            (StatusCode::CREATED, Json(json!({
                "id": body.id,
                "quantization_level": quantization_level,
                "is_free_tier": is_free_tier,
            })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

// DEPRECATED(ADR-039): Browser will use SpacetimeDB direct subscription
pub async fn list_inference(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(ref client) = state.inference_stdb else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB not connected" })));
    };

    match client.list_providers().await {
        Ok(providers) => {
            let list: Vec<serde_json::Value> = providers.iter().map(|p| {
                json!({
                    "id": p.provider_id,
                    "url": p.base_url,
                    "provider": p.provider_type,
                    "model": p.models_json,
                    "status": if p.healthy == 1 { "healthy" } else { "unknown" },
                    "requiresAuth": !p.api_key_ref.is_empty(),
                    "healthCheckedAt": p.last_health_check,
                    "avgLatencyMs": p.avg_latency_ms,
                    "rateLimitRpm": p.rate_limit_rpm,
                    "quantizationLevel": p.quantization_level,
                    "contextWindow": p.context_window,
                    "qualityScore": if p.quality_score < 0.0 { serde_json::Value::Null } else { json!(p.quality_score) },
                })
            }).collect();
            (StatusCode::OK, Json(json!({ "endpoints": list, "source": "spacetimedb" })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("SpacetimeDB query failed: {}", e) }))),
    }
}

pub async fn remove_inference(
    State(state): State<SharedState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(ref stdb_client) = state.inference_stdb else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB not connected" })));
    };

    match stdb_client.remove_provider(&id).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "removed": id }))),
        Err(e) => (StatusCode::OK, Json(json!({ "removed": id, "warning": e }))),
    }
}

/// PATCH /api/inference/endpoints/:id — update quality_score for a calibrated provider.
///
/// Re-registers the provider via the SpacetimeDB upsert reducer, preserving all
/// existing fields and writing the new quality_score. Called by `hex inference test`
/// after a successful inference round-trip.
#[derive(Debug, Deserialize)]
pub struct CalibrateRequest {
    pub quality_score: f32,
    /// If provided, updates the stored context window for this endpoint.
    pub context_window: Option<u32>,
}

pub async fn calibrate_inference(
    State(state): State<SharedState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<CalibrateRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if !(0.0..=1.0).contains(&body.quality_score) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "quality_score must be 0.0–1.0" })));
    }
    let Some(ref stdb_client) = state.inference_stdb else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB not connected" })));
    };

    let providers = match stdb_client.list_providers().await {
        Ok(p) => p,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    };

    let Some(p) = providers.into_iter().find(|p| p.provider_id == id) else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("provider '{}' not found", id) })));
    };

    let new_ctx = body.context_window.unwrap_or(p.context_window);
    match stdb_client.register_provider(
        &p.provider_id,
        &p.provider_type,
        &p.base_url,
        &p.api_key_ref,
        &p.models_json,
        p.rate_limit_rpm,
        p.rate_limit_tpm,
        &p.quantization_level,
        new_ctx,
        body.quality_score,
    ).await {
        Ok(()) => (StatusCode::OK, Json(json!({
            "id": id,
            "quality_score": body.quality_score,
            "context_window": new_ctx,
        }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))),
    }
}

pub async fn check_inference_health(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build().unwrap();

    // Get providers from SpacetimeDB ONLY (ADR-041)
    let checks: Vec<(String, String, String)> = if let Some(ref stdb_client) = state.inference_stdb {
        match stdb_client.list_providers().await {
            Ok(providers) => providers.iter().map(|p| (p.provider_id.clone(), p.base_url.clone(), p.provider_type.clone())).collect(),
            Err(e) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("SpacetimeDB query failed: {}", e) })));
            }
        }
    } else {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "SpacetimeDB not connected" })));
    };

    let mut results = Vec::new();
    for (id, url, provider) in checks {
        let health_url = match provider.as_str() {
            "ollama" => format!("{}/api/tags", url),
            _ => format!("{}/v1/models", url),
        };
        let start = std::time::Instant::now();
        let status = match client.get(&health_url).send().await {
            Ok(res) if res.status().is_success() => "healthy",
            _ => "unhealthy",
        };
        let latency_ms = start.elapsed().as_millis() as u64;
        results.push(json!({ "id": id, "status": status, "latency_ms": latency_ms, "url": url }));
    }
    (StatusCode::OK, Json(json!({ "results": results })))
}

// ── Helpers ───────────────────────────────────────────────

/// Fetch the context window size for an OpenRouter model from the public model metadata API.
///
/// OpenRouter exposes `GET https://openrouter.ai/api/v1/models/{model_id}` — no auth required.
/// Returns `None` on any network or parse error (caller falls back to 0).
async fn fetch_openrouter_context_window(model_id: &str) -> Option<u32> {
    let url = format!("https://openrouter.ai/api/v1/models/{}", model_id);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let resp: serde_json::Value = client
        .get(&url)
        .header("User-Agent", "hex-nexus/1.0")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    resp["context_length"].as_u64().map(|n| n as u32)
}
