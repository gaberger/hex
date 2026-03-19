//! Secret port — vault access contract.

use async_trait::async_trait;

use crate::domain::secret_grant::{ClaimResult, SecretGrant};

/// The secret port — adapters implement secret resolution.
#[async_trait]
pub trait ISecretPort: Send + Sync {
    /// Resolve a secret value by key name.
    async fn resolve_secret(&self, key: &str) -> Result<String, SecretError>;

    /// Claim secrets for an agent (one-shot, consumed on use).
    async fn claim_secrets(&self, agent_id: &str) -> Result<ClaimResult, SecretError>;

    /// Grant a secret to an agent.
    async fn grant_secret(&self, grant: &SecretGrant) -> Result<(), SecretError>;

    /// Revoke a secret grant.
    async fn revoke_secret(&self, agent_id: &str, key: &str) -> Result<(), SecretError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("Secret not found: {0}")]
    NotFound(String),
    #[error("Grant expired for agent {agent_id}, key {key}")]
    Expired { agent_id: String, key: String },
    #[error("Already claimed by agent {0}")]
    AlreadyClaimed(String),
    #[error("Vault unavailable: {0}")]
    VaultUnavailable(String),
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
}
