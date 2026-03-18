//! Secret Grant SpacetimeDB Module (ADR-026)
//!
//! Stores secret **grant metadata** (key names, agent scoping, TTLs) and
//! inference endpoint discovery data. No secret values are ever stored.
//!
//! Tables:
//!   - `secret_grant` (private) — which agent is granted which secret key
//!   - `inference_endpoint` (public) — discoverable local/remote LLM endpoints

use spacetimedb::{table, reducer, ReducerContext, Table};

// ─── Secret Grant (PRIVATE — only reducers can read) ────────────────────────

#[table(name = secret_grant, private)]
#[derive(Clone, Debug)]
pub struct SecretGrant {
    /// Composite key: "{agent_id}:{secret_key}"
    #[unique]
    pub id: String,
    /// Agent that is granted access to this secret
    pub agent_id: String,
    /// Secret key name (e.g. "ANTHROPIC_API_KEY") — NOT the value
    pub secret_key: String,
    /// Purpose tag: "llm", "webhook", "auth", etc.
    pub purpose: String,
    /// ISO 8601 timestamp when the grant was created
    pub granted_at: String,
    /// ISO 8601 timestamp when the grant expires
    pub expires_at: String,
    /// Whether the agent has claimed (consumed) this grant
    pub claimed: bool,
}

/// Create a secret grant for an agent. Called by hex-hub before spawning.
#[reducer]
pub fn grant_secret(
    ctx: &ReducerContext,
    agent_id: String,
    secret_key: String,
    purpose: String,
    granted_at: String,
    expires_at: String,
) -> Result<(), String> {
    let id = format!("{}:{}", agent_id, secret_key);

    // Upsert: if grant already exists, update it
    if let Some(existing) = ctx.db.secret_grant().id().find(&id) {
        ctx.db.secret_grant().id().update(SecretGrant {
            granted_at: granted_at.clone(),
            expires_at,
            claimed: false,
            purpose,
            ..existing
        });
    } else {
        ctx.db.secret_grant().insert(SecretGrant {
            id,
            agent_id,
            secret_key,
            purpose,
            granted_at,
            expires_at,
            claimed: false,
        });
    }

    Ok(())
}

/// Mark a grant as claimed. Called by hex-hub after injecting the secret.
/// Returns error if grant doesn't exist or was already claimed.
#[reducer]
pub fn claim_grant(
    ctx: &ReducerContext,
    agent_id: String,
    secret_key: String,
) -> Result<(), String> {
    let id = format!("{}:{}", agent_id, secret_key);

    match ctx.db.secret_grant().id().find(&id) {
        Some(existing) => {
            if existing.claimed {
                return Err(format!(
                    "Grant '{}' for agent '{}' already claimed",
                    secret_key, agent_id
                ));
            }
            ctx.db.secret_grant().id().update(SecretGrant {
                claimed: true,
                ..existing
            });
            Ok(())
        }
        None => Err(format!(
            "No grant for key '{}' found for agent '{}'",
            secret_key, agent_id
        )),
    }
}

/// Revoke a specific grant. Called on agent termination or manual revocation.
#[reducer]
pub fn revoke_secret(
    ctx: &ReducerContext,
    agent_id: String,
    secret_key: String,
) -> Result<(), String> {
    let id = format!("{}:{}", agent_id, secret_key);
    let deleted = ctx.db.secret_grant().id().delete(&id);
    if !deleted {
        return Err(format!(
            "No grant for key '{}' found for agent '{}'",
            secret_key, agent_id
        ));
    }
    Ok(())
}

/// Revoke ALL grants for an agent. Called when an agent is terminated.
#[reducer]
pub fn revoke_all_for_agent(
    ctx: &ReducerContext,
    agent_id: String,
) -> Result<(), String> {
    let grants: Vec<SecretGrant> = ctx
        .db
        .secret_grant()
        .iter()
        .filter(|g| g.agent_id == agent_id)
        .collect();

    for grant in grants {
        ctx.db.secret_grant().id().delete(&grant.id);
    }

    Ok(())
}

/// Prune all expired grants. Called periodically by hex-hub.
/// `now` is an ISO 8601 timestamp representing the current time.
#[reducer]
pub fn prune_expired(
    ctx: &ReducerContext,
    now: String,
) -> Result<(), String> {
    let expired: Vec<SecretGrant> = ctx
        .db
        .secret_grant()
        .iter()
        .filter(|g| g.expires_at <= now)
        .collect();

    let count = expired.len();
    for grant in expired {
        ctx.db.secret_grant().id().delete(&grant.id);
    }

    if count > 0 {
        log::info!("Pruned {} expired secret grants", count);
    }

    Ok(())
}

// ─── Inference Endpoint (PUBLIC — all agents can subscribe) ─────────────────

#[table(name = inference_endpoint, public)]
#[derive(Clone, Debug)]
pub struct InferenceEndpoint {
    /// Unique endpoint identifier
    #[unique]
    pub id: String,
    /// Full URL (e.g. "http://127.0.0.1:11434")
    pub url: String,
    /// Provider type: "ollama", "openai-compatible", "vllm", "llama-cpp"
    pub provider: String,
    /// Model identifier (e.g. "llama3.1:70b", "mistral:latest")
    pub model: String,
    /// Current status: "healthy", "unhealthy", "unknown"
    pub status: String,
    /// Whether this endpoint requires authentication
    pub requires_auth: bool,
    /// Secret key name in ISecretsPort (empty string if no auth required)
    pub secret_key: String,
    /// ISO 8601 timestamp of last health check
    pub health_checked_at: String,
}

/// Register a new inference endpoint. Called via CLI or hex-hub auto-discovery.
#[reducer]
pub fn register_endpoint(
    ctx: &ReducerContext,
    id: String,
    url: String,
    provider: String,
    model: String,
    requires_auth: bool,
    secret_key: String,
) -> Result<(), String> {
    // Validate provider
    match provider.as_str() {
        "ollama" | "openai-compatible" | "vllm" | "llama-cpp" => {}
        _ => {
            return Err(format!(
                "Unknown provider '{}'. Expected: ollama, openai-compatible, vllm, llama-cpp",
                provider
            ));
        }
    }

    if requires_auth && secret_key.is_empty() {
        return Err("Endpoint requires auth but no secret_key provided".to_string());
    }

    // Upsert
    if let Some(existing) = ctx.db.inference_endpoint().id().find(&id) {
        ctx.db.inference_endpoint().id().update(InferenceEndpoint {
            url,
            provider,
            model,
            requires_auth,
            secret_key,
            ..existing
        });
    } else {
        ctx.db.inference_endpoint().insert(InferenceEndpoint {
            id,
            url,
            provider,
            model,
            status: "unknown".to_string(),
            requires_auth,
            secret_key,
            health_checked_at: String::new(),
        });
    }

    Ok(())
}

/// Update the health status of an endpoint. Called by hex-hub health checker.
#[reducer]
pub fn update_health(
    ctx: &ReducerContext,
    id: String,
    status: String,
    checked_at: String,
) -> Result<(), String> {
    match status.as_str() {
        "healthy" | "unhealthy" | "unknown" => {}
        _ => {
            return Err(format!(
                "Invalid status '{}'. Expected: healthy, unhealthy, unknown",
                status
            ));
        }
    }

    match ctx.db.inference_endpoint().id().find(&id) {
        Some(existing) => {
            ctx.db.inference_endpoint().id().update(InferenceEndpoint {
                status,
                health_checked_at: checked_at,
                ..existing
            });
            Ok(())
        }
        None => Err(format!("Endpoint '{}' not found", id)),
    }
}

/// Remove an inference endpoint.
#[reducer]
pub fn remove_endpoint(
    ctx: &ReducerContext,
    id: String,
) -> Result<(), String> {
    let deleted = ctx.db.inference_endpoint().id().delete(&id);
    if !deleted {
        return Err(format!("Endpoint '{}' not found", id));
    }
    Ok(())
}
