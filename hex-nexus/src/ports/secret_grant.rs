//! Port for secret grant operations (ADR-026).
//!
//! Defines the contract for secret grant lifecycle management.
//! The sole implementation is the SpacetimeDB adapter — there is
//! no in-memory fallback by design (secrets must be distributed).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Domain Types ────────────────────────────────────────────

/// A secret grant record — metadata about which agent can claim which key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretGrant {
    pub id: String,
    pub agent_id: String,
    pub secret_key: String,
    pub purpose: String,
    pub hub_id: String,
    pub granted_at: String,
    pub expires_at: String,
    pub claimed: bool,
    pub claimed_at: Option<String>,
    pub claim_hub_id: Option<String>,
}

/// An encrypted vault entry — actual secret value (encrypted at rest).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEntry {
    pub key: String,
    pub encrypted_value: String,
    pub key_version: u32,
    pub stored_at: String,
    pub stored_by_hub: String,
}

/// An audit log entry for secret operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretAuditEntry {
    pub id: String,
    pub action: String,
    pub agent_id: String,
    pub secret_key: String,
    pub hub_id: String,
    pub timestamp: String,
}

/// Health status of the secret grant backend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretBackendHealth {
    pub connected: bool,
    pub backend: String,
    pub host: String,
    pub circuit_state: String,
    pub last_error: Option<String>,
}

// ── Port Trait ──────────────────────────────────────────────

#[async_trait]
pub trait ISecretGrantPort: Send + Sync {
    // ── Grant lifecycle ──

    /// Create a grant allowing an agent to claim a secret key.
    async fn grant(
        &self,
        agent_id: &str,
        secret_key: &str,
        purpose: &str,
        hub_id: &str,
        ttl_secs: u64,
    ) -> Result<SecretGrant, String>;

    /// Claim all unclaimed grants for an agent. Returns resolved secret values.
    async fn claim(
        &self,
        agent_id: &str,
        nonce: &str,
        hub_id: &str,
    ) -> Result<Vec<SecretGrant>, String>;

    /// Revoke a specific grant.
    async fn revoke(&self, agent_id: &str, secret_key: &str) -> Result<(), String>;

    /// Revoke all grants for an agent.
    async fn revoke_all(&self, agent_id: &str) -> Result<usize, String>;

    /// List all active (non-expired) grants.
    async fn list_grants(&self) -> Result<Vec<SecretGrant>, String>;

    /// List grants for a specific agent.
    async fn list_grants_for_agent(&self, agent_id: &str) -> Result<Vec<SecretGrant>, String>;

    /// Prune expired grants.
    async fn prune_expired(&self) -> Result<usize, String>;

    // ── Vault (encrypted values) ──

    /// Store an encrypted secret value.
    async fn vault_store(&self, key: &str, value: &str) -> Result<(), String>;

    /// Retrieve and decrypt a secret value.
    async fn vault_get(&self, key: &str) -> Result<Option<String>, String>;

    /// Delete a secret from the vault.
    async fn vault_delete(&self, key: &str) -> Result<(), String>;

    /// List ALL secrets from the vault (key → decrypted value).
    async fn vault_list(&self) -> Result<std::collections::HashMap<String, String>, String>;

    // ── Health ──

    /// Check backend connectivity and return health status.
    async fn health(&self) -> SecretBackendHealth;

    /// Whether the backend is currently reachable.
    async fn is_healthy(&self) -> bool;
}
