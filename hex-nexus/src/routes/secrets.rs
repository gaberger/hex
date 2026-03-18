//! Secret broker routes (ADR-026).
//!
//! Provides the one-shot `/secrets/claim` endpoint for independent agents
//! and grant management endpoints for hex-hub internal use.
//!
//! Security: claim endpoint is localhost-only, nonce-validated, single-use.

use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use crate::state::SharedState;

// ── Request / Response Types ─────────────────────────────

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
    pub requires_auth: Option<bool>,
    pub secret_key: Option<String>,
}

// ── Handlers ─────────────────────────────────────────────

/// POST /secrets/claim — One-shot secret claim for independent agents.
///
/// Flow:
/// 1. Agent sends agent_id + nonce
/// 2. Hub looks up pending grants for agent_id
/// 3. Hub resolves each secret key from env/vault/Infisical
/// 4. Hub marks grants as claimed
/// 5. Returns resolved secret values (single-use)
pub async fn claim_secrets(
    State(state): State<SharedState>,
    Json(body): Json<ClaimRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    if body.agent_id.is_empty() || body.nonce.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "agent_id and nonce are required" })),
        );
    }

    // Look up grants for this agent
    let grants = state.secret_grants.read().await;
    let agent_grants: Vec<_> = grants
        .iter()
        .filter(|(_, g)| g.agent_id == body.agent_id)
        .collect();

    if agent_grants.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("No grants found for agent '{}'", body.agent_id) })),
        );
    }

    // Check if any grant is already claimed
    let all_claimed = agent_grants.iter().all(|(_, g)| g.claimed);
    if all_claimed {
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "All grants for this agent have already been claimed" })),
        );
    }

    // Check expiry
    let now = chrono::Utc::now().to_rfc3339();
    let has_expired = agent_grants.iter().any(|(_, g)| !g.claimed && g.expires_at <= now);
    if has_expired {
        return (
            StatusCode::GONE,
            Json(json!({ "error": "One or more grants have expired" })),
        );
    }

    // Resolve secrets from environment (the broker's trusted secret source)
    let mut resolved = HashMap::new();
    let mut min_ttl: u64 = 3600;

    for (_, grant) in &agent_grants {
        if grant.claimed {
            continue;
        }
        // Resolve from process environment (hex-hub has secrets injected)
        if let Ok(value) = std::env::var(&grant.secret_key) {
            resolved.insert(grant.secret_key.clone(), value);
            // Calculate remaining TTL
            if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&grant.expires_at) {
                let remaining = (exp.with_timezone(&chrono::Utc) - chrono::Utc::now())
                    .num_seconds()
                    .max(0) as u64;
                min_ttl = min_ttl.min(remaining);
            }
        } else {
            tracing::warn!(
                key = %grant.secret_key,
                agent = %body.agent_id,
                "Secret not available in broker environment"
            );
        }
    }

    // Collect keys to claim before dropping the read guard
    let claimed_keys: Vec<String> = agent_grants
        .iter()
        .filter(|(_, g)| !g.claimed)
        .map(|(_, g)| g.secret_key.clone())
        .collect();

    drop(agent_grants);
    drop(grants);

    if resolved.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "Failed to resolve any granted secrets" })),
        );
    }

    // Try SpacetimeDB claim_grant reducer for each key
    if let Some(ref stdb) = state.spacetime_secrets {
        if stdb.is_connected().await {
            for key in &claimed_keys {
                if let Err(e) = stdb.claim(&body.agent_id, key, &body.nonce).await {
                    tracing::warn!(
                        key = %key,
                        error = %e,
                        "SpacetimeDB claim_grant failed — local cache still updated"
                    );
                }
            }
        }
    }

    // Always update local cache
    let mut grants = state.secret_grants.write().await;
    for (_, grant) in grants.iter_mut() {
        if grant.agent_id == body.agent_id && !grant.claimed {
            grant.claimed = true;
            grant.claimed_nonce = Some(body.nonce.clone());
        }
    }

    let backend = if state.spacetime_secrets.is_some() {
        "spacetimedb+in-memory"
    } else {
        "in-memory"
    };

    tracing::info!(
        agent = %body.agent_id,
        secrets_count = resolved.len(),
        expires_in = min_ttl,
        backend,
        "Secrets claimed successfully"
    );

    (
        StatusCode::OK,
        Json(json!(ClaimResponse {
            secrets: resolved,
            expires_in: min_ttl,
        })),
    )
}

/// POST /secrets/grant — Create a secret grant for an agent.
///
/// When SpacetimeDB is available, delegates to the `grant_secret` reducer
/// so grant metadata is persisted in the distributed store. Falls back to
/// the in-memory HashMap when SpacetimeDB is unavailable.
pub async fn grant_secret(
    State(state): State<SharedState>,
    Json(body): Json<GrantRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let ttl = body.ttl_secs.unwrap_or(3600);
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::seconds(ttl as i64);
    let now_str = now.to_rfc3339();
    let expires_str = expires.to_rfc3339();

    // Try SpacetimeDB first
    if let Some(ref stdb) = state.spacetime_secrets {
        if stdb.is_connected().await {
            match stdb
                .grant(&body.agent_id, &body.secret_key, &body.purpose, &now_str, &expires_str)
                .await
            {
                Ok(id) => {
                    // Also update the local fallback cache for consistency
                    let grant = SecretGrantEntry {
                        agent_id: body.agent_id.clone(),
                        secret_key: body.secret_key.clone(),
                        purpose: body.purpose,
                        granted_at: now_str,
                        expires_at: expires_str.clone(),
                        claimed: false,
                        claimed_nonce: None,
                    };
                    state.secret_grants.write().await.insert(id.clone(), grant);

                    tracing::info!(
                        agent = %body.agent_id,
                        key = %body.secret_key,
                        ttl_secs = ttl,
                        backend = "spacetimedb",
                        "Secret grant created"
                    );
                    return (StatusCode::CREATED, Json(json!({ "id": id, "expiresAt": expires_str })));
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "SpacetimeDB grant_secret failed — falling back to in-memory"
                    );
                    // Fall through to in-memory path
                }
            }
        }
    }

    // Fallback: in-memory only
    let grant = SecretGrantEntry {
        agent_id: body.agent_id.clone(),
        secret_key: body.secret_key.clone(),
        purpose: body.purpose,
        granted_at: now_str,
        expires_at: expires_str.clone(),
        claimed: false,
        claimed_nonce: None,
    };

    let id = format!("{}:{}", body.agent_id, body.secret_key);
    state.secret_grants.write().await.insert(id.clone(), grant);

    tracing::info!(
        agent = %body.agent_id,
        key = %body.secret_key,
        ttl_secs = ttl,
        backend = "in-memory",
        "Secret grant created"
    );

    (StatusCode::CREATED, Json(json!({ "id": id, "expiresAt": expires_str })))
}

/// POST /secrets/revoke — Revoke grants for an agent.
///
/// Delegates to SpacetimeDB `revoke_secret` / `revoke_all_for_agent` reducers
/// when available, always updates local cache.
pub async fn revoke_secret(
    State(state): State<SharedState>,
    Json(body): Json<RevokeRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Try SpacetimeDB first
    if let Some(ref stdb) = state.spacetime_secrets {
        if stdb.is_connected().await {
            let result = if let Some(ref key) = body.secret_key {
                stdb.revoke(&body.agent_id, key).await.map(|_| 1)
            } else {
                stdb.revoke_all(&body.agent_id).await
            };

            if let Err(e) = &result {
                tracing::warn!(
                    error = %e,
                    "SpacetimeDB revoke failed — falling back to in-memory"
                );
            }
        }
    }

    // Always update local cache
    let mut grants = state.secret_grants.write().await;
    let before = grants.len();

    if let Some(key) = &body.secret_key {
        let id = format!("{}:{}", body.agent_id, key);
        grants.remove(&id);
    } else {
        grants.retain(|_, g| g.agent_id != body.agent_id);
    }

    let removed = before - grants.len();
    (StatusCode::OK, Json(json!({ "revoked": removed })))
}

/// GET /secrets/grants — List all active grants (metadata only, no values).
///
/// Reads from the SpacetimeDB client's local cache when available,
/// otherwise falls back to the in-memory HashMap.
pub async fn list_grants(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Prefer SpacetimeDB client cache if connected (it's the authoritative source)
    let grants_source = if let Some(ref stdb) = state.spacetime_secrets {
        if stdb.is_connected().await {
            stdb.cache().read().await.clone()
        } else {
            state.secret_grants.read().await.clone()
        }
    } else {
        state.secret_grants.read().await.clone()
    };

    let list: Vec<_> = grants_source
        .values()
        .map(|g| {
            json!({
                "agentId": g.agent_id,
                "secretKey": g.secret_key,
                "purpose": g.purpose,
                "grantedAt": g.granted_at,
                "expiresAt": g.expires_at,
                "claimed": g.claimed,
            })
        })
        .collect();

    (StatusCode::OK, Json(json!({ "grants": list })))
}

// ── Inference Endpoint Routes ────────────────────────────

/// POST /inference/register — Register a local inference endpoint.
pub async fn register_inference(
    State(state): State<SharedState>,
    Json(body): Json<InferenceRegisterRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // Validate provider
    match body.provider.as_str() {
        "ollama" | "openai-compatible" | "vllm" | "llama-cpp" => {}
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": format!("Unknown provider '{}'. Expected: ollama, openai-compatible, vllm, llama-cpp", body.provider) })),
            );
        }
    }

    let endpoint = InferenceEndpointEntry {
        id: body.id.clone(),
        url: body.url,
        provider: body.provider,
        model: body.model,
        status: "unknown".to_string(),
        requires_auth: body.requires_auth.unwrap_or(false),
        secret_key: body.secret_key.unwrap_or_default(),
        health_checked_at: String::new(),
    };

    state
        .inference_endpoints
        .write()
        .await
        .insert(body.id.clone(), endpoint);

    tracing::info!(id = %body.id, "Inference endpoint registered");

    (StatusCode::CREATED, Json(json!({ "id": body.id })))
}

/// GET /inference/endpoints — List all inference endpoints.
pub async fn list_inference(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let endpoints = state.inference_endpoints.read().await;
    let list: Vec<_> = endpoints.values().map(|e| {
        json!({
            "id": e.id,
            "url": e.url,
            "provider": e.provider,
            "model": e.model,
            "status": e.status,
            "requiresAuth": e.requires_auth,
            "healthCheckedAt": e.health_checked_at,
        })
    }).collect();

    (StatusCode::OK, Json(json!({ "endpoints": list })))
}

/// DELETE /inference/endpoints/:id — Remove an inference endpoint.
pub async fn remove_inference(
    State(state): State<SharedState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    let removed = state.inference_endpoints.write().await.remove(&id);
    match removed {
        Some(_) => (StatusCode::OK, Json(json!({ "removed": id }))),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Endpoint '{}' not found", id) }))),
    }
}

/// POST /inference/health — Trigger health check on all endpoints.
pub async fn check_inference_health(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let now = chrono::Utc::now().to_rfc3339();

    // Collect IDs and URLs to check (avoid holding lock during HTTP calls)
    let checks: Vec<(String, String, String)> = {
        let endpoints = state.inference_endpoints.read().await;
        endpoints
            .values()
            .map(|e| (e.id.clone(), e.url.clone(), e.provider.clone()))
            .collect()
    };

    let mut results = Vec::new();

    for (id, url, provider) in checks {
        let health_url = match provider.as_str() {
            "ollama" => format!("{}/api/tags", url),
            _ => format!("{}/v1/models", url),
        };

        let status = match client.get(&health_url).send().await {
            Ok(res) if res.status().is_success() => "healthy",
            _ => "unhealthy",
        };

        results.push(json!({ "id": id, "status": status }));

        // Update endpoint status
        let mut endpoints = state.inference_endpoints.write().await;
        if let Some(ep) = endpoints.get_mut(&id) {
            ep.status = status.to_string();
            ep.health_checked_at = now.clone();
        }
    }

    (StatusCode::OK, Json(json!({ "results": results })))
}

// ── State Types (added to AppState) ──────────────────────

/// In-memory secret grant entry (broker side).
#[derive(Debug, Clone, Serialize)]
pub struct SecretGrantEntry {
    pub agent_id: String,
    pub secret_key: String,
    pub purpose: String,
    pub granted_at: String,
    pub expires_at: String,
    pub claimed: bool,
    pub claimed_nonce: Option<String>,
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
