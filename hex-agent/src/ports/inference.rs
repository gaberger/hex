//! Inference discovery port (ADR-026).
//!
//! Outbound port for discovering available inference endpoints.
//! Backed by SpacetimeDB subscription to the inference_endpoint table.

use async_trait::async_trait;
use crate::domain::secret_grant::{InferenceEndpoint, InferenceProvider};

/// Port for discovering and selecting inference endpoints.
#[async_trait]
pub trait InferenceDiscoveryPort: Send + Sync {
    /// List all registered inference endpoints.
    async fn list_endpoints(&self) -> Result<Vec<InferenceEndpoint>, InferenceDiscoveryError>;

    /// List only healthy endpoints, optionally filtered by provider.
    async fn healthy_endpoints(
        &self,
        provider: Option<&InferenceProvider>,
    ) -> Result<Vec<InferenceEndpoint>, InferenceDiscoveryError>;

    /// Get a specific endpoint by ID.
    async fn get_endpoint(&self, id: &str) -> Result<Option<InferenceEndpoint>, InferenceDiscoveryError>;
}

/// Errors from inference discovery.
#[derive(Debug, thiserror::Error)]
pub enum InferenceDiscoveryError {
    #[error("SpacetimeDB connection lost")]
    Disconnected,

    #[error("Discovery failed: {0}")]
    Other(String),
}
