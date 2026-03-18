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
}
