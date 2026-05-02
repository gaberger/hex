//! [`Hypothesis`] — a falsifiable predicted effect on a target [`super::objective::Objective`]
//! (ADR-2605021400).
//!
//! Field set kept deliberately minimal so the future `StashExperimentAdapter`
//! (ADR-2605021400 §Implementation P3) can projection-map cleanly to stash's
//! native `hypothesis` shape.

use super::objective::ObjectiveId;
use serde::{Deserialize, Serialize};

/// Newtype identifier for a [`Hypothesis`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HypothesisId(pub String);

/// Lifecycle state of a hypothesis.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum HypothesisStatus {
    #[default]
    Untested,
    Confirmed {
        /// ISO 8601 timestamp when the hypothesis was confirmed.
        confirmed_at: String,
    },
    Rejected {
        /// ISO 8601 timestamp when the hypothesis was rejected.
        rejected_at: String,
        reason: String,
    },
    Inconclusive {
        /// ISO 8601 timestamp when the hypothesis was reviewed.
        reviewed_at: String,
    },
}

/// A falsifiable claim that some change will move a target [`super::objective::Objective`]
/// by `predicted_delta` with confidence `predicted_confidence`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: HypothesisId,
    pub content: String,
    pub target_objective: ObjectiveId,
    /// Signed predicted change in the objective's score (sign matches whether
    /// the comparison operator is "greater than" / "less than"; e.g. negative
    /// delta is good for `LessThan` latency objectives).
    pub predicted_delta: f64,
    /// Predicted confidence in the delta, on [0.0, 1.0].
    pub predicted_confidence: f64,
    /// Free-text plan describing how the hypothesis will be tested
    /// (workload, duration, statistical method).
    pub verification_plan: String,
    /// ID of the ADR that this hypothesis attaches to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adr_id: Option<String>,
    #[serde(default)]
    pub status: HypothesisStatus,
    /// ISO 8601 timestamp when this hypothesis was created.
    pub created_at: String,
}
