//! Structured errors emitted by the orchestration layer.
//!
//! Replaces string-shaped errors in the workplan executor (and the
//! composition root) with typed variants that carry remediation hints.
//! See ADR-2604112000 and wp-hex-standalone-dispatch P2.2 for the
//! decision record.

use thiserror::Error;

/// One prerequisite is missing from the composition root, preventing the
/// executor from dispatching agents.
///
/// Each variant names exactly which piece is absent. The `remediation`
/// method returns a one-line operator hint so the executor's phase error
/// path can surface an actionable message instead of a generic "not
/// initialized" string.
#[derive(Debug, Error)]
pub enum MissingComposition {
    /// No inference adapter wired at the composition root. Both variants
    /// (standalone and Claude-integrated) require one.
    #[error("no inference adapter available for composition root: {reason}")]
    InferenceAdapter { reason: String },

    /// The HexFlo coordination layer is not reachable. Typically this means
    /// SpacetimeDB is down or the state port's health probe is failing.
    #[error("HexFlo coordination layer is not reachable: {reason}")]
    HexFloUnreachable { reason: String },

    /// Some other required port is missing from the composition root.
    /// Used as a catch-all for future port additions without forcing a
    /// variant explosion.
    #[error("composition root is incomplete: {details}")]
    IncompletePortWiring { details: String },
}

impl MissingComposition {
    /// One-line remediation hint for the operator. Shown next to the error
    /// message in executor logs and in the `hex doctor composition` output.
    pub fn remediation(&self) -> &'static str {
        match self {
            Self::InferenceAdapter { .. } => {
                "check OLLAMA_HOST / HEX_INFERENCE_URL and ensure the inference provider is reachable; run `hex inference list` to confirm the nexus has a registered adapter"
            }
            Self::HexFloUnreachable { .. } => {
                "check that SpacetimeDB is running (`hex nexus status`) and that the state port is published for the current host"
            }
            Self::IncompletePortWiring { .. } => {
                "run `hex doctor composition` to list which ports are unwired at the nexus composition root"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_adapter_display_mentions_reason() {
        let err = MissingComposition::InferenceAdapter {
            reason: "ollama not reachable".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("inference"));
        assert!(msg.contains("ollama not reachable"));
    }

    #[test]
    fn remediation_differs_per_variant() {
        let inf = MissingComposition::InferenceAdapter {
            reason: "x".to_string(),
        };
        let hex = MissingComposition::HexFloUnreachable {
            reason: "y".to_string(),
        };
        let wire = MissingComposition::IncompletePortWiring {
            details: "z".to_string(),
        };
        assert_ne!(inf.remediation(), hex.remediation());
        assert_ne!(hex.remediation(), wire.remediation());
        assert_ne!(inf.remediation(), wire.remediation());
    }
}
