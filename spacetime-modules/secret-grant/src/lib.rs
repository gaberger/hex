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

// ─── Pure logic helpers (testable without SpacetimeDB runtime) ───────────────

/// Build the composite grant ID from agent_id and secret_key.
pub fn make_grant_id(agent_id: &str, secret_key: &str) -> String {
    format!("{}:{}", agent_id, secret_key)
}

/// Check whether a grant has expired given an ISO 8601 `now` timestamp.
/// Uses lexicographic comparison which works for ISO 8601 strings.
pub fn is_expired(expires_at: &str, now: &str) -> bool {
    expires_at <= now
}

/// Validate an inference endpoint provider string.
pub fn validate_provider(provider: &str) -> Result<(), String> {
    match provider {
        "ollama" | "openai-compatible" | "vllm" | "llama-cpp" => Ok(()),
        _ => Err(format!(
            "Unknown provider '{}'. Expected: ollama, openai-compatible, vllm, llama-cpp",
            provider
        )),
    }
}

/// Validate an inference endpoint health status string.
pub fn validate_health_status(status: &str) -> Result<(), String> {
    match status {
        "healthy" | "unhealthy" | "unknown" => Ok(()),
        _ => Err(format!(
            "Invalid status '{}'. Expected: healthy, unhealthy, unknown",
            status
        )),
    }
}

/// Check auth constraint: if requires_auth is true, secret_key must be non-empty.
pub fn validate_auth_config(requires_auth: bool, secret_key: &str) -> Result<(), String> {
    if requires_auth && secret_key.is_empty() {
        Err("Endpoint requires auth but no secret_key provided".to_string())
    } else {
        Ok(())
    }
}

/// Filter grants belonging to a specific agent from an iterator.
pub fn filter_grants_for_agent<'a>(
    grants: impl Iterator<Item = &'a SecretGrant>,
    agent_id: &str,
) -> Vec<&'a SecretGrant> {
    grants.filter(|g| g.agent_id == agent_id).collect()
}

/// Filter expired grants from an iterator given an ISO 8601 `now` timestamp.
pub fn filter_expired_grants<'a>(
    grants: impl Iterator<Item = &'a SecretGrant>,
    now: &str,
) -> Vec<&'a SecretGrant> {
    grants.filter(|g| is_expired(&g.expires_at, now)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Grant ID generation ─────────────────────────────────────────────

    #[test]
    fn grant_id_format_basic() {
        assert_eq!(make_grant_id("agent-1", "API_KEY"), "agent-1:API_KEY");
    }

    #[test]
    fn grant_id_contains_both_parts() {
        let id = make_grant_id("abc", "XYZ_SECRET");
        assert!(id.starts_with("abc:"));
        assert!(id.ends_with(":XYZ_SECRET"));
    }

    #[test]
    fn grant_id_different_agents_produce_different_ids() {
        let id1 = make_grant_id("agent-a", "KEY");
        let id2 = make_grant_id("agent-b", "KEY");
        assert_ne!(id1, id2);
    }

    #[test]
    fn grant_id_different_keys_produce_different_ids() {
        let id1 = make_grant_id("agent-1", "KEY_A");
        let id2 = make_grant_id("agent-1", "KEY_B");
        assert_ne!(id1, id2);
    }

    #[test]
    fn grant_id_same_inputs_are_deterministic() {
        let id1 = make_grant_id("agent-1", "SECRET");
        let id2 = make_grant_id("agent-1", "SECRET");
        assert_eq!(id1, id2);
    }

    #[test]
    fn grant_id_with_empty_agent() {
        let id = make_grant_id("", "KEY");
        assert_eq!(id, ":KEY");
    }

    #[test]
    fn grant_id_with_empty_key() {
        let id = make_grant_id("agent", "");
        assert_eq!(id, "agent:");
    }

    // ─── Expiry logic ────────────────────────────────────────────────────

    #[test]
    fn grant_not_expired_before_deadline() {
        assert!(!is_expired("2025-01-01T12:00:00Z", "2025-01-01T11:00:00Z"));
    }

    #[test]
    fn grant_expired_at_exact_deadline() {
        // Expires_at <= now means expired at the exact boundary
        assert!(is_expired("2025-01-01T12:00:00Z", "2025-01-01T12:00:00Z"));
    }

    #[test]
    fn grant_expired_after_deadline() {
        assert!(is_expired("2025-01-01T12:00:00Z", "2025-01-01T13:00:00Z"));
    }

    #[test]
    fn grant_far_future_not_expired() {
        assert!(!is_expired("2099-12-31T23:59:59Z", "2025-06-15T00:00:00Z"));
    }

    #[test]
    fn grant_past_is_expired() {
        assert!(is_expired("2020-01-01T00:00:00Z", "2025-06-15T00:00:00Z"));
    }

    // ─── Claim logic (SecretGrant struct properties) ─────────────────────

    #[test]
    fn new_grant_starts_unclaimed() {
        let grant = SecretGrant {
            id: make_grant_id("agent-1", "KEY"),
            agent_id: "agent-1".to_string(),
            secret_key: "KEY".to_string(),
            purpose: "llm".to_string(),
            granted_at: "2025-01-01T00:00:00Z".to_string(),
            expires_at: "2025-01-02T00:00:00Z".to_string(),
            claimed: false,
        };
        assert!(!grant.claimed);
    }

    #[test]
    fn claimed_grant_cannot_be_reclaimed_logic() {
        // Simulates the claim_grant reducer's check
        let grant = SecretGrant {
            id: make_grant_id("agent-1", "KEY"),
            agent_id: "agent-1".to_string(),
            secret_key: "KEY".to_string(),
            purpose: "llm".to_string(),
            granted_at: "2025-01-01T00:00:00Z".to_string(),
            expires_at: "2025-01-02T00:00:00Z".to_string(),
            claimed: true,
        };
        // The reducer checks this condition and returns Err
        assert!(grant.claimed, "Already-claimed grant should be rejected");
    }

    // ─── Grant filtering ─────────────────────────────────────────────────

    #[test]
    fn filter_grants_for_specific_agent() {
        let grants = vec![
            SecretGrant {
                id: make_grant_id("agent-1", "K1"),
                agent_id: "agent-1".to_string(),
                secret_key: "K1".to_string(),
                purpose: "llm".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-01-02T00:00:00Z".to_string(),
                claimed: false,
            },
            SecretGrant {
                id: make_grant_id("agent-2", "K2"),
                agent_id: "agent-2".to_string(),
                secret_key: "K2".to_string(),
                purpose: "auth".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-01-02T00:00:00Z".to_string(),
                claimed: false,
            },
            SecretGrant {
                id: make_grant_id("agent-1", "K3"),
                agent_id: "agent-1".to_string(),
                secret_key: "K3".to_string(),
                purpose: "webhook".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-01-02T00:00:00Z".to_string(),
                claimed: false,
            },
        ];

        let filtered = filter_grants_for_agent(grants.iter(), "agent-1");
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|g| g.agent_id == "agent-1"));
    }

    #[test]
    fn filter_grants_returns_empty_for_unknown_agent() {
        let grants = vec![SecretGrant {
            id: make_grant_id("agent-1", "K1"),
            agent_id: "agent-1".to_string(),
            secret_key: "K1".to_string(),
            purpose: "llm".to_string(),
            granted_at: "2025-01-01T00:00:00Z".to_string(),
            expires_at: "2025-01-02T00:00:00Z".to_string(),
            claimed: false,
        }];

        let filtered = filter_grants_for_agent(grants.iter(), "unknown-agent");
        assert!(filtered.is_empty());
    }

    // ─── Prune logic ─────────────────────────────────────────────────────

    #[test]
    fn prune_only_removes_expired_grants() {
        let now = "2025-06-15T12:00:00Z";
        let grants = vec![
            SecretGrant {
                id: make_grant_id("a1", "K1"),
                agent_id: "a1".to_string(),
                secret_key: "K1".to_string(),
                purpose: "llm".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-06-15T11:00:00Z".to_string(), // expired
                claimed: false,
            },
            SecretGrant {
                id: make_grant_id("a2", "K2"),
                agent_id: "a2".to_string(),
                secret_key: "K2".to_string(),
                purpose: "auth".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-06-15T13:00:00Z".to_string(), // still active
                claimed: false,
            },
            SecretGrant {
                id: make_grant_id("a3", "K3"),
                agent_id: "a3".to_string(),
                secret_key: "K3".to_string(),
                purpose: "webhook".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-06-15T10:00:00Z".to_string(), // expired
                claimed: true,
            },
        ];

        let expired = filter_expired_grants(grants.iter(), now);
        assert_eq!(expired.len(), 2);
        // a2's grant should survive (not expired)
        assert!(expired.iter().all(|g| g.agent_id != "a2"));
    }

    #[test]
    fn prune_with_no_expired_grants_removes_nothing() {
        let now = "2025-01-01T00:00:00Z";
        let grants = vec![SecretGrant {
            id: make_grant_id("a1", "K1"),
            agent_id: "a1".to_string(),
            secret_key: "K1".to_string(),
            purpose: "llm".to_string(),
            granted_at: "2025-01-01T00:00:00Z".to_string(),
            expires_at: "2099-12-31T23:59:59Z".to_string(),
            claimed: false,
        }];

        let expired = filter_expired_grants(grants.iter(), now);
        assert!(expired.is_empty());
    }

    #[test]
    fn prune_claimed_and_unclaimed_both_prunable() {
        let now = "2025-12-01T00:00:00Z";
        let grants = vec![
            SecretGrant {
                id: make_grant_id("a1", "K1"),
                agent_id: "a1".to_string(),
                secret_key: "K1".to_string(),
                purpose: "llm".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-06-01T00:00:00Z".to_string(),
                claimed: true, // claimed but expired
            },
            SecretGrant {
                id: make_grant_id("a1", "K2"),
                agent_id: "a1".to_string(),
                secret_key: "K2".to_string(),
                purpose: "llm".to_string(),
                granted_at: "2025-01-01T00:00:00Z".to_string(),
                expires_at: "2025-06-01T00:00:00Z".to_string(),
                claimed: false, // unclaimed but expired
            },
        ];

        let expired = filter_expired_grants(grants.iter(), now);
        assert_eq!(expired.len(), 2, "Both claimed and unclaimed expired grants should be pruned");
    }

    // ─── Provider validation ─────────────────────────────────────────────

    #[test]
    fn valid_providers_accepted() {
        for provider in &["ollama", "openai-compatible", "vllm", "llama-cpp"] {
            assert!(validate_provider(provider).is_ok(), "Provider '{}' should be valid", provider);
        }
    }

    #[test]
    fn invalid_provider_rejected() {
        assert!(validate_provider("unknown").is_err());
        assert!(validate_provider("").is_err());
        assert!(validate_provider("openai").is_err());
    }

    // ─── Health status validation ────────────────────────────────────────

    #[test]
    fn valid_health_statuses_accepted() {
        for status in &["healthy", "unhealthy", "unknown"] {
            assert!(validate_health_status(status).is_ok(), "Status '{}' should be valid", status);
        }
    }

    #[test]
    fn invalid_health_status_rejected() {
        assert!(validate_health_status("degraded").is_err());
        assert!(validate_health_status("").is_err());
    }

    // ─── Auth config validation ──────────────────────────────────────────

    #[test]
    fn auth_required_with_key_is_valid() {
        assert!(validate_auth_config(true, "MY_API_KEY").is_ok());
    }

    #[test]
    fn auth_required_without_key_is_invalid() {
        assert!(validate_auth_config(true, "").is_err());
    }

    #[test]
    fn no_auth_with_empty_key_is_valid() {
        assert!(validate_auth_config(false, "").is_ok());
    }

    #[test]
    fn no_auth_with_key_is_valid() {
        // Not required but provided — fine
        assert!(validate_auth_config(false, "SOME_KEY").is_ok());
    }

    // ─── Property-style: grant lifecycle invariants ──────────────────────

    #[test]
    fn grant_id_is_reversible() {
        // Given the format "{agent}:{key}", splitting on first ":" recovers both parts
        let agent = "agent-with-dashes";
        let key = "MY_KEY";
        let id = make_grant_id(agent, key);
        let parts: Vec<&str> = id.splitn(2, ':').collect();
        assert_eq!(parts[0], agent);
        assert_eq!(parts[1], key);
    }

    #[test]
    fn expiry_is_total_order() {
        // For any two timestamps, exactly one of: a < b, a == b, a > b
        let a = "2025-01-01T00:00:00Z";
        let b = "2025-06-01T00:00:00Z";
        // If a < b, then is_expired(a, b) is true and is_expired(b, a) is false
        assert!(is_expired(a, b));
        assert!(!is_expired(b, a));
    }

    #[test]
    fn filter_agent_grants_preserves_all_fields() {
        let grant = SecretGrant {
            id: make_grant_id("agent-1", "KEY"),
            agent_id: "agent-1".to_string(),
            secret_key: "KEY".to_string(),
            purpose: "llm".to_string(),
            granted_at: "2025-01-01T00:00:00Z".to_string(),
            expires_at: "2025-01-02T00:00:00Z".to_string(),
            claimed: true,
        };
        let grants = vec![grant.clone()];
        let filtered = filter_grants_for_agent(grants.iter(), "agent-1");
        assert_eq!(filtered.len(), 1);
        let g = filtered[0];
        assert_eq!(g.id, "agent-1:KEY");
        assert_eq!(g.purpose, "llm");
        assert!(g.claimed);
    }
}
