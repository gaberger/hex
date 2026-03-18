//! Secret grant domain types (ADR-026).
//!
//! Pure value objects for secret distribution. No external dependencies.

use serde::{Deserialize, Serialize};

/// Purpose tag for a secret grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantPurpose {
    /// LLM inference API key
    Llm,
    /// Webhook authentication
    Webhook,
    /// General authentication token
    Auth,
    /// Custom purpose
    Custom(String),
}

/// A secret grant — metadata only, never contains the actual secret value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretGrant {
    pub agent_id: String,
    pub secret_key: String,
    pub purpose: GrantPurpose,
    pub granted_at: String,
    pub expires_at: String,
    pub claimed: bool,
}

/// Result of claiming secrets from the hub broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimResult {
    /// Resolved secret values keyed by secret_key name.
    pub secrets: std::collections::HashMap<String, String>,
    /// Seconds until the claimed secrets expire.
    pub expires_in: u64,
}

/// A discoverable inference endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceEndpoint {
    pub id: String,
    pub url: String,
    pub provider: InferenceProvider,
    pub model: String,
    pub status: EndpointStatus,
    pub requires_auth: bool,
    /// Secret key name (empty if no auth required).
    pub secret_key: String,
}

/// Supported local inference providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InferenceProvider {
    Ollama,
    OpenaiCompatible,
    Vllm,
    LlamaCpp,
}

impl InferenceProvider {
    /// Default base URL for this provider.
    pub fn default_url(&self) -> &'static str {
        match self {
            Self::Ollama => "http://127.0.0.1:11434",
            Self::Vllm => "http://127.0.0.1:8000",
            Self::LlamaCpp => "http://127.0.0.1:8080",
            Self::OpenaiCompatible => "http://127.0.0.1:8000",
        }
    }

    /// The chat completions path for this provider.
    pub fn chat_path(&self) -> &'static str {
        match self {
            Self::Ollama => "/v1/chat/completions",
            Self::OpenaiCompatible | Self::Vllm | Self::LlamaCpp => "/v1/chat/completions",
        }
    }
}

/// Health status of an inference endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EndpointStatus {
    Healthy,
    Unhealthy,
    Unknown,
}
