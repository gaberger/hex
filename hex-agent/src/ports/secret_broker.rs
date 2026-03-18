//! Secret broker port (ADR-026).
//!
//! Outbound port for resolving secrets at runtime. Two adapters:
//! - EnvSecretsAdapter: reads from process env vars (spawned agents)
//! - HubClaimAdapter: one-shot HTTP claim from hex-hub (independent agents)

use async_trait::async_trait;
use std::collections::HashMap;

/// Result of a single secret resolution.
pub type SecretResult = Result<String, SecretError>;

/// Errors from secret resolution.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("Secret '{key}' not found")]
    NotFound { key: String },

    #[error("Secret '{key}' has expired")]
    Expired { key: String },

    #[error("Claim rejected: {reason}")]
    ClaimRejected { reason: String },

    #[error("Hub unreachable: {0}")]
    HubUnreachable(String),

    #[error("Secret resolution failed: {0}")]
    Other(String),
}

/// Port for resolving secrets at runtime.
///
/// Agents use this to obtain API keys for inference servers and other services.
/// The port intentionally does NOT expose grant management — that's hex-hub's
/// responsibility via SpacetimeDB reducers.
#[async_trait]
pub trait SecretBrokerPort: Send + Sync {
    /// Resolve a single secret by key name.
    async fn resolve_secret(&self, key: &str) -> SecretResult;

    /// Claim all granted secrets for this agent from hex-hub.
    /// Returns a map of secret_key → secret_value.
    async fn claim_secrets(&self, agent_id: &str) -> Result<HashMap<String, String>, SecretError>;

    /// Check whether a secret is available without retrieving its value.
    async fn has_secret(&self, key: &str) -> bool;
}
