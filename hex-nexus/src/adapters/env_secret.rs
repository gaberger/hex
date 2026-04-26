//! EnvSecretAdapter — `ISecretPort` impl that resolves secrets from
//! environment variables.
//!
//! First concrete production `ISecretPort` impl in hex-nexus. Unblocks
//! wiring of [`SecretShadowRouter`] (`orchestration::secret_shadow_router`)
//! by giving operators a real adapter to bind as the live entry on the
//! `secret` substrate port. Future adapters (vault, OS keychain, file-
//! based) are alternative strategies the substrate can shadow-test
//! against this one.
//!
//! Read-only: `resolve_secret` reads `std::env::var`. The mutating
//! methods (`grant_secret`, `revoke_secret`) are no-ops returning Ok —
//! env vars are externally managed; the substrate doesn't write to them.
//! `claim_secrets` returns the configured agent's grants from a static
//! per-process registry that's intentionally minimal: this adapter is
//! the simplest possible thing that satisfies the trait so production
//! wiring can land before fancier adapters are written.

use std::collections::HashMap;
use std::env;
use std::sync::Mutex;

use async_trait::async_trait;
use hex_core::domain::secret_grant::{ClaimResult, SecretGrant};
use hex_core::ports::secret::{ISecretPort, SecretError};

pub struct EnvSecretAdapter {
    /// Optional prefix prepended to every key. Lets operators isolate
    /// substrate-managed secrets from incidentally-set env vars (e.g.
    /// `HEX_SECRET_` so `resolve_secret("API_KEY")` reads `HEX_SECRET_API_KEY`).
    /// Empty string for direct lookup.
    prefix: String,
    /// Per-process grant registry. Populated by `grant_secret`, consumed
    /// by `claim_secrets`. Persistence across process restarts is out of
    /// scope for an env-backed adapter; the SpacetimeDB-backed adapter
    /// (future work) is the durable surface.
    grants: Mutex<HashMap<String, Vec<SecretGrant>>>, // agent_id -> grants
}

impl EnvSecretAdapter {
    pub fn new() -> Self {
        Self::with_prefix(String::new())
    }

    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            grants: Mutex::new(HashMap::new()),
        }
    }

    fn resolved_key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}{}", self.prefix, key)
        }
    }
}

impl Default for EnvSecretAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ISecretPort for EnvSecretAdapter {
    async fn resolve_secret(&self, key: &str) -> Result<String, SecretError> {
        let resolved = self.resolved_key(key);
        env::var(&resolved).map_err(|_| SecretError::NotFound(key.into()))
    }

    async fn claim_secrets(&self, agent_id: &str) -> Result<ClaimResult, SecretError> {
        // Drain the agent's pending grants and resolve each one to its
        // env value. Drain semantics match `claim_secrets`'s "one-shot,
        // consumed on use" trait contract.
        let drained: Vec<SecretGrant> = {
            let mut grants = self.grants.lock().unwrap();
            grants.remove(agent_id).unwrap_or_default()
        };
        let mut secrets = HashMap::new();
        for g in &drained {
            if let Ok(v) = env::var(self.resolved_key(&g.secret_key)) {
                secrets.insert(g.secret_key.clone(), v);
            }
            // Missing-from-env grants are silently dropped — operator
            // shouldn't have granted a non-existent key, but we don't
            // fail the whole claim on one missing entry.
        }
        Ok(ClaimResult {
            secrets,
            // Env-resolved values don't have an expiry (the env var lives
            // as long as the process). Report 0 to mean "no rotation
            // window managed by this adapter".
            expires_in: 0,
        })
    }

    async fn grant_secret(&self, grant: &SecretGrant) -> Result<(), SecretError> {
        let mut grants = self.grants.lock().unwrap();
        grants
            .entry(grant.agent_id.clone())
            .or_default()
            .push(grant.clone());
        Ok(())
    }

    async fn revoke_secret(&self, agent_id: &str, key: &str) -> Result<(), SecretError> {
        let mut grants = self.grants.lock().unwrap();
        if let Some(list) = grants.get_mut(agent_id) {
            list.retain(|g| g.secret_key != key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::secret_grant::GrantPurpose;

    // Env-var test isolation: tests that touch process-wide env state
    // serialize on this lock to avoid cross-test interference. Other
    // env-touching tests in the workspace (e.g. session_tests) use the
    // same pattern.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn unique_var(name: &str) -> String {
        // Use a per-test unique prefix derived from std::process::id +
        // an atomic counter so parallel test invocations don't trample
        // each other's env vars even within the lock-guarded region.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("HEX_TEST_ENV_SECRET_{}_{}_{}", std::process::id(), n, name)
    }

    fn grant_for(agent: &str, key: &str) -> SecretGrant {
        SecretGrant {
            agent_id: agent.into(),
            secret_key: key.into(),
            purpose: GrantPurpose::Llm,
            granted_at: chrono::Utc::now().to_rfc3339(),
            expires_at: chrono::Utc::now().to_rfc3339(),
            claimed: false,
        }
    }

    #[tokio::test]
    async fn resolves_secret_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        let key = unique_var("RESOLVES");
        std::env::set_var(&key, "secret-value");
        let adapter = EnvSecretAdapter::new();
        let v = adapter.resolve_secret(&key).await.unwrap();
        assert_eq!(v, "secret-value");
        std::env::remove_var(&key);
    }

    #[tokio::test]
    async fn returns_not_found_for_missing_env_var() {
        let _lock = ENV_LOCK.lock().unwrap();
        let key = unique_var("MISSING");
        std::env::remove_var(&key);
        let adapter = EnvSecretAdapter::new();
        let err = adapter.resolve_secret(&key).await.unwrap_err();
        assert!(matches!(err, SecretError::NotFound(_)));
    }

    #[tokio::test]
    async fn prefix_is_prepended_to_lookups() {
        let _lock = ENV_LOCK.lock().unwrap();
        let suffix = unique_var("PREFIX");
        let full = format!("HEX_SECRET_{}", suffix);
        std::env::set_var(&full, "with-prefix");
        let adapter = EnvSecretAdapter::with_prefix("HEX_SECRET_");
        let v = adapter.resolve_secret(&suffix).await.unwrap();
        assert_eq!(v, "with-prefix");
        std::env::remove_var(&full);
    }

    #[tokio::test]
    async fn empty_prefix_is_direct_lookup() {
        let _lock = ENV_LOCK.lock().unwrap();
        let key = unique_var("DIRECT");
        std::env::set_var(&key, "direct");
        let adapter = EnvSecretAdapter::with_prefix("");
        let v = adapter.resolve_secret(&key).await.unwrap();
        assert_eq!(v, "direct");
        std::env::remove_var(&key);
    }

    #[tokio::test]
    async fn grant_then_claim_returns_resolved_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        let key = unique_var("CLAIM");
        std::env::set_var(&key, "claimed-value");
        let adapter = EnvSecretAdapter::new();
        adapter.grant_secret(&grant_for("agent-a", &key)).await.unwrap();
        let claim = adapter.claim_secrets("agent-a").await.unwrap();
        assert_eq!(claim.secrets.get(&key), Some(&"claimed-value".to_string()));
        // One-shot: second claim returns empty.
        let again = adapter.claim_secrets("agent-a").await.unwrap();
        assert!(again.secrets.is_empty(), "claim_secrets is one-shot");
        std::env::remove_var(&key);
    }

    #[tokio::test]
    async fn claim_with_no_grants_returns_empty() {
        let adapter = EnvSecretAdapter::new();
        let claim = adapter.claim_secrets("never-granted").await.unwrap();
        assert!(claim.secrets.is_empty());
        assert_eq!(claim.expires_in, 0);
    }

    #[tokio::test]
    async fn revoke_removes_grant_from_pending() {
        let _lock = ENV_LOCK.lock().unwrap();
        let key = unique_var("REVOKE");
        std::env::set_var(&key, "should-not-be-claimed");
        let adapter = EnvSecretAdapter::new();
        adapter.grant_secret(&grant_for("agent-b", &key)).await.unwrap();
        adapter.revoke_secret("agent-b", &key).await.unwrap();
        let claim = adapter.claim_secrets("agent-b").await.unwrap();
        assert!(claim.secrets.is_empty(), "revoked grant must not surface in claim");
        std::env::remove_var(&key);
    }

    #[tokio::test]
    async fn missing_env_var_during_claim_silently_drops_that_entry() {
        let _lock = ENV_LOCK.lock().unwrap();
        let present = unique_var("PRESENT");
        let absent = unique_var("ABSENT");
        std::env::set_var(&present, "ok");
        std::env::remove_var(&absent);
        let adapter = EnvSecretAdapter::new();
        adapter.grant_secret(&grant_for("agent-c", &present)).await.unwrap();
        adapter.grant_secret(&grant_for("agent-c", &absent)).await.unwrap();
        let claim = adapter.claim_secrets("agent-c").await.unwrap();
        assert_eq!(claim.secrets.len(), 1);
        assert!(claim.secrets.contains_key(&present));
        std::env::remove_var(&present);
    }
}
