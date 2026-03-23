//! Environment variable secrets adapter (ADR-026).
//!
//! Reads secrets from process environment variables. Used by agents that
//! are spawned by hex-hub, which injects granted secrets as env vars.

use async_trait::async_trait;
use std::collections::HashMap;

use crate::ports::secret_broker::{SecretBrokerPort, SecretError, SecretResult};

/// Resolves secrets from process environment variables.
///
/// This is the primary adapter for agents spawned by hex-hub. The broker
/// resolves secrets via ISecretsPort and injects them as env vars into
/// the child process before it starts.
pub struct EnvSecretsAdapter;

impl Default for EnvSecretsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvSecretsAdapter {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SecretBrokerPort for EnvSecretsAdapter {
    async fn resolve_secret(&self, key: &str) -> SecretResult {
        std::env::var(key).map_err(|_| SecretError::NotFound {
            key: key.to_string(),
        })
    }

    async fn claim_secrets(&self, _agent_id: &str) -> Result<HashMap<String, String>, SecretError> {
        // Env adapter doesn't need to claim — secrets are already injected.
        // Return all env vars matching common secret patterns.
        let patterns = ["_KEY", "_SECRET", "_TOKEN", "_PASSWORD"];
        let secrets: HashMap<String, String> = std::env::vars()
            .filter(|(k, _)| patterns.iter().any(|p| k.ends_with(p)))
            .collect();
        Ok(secrets)
    }

    async fn has_secret(&self, key: &str) -> bool {
        std::env::var(key).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_existing_env_var() {
        unsafe { std::env::set_var("TEST_SECRET_KEY_926", "test-value") };
        let adapter = EnvSecretsAdapter::new();
        let result = adapter.resolve_secret("TEST_SECRET_KEY_926").await;
        assert_eq!(result.unwrap(), "test-value");
        unsafe { std::env::remove_var("TEST_SECRET_KEY_926") };
    }

    #[tokio::test]
    async fn resolve_missing_returns_not_found() {
        let adapter = EnvSecretsAdapter::new();
        let result = adapter.resolve_secret("DEFINITELY_MISSING_KEY_XYZ").await;
        assert!(matches!(result, Err(SecretError::NotFound { .. })));
    }

    #[tokio::test]
    async fn has_secret_checks_existence() {
        unsafe { std::env::set_var("TEST_HAS_SECRET_KEY", "yes") };
        let adapter = EnvSecretsAdapter::new();
        assert!(adapter.has_secret("TEST_HAS_SECRET_KEY").await);
        assert!(!adapter.has_secret("NOPE_NOT_HERE").await);
        unsafe { std::env::remove_var("TEST_HAS_SECRET_KEY") };
    }

    #[tokio::test]
    async fn resolve_returns_exact_value() {
        let key = "TEST_EXACT_VALUE_KEY_428";
        let value = "s3cr3t-with-special-chars!@#$%";
        unsafe { std::env::set_var(key, value) };
        let adapter = EnvSecretsAdapter::new();
        assert_eq!(adapter.resolve_secret(key).await.unwrap(), value);
        unsafe { std::env::remove_var(key) };
    }

    #[tokio::test]
    async fn resolve_empty_value_succeeds() {
        let key = "TEST_EMPTY_VALUE_KEY_429";
        unsafe { std::env::set_var(key, "") };
        let adapter = EnvSecretsAdapter::new();
        assert_eq!(adapter.resolve_secret(key).await.unwrap(), "");
        unsafe { std::env::remove_var(key) };
    }

    #[tokio::test]
    async fn has_secret_with_empty_value_returns_true() {
        let key = "TEST_EMPTY_HAS_KEY_430";
        unsafe { std::env::set_var(key, "") };
        let adapter = EnvSecretsAdapter::new();
        assert!(adapter.has_secret(key).await);
        unsafe { std::env::remove_var(key) };
    }

    #[tokio::test]
    async fn claim_secrets_finds_matching_patterns() {
        let keys = [
            "TEST_CLAIM_API_KEY",
            "TEST_CLAIM_DB_SECRET",
            "TEST_CLAIM_AUTH_TOKEN",
            "TEST_CLAIM_DB_PASSWORD",
        ];
        for k in &keys {
            unsafe { std::env::set_var(k, "val") };
        }
        // Also set a non-matching key
        unsafe { std::env::set_var("TEST_CLAIM_HOSTNAME", "localhost") };

        let adapter = EnvSecretsAdapter::new();
        let secrets = adapter.claim_secrets("any-agent").await.unwrap();

        // All four pattern-matching keys should be present
        for k in &keys {
            assert!(secrets.contains_key(*k), "Expected key '{}' in claimed secrets", k);
        }
        // Non-matching key should NOT be present
        assert!(!secrets.contains_key("TEST_CLAIM_HOSTNAME"));

        for k in &keys {
            unsafe { std::env::remove_var(k) };
        }
        unsafe { std::env::remove_var("TEST_CLAIM_HOSTNAME") };
    }

    #[tokio::test]
    async fn resolve_after_remove_returns_not_found() {
        let key = "TEST_REMOVE_KEY_431";
        unsafe { std::env::set_var(key, "temporary") };
        let adapter = EnvSecretsAdapter::new();
        assert!(adapter.resolve_secret(key).await.is_ok());
        unsafe { std::env::remove_var(key) };
        let err = adapter.resolve_secret(key).await.unwrap_err();
        assert!(matches!(err, SecretError::NotFound { .. }));
    }

    #[tokio::test]
    async fn not_found_error_contains_key_name() {
        let adapter = EnvSecretsAdapter::new();
        let key = "UNIQUE_MISSING_KEY_XQ7";
        let err = adapter.resolve_secret(key).await.unwrap_err();
        match err {
            SecretError::NotFound { key: k } => assert_eq!(k, key),
            _ => panic!("Expected NotFound error"),
        }
    }
}
