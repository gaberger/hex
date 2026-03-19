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
    pub requires_auth: Option<bool>,
    pub secret_key: Option<String>,
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
        "ollama" | "openai-compatible" | "vllm" | "llama-cpp" => {}
        _ => return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Unknown provider '{}'", body.provider) }))),
    }
    let endpoint = InferenceEndpointEntry {
        id: body.id.clone(), url: body.url, provider: body.provider, model: body.model,
        status: "unknown".to_string(), requires_auth: body.requires_auth.unwrap_or(false),
        secret_key: body.secret_key.unwrap_or_default(), health_checked_at: String::new(),
    };
    state.inference_endpoints.write().await.insert(body.id.clone(), endpoint);
    (StatusCode::CREATED, Json(json!({ "id": body.id })))
}

pub async fn list_inference(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let endpoints = state.inference_endpoints.read().await;
    let list: Vec<serde_json::Value> = endpoints.values().map(|e| {
        json!({ "id": e.id, "url": e.url, "provider": e.provider, "model": e.model,
                "status": e.status, "requiresAuth": e.requires_auth, "healthCheckedAt": e.health_checked_at })
    }).collect();
    (StatusCode::OK, Json(json!({ "endpoints": list })))
}

pub async fn remove_inference(
    State(state): State<SharedState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.inference_endpoints.write().await.remove(&id) {
        Some(_) => (StatusCode::OK, Json(json!({ "removed": id }))),
        None => (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Endpoint '{}' not found", id) }))),
    }
}

pub async fn check_inference_health(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(5)).build().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let checks: Vec<(String, String, String)> = {
        let eps = state.inference_endpoints.read().await;
        eps.values().map(|e| (e.id.clone(), e.url.clone(), e.provider.clone())).collect()
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
        let mut eps = state.inference_endpoints.write().await;
        if let Some(ep) = eps.get_mut(&id) {
            ep.status = status.to_string();
            ep.health_checked_at = now.clone();
        }
    }
    (StatusCode::OK, Json(json!({ "results": results })))
}
