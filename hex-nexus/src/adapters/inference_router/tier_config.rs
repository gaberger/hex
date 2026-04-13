//! Tier→model mapping configuration for inference routing (ADR-2604120202 P1.2).
//!
//! Each task tier maps to a model name that the inference router uses to
//! select the right server.  Defaults are baked in; per-project overrides
//! come from `.hex/project.json` → `inference.tier_models`.

use crate::remote::transport::{TaskTier, TransportError};

/// Tier→model mapping loaded by the composition root.
///
/// `t3` is `Option` because frontier models require an external provider.
/// When `None`, requesting a T3 task returns a remediation error instead
/// of silently falling back to a weaker model.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TierModelConfig {
    #[serde(alias = "T1")]
    pub t1: String,
    #[serde(alias = "T2")]
    pub t2: String,
    #[serde(alias = "T2.5", alias = "T2_5")]
    pub t2_5: String,
    #[serde(alias = "T3")]
    pub t3: Option<String>,
}

impl Default for TierModelConfig {
    fn default() -> Self {
        Self {
            t1: "qwen3:4b".into(),
            t2: "qwen2.5-coder:32b".into(),
            t2_5: "devstral-small-2:24b".into(),
            t3: None, // frontier — must be explicitly configured
        }
    }
}

impl TierModelConfig {
    /// Resolve the model name for a given tier.
    ///
    /// Returns `Err` for T3 when no frontier model is configured, with a
    /// remediation hint directing the user to configure one.
    pub fn model_for_tier(&self, tier: TaskTier) -> Result<&str, TransportError> {
        match tier {
            TaskTier::T1 => Ok(&self.t1),
            TaskTier::T2 => Ok(&self.t2),
            TaskTier::T2_5 => Ok(&self.t2_5),
            TaskTier::T3 => self.t3.as_deref().ok_or_else(|| {
                TransportError::Protocol(
                    "T3 task requires a frontier model but none is configured. \
                     Set inference.tier_models.t3 in .hex/project.json or \
                     add a cloud provider with `hex inference add --cloud`."
                        .into(),
                )
            }),
        }
    }

    /// Whether this config has a frontier (T3) model available.
    pub fn has_frontier(&self) -> bool {
        self.t3.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve_t1_through_t2_5() {
        let cfg = TierModelConfig::default();
        assert_eq!(cfg.model_for_tier(TaskTier::T1).unwrap(), "qwen3:4b");
        assert_eq!(cfg.model_for_tier(TaskTier::T2).unwrap(), "qwen2.5-coder:32b");
        assert_eq!(cfg.model_for_tier(TaskTier::T2_5).unwrap(), "devstral-small-2:24b");
    }

    #[test]
    fn t3_without_config_returns_remediation_error() {
        let cfg = TierModelConfig::default();
        let err = cfg.model_for_tier(TaskTier::T3).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("frontier model"), "error should mention frontier: {msg}");
        assert!(msg.contains("tier_models.t3"), "error should contain remediation hint: {msg}");
    }

    #[test]
    fn t3_with_config_resolves() {
        let cfg = TierModelConfig {
            t3: Some("claude".into()),
            ..Default::default()
        };
        assert_eq!(cfg.model_for_tier(TaskTier::T3).unwrap(), "claude");
    }

    #[test]
    fn has_frontier_reflects_t3_presence() {
        assert!(!TierModelConfig::default().has_frontier());
        let cfg = TierModelConfig {
            t3: Some("claude".into()),
            ..Default::default()
        };
        assert!(cfg.has_frontier());
    }

    #[test]
    fn deserialize_with_aliases() {
        let json = r#"{"T1": "tiny", "T2": "medium", "T2.5": "big", "T3": "frontier"}"#;
        let cfg: TierModelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.t1, "tiny");
        assert_eq!(cfg.t2_5, "big");
        assert_eq!(cfg.t3.as_deref(), Some("frontier"));
    }
}
