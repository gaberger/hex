//! Hub claim secrets adapter (ADR-026).
//!
//! Resolves secrets by making a one-shot HTTP claim to hex-hub's
//! /secrets/claim endpoint. Used by independently started agents
//! (debugging, remote nodes) that weren't spawned by hex-hub.

use async_trait::async_trait;
use std::collections::HashMap;

use crate::ports::secret_broker::{SecretBrokerPort, SecretError, SecretResult};

/// Configuration for connecting to hex-hub's secret broker.
#[derive(Debug, Clone)]
pub struct HubClaimConfig {
    /// hex-hub base URL (e.g. "http://127.0.0.1:4280")
    pub hub_url: String,
    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl Default for HubClaimConfig {
    fn default() -> Self {
        Self {
            hub_url: "http://127.0.0.1:4280".to_string(),
            timeout_secs: 10,
        }
    }
}

/// Resolves secrets via one-shot HTTP claim to hex-hub.
///
/// Flow:
/// 1. Agent calls claim_secrets(agent_id)
/// 2. Adapter sends POST to hub_url/secrets/claim with agent_id + nonce
/// 3. hex-hub verifies grant exists, resolves via ISecretsPort, responds
/// 4. Adapter caches resolved secrets in memory for subsequent resolve_secret() calls
/// 5. Claim is single-use on the hub side (409 on replay)
pub struct HubClaimSecretsAdapter {
    config: HubClaimConfig,
    client: reqwest::Client,
    /// Cached secrets after a successful claim.
    cache: tokio::sync::RwLock<HashMap<String, String>>,
}

impl HubClaimSecretsAdapter {
    pub fn new(config: HubClaimConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            config,
            client,
            cache: tokio::sync::RwLock::new(HashMap::new()),
        }
    }
}

/// Claim request body.
#[derive(serde::Serialize)]
struct ClaimRequest {
    agent_id: String,
    nonce: String,
}

/// Claim response body.
#[derive(serde::Deserialize)]
struct ClaimResponse {
    secrets: HashMap<String, String>,
    expires_in: u64,
}

#[async_trait]
impl SecretBrokerPort for HubClaimSecretsAdapter {
    async fn resolve_secret(&self, key: &str) -> SecretResult {
        // Check cache first
        let cache = self.cache.read().await;
        if let Some(value) = cache.get(key) {
            return Ok(value.clone());
        }
        drop(cache);

        // Fall back to env var (hex-hub may have injected it)
        std::env::var(key).map_err(|_| SecretError::NotFound {
            key: key.to_string(),
        })
    }

    async fn claim_secrets(&self, agent_id: &str) -> Result<HashMap<String, String>, SecretError> {
        let nonce = uuid::Uuid::new_v4().to_string();
        let url = format!("{}/secrets/claim", self.config.hub_url);

        let response = self
            .client
            .post(&url)
            .json(&ClaimRequest {
                agent_id: agent_id.to_string(),
                nonce,
            })
            .send()
            .await
            .map_err(|e| SecretError::HubUnreachable(e.to_string()))?;

        let status = response.status().as_u16();
        match status {
            200 => {
                let claim: ClaimResponse = response
                    .json()
                    .await
                    .map_err(|e| SecretError::Other(format!("Invalid claim response: {}", e)))?;

                // Cache the claimed secrets
                let mut cache = self.cache.write().await;
                for (k, v) in &claim.secrets {
                    cache.insert(k.clone(), v.clone());
                }

                tracing::info!(
                    agent_id,
                    secret_count = claim.secrets.len(),
                    expires_in = claim.expires_in,
                    "Claimed secrets from hub"
                );

                Ok(claim.secrets)
            }
            409 => Err(SecretError::ClaimRejected {
                reason: "Grant already claimed (single-use)".to_string(),
            }),
            404 => Err(SecretError::ClaimRejected {
                reason: "No grants found for this agent".to_string(),
            }),
            410 => Err(SecretError::Expired {
                key: agent_id.to_string(),
            }),
            _ => {
                let body = response.text().await.unwrap_or_default();
                Err(SecretError::Other(format!(
                    "Claim failed (HTTP {}): {}",
                    status, body
                )))
            }
        }
    }

    async fn has_secret(&self, key: &str) -> bool {
        let cache = self.cache.read().await;
        if cache.contains_key(key) {
            return true;
        }
        drop(cache);
        std::env::var(key).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_adapter() -> HubClaimSecretsAdapter {
        HubClaimSecretsAdapter::new(HubClaimConfig::default())
    }

    #[test]
    fn default_config_uses_localhost() {
        let config = HubClaimConfig::default();
        assert_eq!(config.hub_url, "http://127.0.0.1:4280");
        assert_eq!(config.timeout_secs, 10);
    }

    #[test]
    fn custom_config_is_preserved() {
        let config = HubClaimConfig {
            hub_url: "http://10.0.0.5:9999".to_string(),
            timeout_secs: 30,
        };
        let adapter = HubClaimSecretsAdapter::new(config.clone());
        assert_eq!(adapter.config.hub_url, "http://10.0.0.5:9999");
        assert_eq!(adapter.config.timeout_secs, 30);
    }

    #[tokio::test]
    async fn resolve_from_cache_returns_cached_value() {
        let adapter = make_adapter();
        // Manually populate the cache
        {
            let mut cache = adapter.cache.write().await;
            cache.insert("CACHED_KEY".to_string(), "cached-value".to_string());
        }
        let result = adapter.resolve_secret("CACHED_KEY").await;
        assert_eq!(result.unwrap(), "cached-value");
    }

    #[tokio::test]
    async fn resolve_falls_back_to_env_when_not_cached() {
        let key = "TEST_HUB_FALLBACK_KEY_501";
        unsafe { std::env::set_var(key, "env-value") };
        let adapter = make_adapter();
        let result = adapter.resolve_secret(key).await;
        assert_eq!(result.unwrap(), "env-value");
        unsafe { std::env::remove_var(key) };
    }

    #[tokio::test]
    async fn resolve_not_found_when_neither_cached_nor_env() {
        let adapter = make_adapter();
        let result = adapter.resolve_secret("TOTALLY_MISSING_KEY_502").await;
        assert!(matches!(result, Err(SecretError::NotFound { .. })));
    }

    #[tokio::test]
    async fn has_secret_returns_true_for_cached() {
        let adapter = make_adapter();
        {
            let mut cache = adapter.cache.write().await;
            cache.insert("HAS_CACHED_KEY".to_string(), "v".to_string());
        }
        assert!(adapter.has_secret("HAS_CACHED_KEY").await);
    }

    #[tokio::test]
    async fn has_secret_returns_true_for_env_var() {
        let key = "TEST_HUB_HAS_ENV_KEY_503";
        unsafe { std::env::set_var(key, "x") };
        let adapter = make_adapter();
        assert!(adapter.has_secret(key).await);
        unsafe { std::env::remove_var(key) };
    }

    #[tokio::test]
    async fn has_secret_returns_false_when_missing() {
        let adapter = make_adapter();
        assert!(!adapter.has_secret("MISSING_KEY_504").await);
    }

    #[tokio::test]
    async fn cache_prefers_over_env() {
        let key = "TEST_HUB_CACHE_PREF_505";
        unsafe { std::env::set_var(key, "env-value") };
        let adapter = make_adapter();
        {
            let mut cache = adapter.cache.write().await;
            cache.insert(key.to_string(), "cached-value".to_string());
        }
        // Cache should win
        let result = adapter.resolve_secret(key).await;
        assert_eq!(result.unwrap(), "cached-value");
        unsafe { std::env::remove_var(key) };
    }

    #[tokio::test]
    async fn claim_to_unreachable_hub_returns_hub_unreachable() {
        // Connect to a port that (almost certainly) has nothing listening
        let config = HubClaimConfig {
            hub_url: "http://127.0.0.1:1".to_string(),
            timeout_secs: 1,
        };
        let adapter = HubClaimSecretsAdapter::new(config);
        let result = adapter.claim_secrets("agent-1").await;
        assert!(matches!(result, Err(SecretError::HubUnreachable(_))));
    }

    #[tokio::test]
    async fn multiple_cache_entries_coexist() {
        let adapter = make_adapter();
        {
            let mut cache = adapter.cache.write().await;
            cache.insert("KEY_A".to_string(), "value-a".to_string());
            cache.insert("KEY_B".to_string(), "value-b".to_string());
            cache.insert("KEY_C".to_string(), "value-c".to_string());
        }
        assert_eq!(adapter.resolve_secret("KEY_A").await.unwrap(), "value-a");
        assert_eq!(adapter.resolve_secret("KEY_B").await.unwrap(), "value-b");
        assert_eq!(adapter.resolve_secret("KEY_C").await.unwrap(), "value-c");
    }
}
