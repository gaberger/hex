//! `Endpoint` — one inference backend, described at runtime.
//!
//! Replaces `hex-nexus/src/routes/secrets.rs::InferenceEndpointEntry` per
//! ADR-2608241500 P2.4, with one change that matters: the secret is no longer
//! a reference into a SpacetimeDB vault. It is the name of an environment
//! variable, resolved locally.
//!
//! # Why the vault goes away
//!
//! The daemon stored API keys in a SpacetimeDB vault and resolved references
//! at dispatch time, under a 3-second timeout, with three separate failure
//! modes that each aborted a request. For a single-user tool on one machine
//! that is a distributed system standing in for `std::env::var`. The key was
//! already in the environment at some point — that is how it reached the
//! vault. Reading it from the environment removes a network hop, a timeout,
//! and a class of "secret_resolution_failed" errors, and it keeps keys out of
//! every file hex writes.

use serde::{Deserialize, Serialize};

/// A single inference backend hex can dispatch to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Endpoint {
    /// Stable identifier, e.g. `ollama-local`, `tenstorrent-qwen3-32b`.
    pub id: String,
    /// Base URL. Family-specific paths are appended by the adapter.
    pub url: String,
    /// Provider family: `ollama`, `openrouter`, `openai_compat`, `vllm`, …
    pub provider: String,
    /// The model this endpoint serves for the request at hand.
    pub model: String,
    /// Every model this endpoint advertises. Matched exactly by
    /// [`crate::registry::serving`]; never by substring.
    #[serde(default)]
    pub models: Vec<String>,
    /// `healthy` | `unknown` | anything else (treated as unhealthy).
    pub status: String,
    /// Whether a bearer token is required.
    pub requires_auth: bool,
    /// The resolved API key, or — before [`Endpoint::resolve_secret`] runs —
    /// the name of the environment variable holding it.
    pub secret_key: String,
    /// RFC 3339 timestamp of the last health check, or empty.
    pub health_checked_at: String,
    /// Benchmarked quality, 0.0 when never calibrated. Set by
    /// `hex inference test` and preserved across registry rewrites.
    #[serde(default)]
    pub quality_score: f32,
    /// Weight quantisation (`q4`, `q8`, `cloud`, …). Retained for ordering
    /// and diagnostics; hex does not reason about it beyond preferring a
    /// higher-quality endpoint when nothing else separates two candidates.
    #[serde(default)]
    pub quantization_level: String,
}

impl Endpoint {
    /// Provider families that accept an OpenAI-compatible `tools` array.
    /// Callers use this to pick a candidate for the tools fast-path and fall
    /// back to no-tools delegation for everything else.
    pub fn supports_tools(&self) -> bool {
        let p = self.provider.to_ascii_lowercase();
        matches!(
            p.as_str(),
            "openrouter"
                | "openai"
                | "openai-compat"
                | "openai_compat"
                | "openai-compatible"
                | "ollama"
                | "vllm"
                | "llama-cpp"
                | "llamacpp"
        ) || self.url.contains("openrouter.ai")
    }

    /// True for backends running on the operator's own hardware. These get a
    /// longer timeout (cold model load) and a different retry shape (503 means
    /// "still loading", not "broken").
    pub fn is_local(&self) -> bool {
        matches!(
            self.provider.to_ascii_lowercase().as_str(),
            "ollama" | "vllm" | "llama-cpp" | "llamacpp"
        )
    }

    /// Priority order for tools-capable providers. Lower wins.
    ///
    /// Default order is "free and local first, paid and remote last":
    ///   0  ollama / vllm / llama-cpp        (local, free)
    ///   2  openai-compat (custom hosted)
    ///   3  openrouter
    ///   5  unknown family
    ///
    /// Within a tier, healthy beats unknown beats unhealthy. An operator's
    /// `hex inference add ollama …` therefore out-prioritises the env-var
    /// OpenRouter fallback without anyone editing code.
    pub fn priority_for_tools(&self) -> i32 {
        let provider_tier: i32 = match self.provider.to_ascii_lowercase().as_str() {
            "ollama" | "vllm" | "llama-cpp" | "llamacpp" => 0,
            "openai-compat" | "openai_compat" | "openai-compatible" | "openai" => 2,
            "openrouter" => 3,
            _ => 5,
        };
        let health: i32 = match self.status.to_ascii_lowercase().as_str() {
            "healthy" => 0,
            "" | "unknown" => 1,
            _ => 2,
        };
        provider_tier * 10 + health
    }

    /// Resolve `secret_key` from the environment when it names a variable
    /// rather than holding a literal key.
    ///
    /// A value starting with `sk-` is already a literal and is left alone.
    /// Returns `false` when a reference cannot be resolved, so the caller can
    /// skip this candidate instead of sending `Bearer TENSTORRENT` upstream
    /// and getting a misleading 401 — the exact failure the daemon's vault
    /// path guarded against with a timeout and three error branches.
    pub fn resolve_secret(&mut self) -> bool {
        if !self.requires_auth || self.secret_key.is_empty() {
            return true;
        }
        if self.secret_key.starts_with("sk-") {
            return true;
        }
        match std::env::var(&self.secret_key) {
            Ok(val) if !val.is_empty() => {
                self.secret_key = val;
                true
            }
            _ => false,
        }
    }

    /// Honour a caller's requested model on a local endpoint.
    ///
    /// One Ollama or vLLM server serves any locally-available model, so a
    /// registered endpoint's model must not pin the request — otherwise
    /// `hex do --model` and the bench `--model` are silently ignored and every
    /// call runs the registered default. Diagnostic 2026-06-07: the bench
    /// `react` arm ran gemma4-12b regardless of `--model` because of this.
    ///
    /// Remote endpoints are left alone: they serve exactly what they advertise.
    pub fn honour_requested_model(&mut self, requested: Option<&str>) {
        let Some(m) = requested.map(str::trim).filter(|m| !m.is_empty()) else {
            return;
        };
        if m != self.model && self.is_local() {
            self.model = m.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(provider: &str, status: &str) -> Endpoint {
        Endpoint {
            id: "e".into(),
            url: "http://localhost:11434".into(),
            provider: provider.into(),
            model: "m".into(),
            models: vec!["m".into()],
            status: status.into(),
            requires_auth: false,
            secret_key: String::new(),
            health_checked_at: String::new(),
            quality_score: 0.0,
            quantization_level: String::new(),
        }
    }

    #[test]
    fn local_families_are_recognised() {
        for p in ["ollama", "vllm", "llama-cpp", "llamacpp", "Ollama"] {
            assert!(ep(p, "healthy").is_local(), "{p} should be local");
        }
        for p in ["openrouter", "openai_compat", "anthropic"] {
            assert!(!ep(p, "healthy").is_local(), "{p} should not be local");
        }
    }

    #[test]
    fn openrouter_is_detected_by_url_even_with_an_odd_family() {
        let mut e = ep("mystery", "unknown");
        e.url = "https://openrouter.ai/api/v1".into();
        assert!(e.supports_tools());
    }

    #[test]
    fn unknown_family_does_not_claim_tool_support() {
        assert!(!ep("mystery", "healthy").supports_tools());
    }

    #[test]
    fn local_outranks_cloud_and_healthy_outranks_unknown() {
        assert!(ep("ollama", "healthy").priority_for_tools() < ep("ollama", "unknown").priority_for_tools());
        assert!(ep("ollama", "unknown").priority_for_tools() < ep("openai_compat", "healthy").priority_for_tools());
        assert!(ep("openai_compat", "healthy").priority_for_tools() < ep("openrouter", "healthy").priority_for_tools());
        assert!(ep("openrouter", "healthy").priority_for_tools() < ep("mystery", "healthy").priority_for_tools());
    }

    #[test]
    fn a_literal_key_needs_no_resolution() {
        let mut e = ep("openrouter", "unknown");
        e.requires_auth = true;
        e.secret_key = "sk-or-v1-literal".into();
        assert!(e.resolve_secret());
        assert_eq!(e.secret_key, "sk-or-v1-literal");
    }

    #[test]
    fn an_unresolvable_reference_reports_failure_instead_of_sending_the_ref() {
        let mut e = ep("openai_compat", "unknown");
        e.requires_auth = true;
        e.secret_key = "HEX_TEST_KEY_THAT_DOES_NOT_EXIST".into();
        assert!(!e.resolve_secret());
        // The reference is left intact so the caller can log which one failed.
        assert_eq!(e.secret_key, "HEX_TEST_KEY_THAT_DOES_NOT_EXIST");
    }

    #[test]
    fn no_auth_endpoints_resolve_trivially() {
        let mut e = ep("ollama", "healthy");
        assert!(e.resolve_secret());
    }

    #[test]
    fn requested_model_overrides_a_local_endpoint() {
        let mut e = ep("ollama", "healthy");
        e.honour_requested_model(Some("qwen3:4b"));
        assert_eq!(e.model, "qwen3:4b");
    }

    #[test]
    fn requested_model_does_not_override_a_remote_endpoint() {
        let mut e = ep("openai_compat", "healthy");
        e.honour_requested_model(Some("qwen3:4b"));
        assert_eq!(e.model, "m");
    }

    #[test]
    fn blank_requested_model_changes_nothing() {
        let mut e = ep("ollama", "healthy");
        e.honour_requested_model(Some("   "));
        e.honour_requested_model(None);
        assert_eq!(e.model, "m");
    }
}
