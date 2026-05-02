//! [`Verdict`] — the quantified outcome record that closes the experimental
//! loop (ADR-2605021400).
//!
//! A Verdict is what stash's binary `confirm_hypothesis` / `reject_hypothesis`
//! becomes when projected through hex: same idea, but carries the measured
//! delta and a confidence, plus an explicit graduate / hold / rollback /
//! inconclusive decision.

use super::hypothesis::HypothesisId;
use super::objective::ObjectiveId;
use serde::{Deserialize, Serialize};

/// Newtype identifier for a [`Verdict`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VerdictId(pub String);

/// What action follows from this verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum VerdictDecision {
    /// Promote the trial variant to canonical.
    Graduate,
    /// Keep the trial variant running until a future review point.
    Hold {
        /// ISO 8601 timestamp at which the verdict should be re-evaluated.
        until: String,
    },
    /// Revert; the trial variant did not improve the objective.
    Rollback {
        reason: String,
    },
    /// Insufficient signal to decide.
    Inconclusive,
}

/// The recorded outcome of a [`super::hypothesis::Hypothesis`] tested under a
/// trial variant. Computing `delta` and `confidence` is the responsibility of
/// the future `VerdictPolicy` port (ADR-2605021400 §Implementation P7), not
/// of this domain type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub id: VerdictId,
    /// Identifier of the trial this verdict is for.
    ///
    /// **Phase 1 stub:** carried as `String` until the `Trial` and `TrialId`
    /// types land in `wp-experiment-loop-p2`. At that point this field
    /// becomes `TrialId`. See ADR-2605021400 §Implementation P5.
    pub trial_id: String,
    pub hypothesis_id: HypothesisId,
    pub objective_id: ObjectiveId,
    /// Measured score for the baseline (pre-change) variant.
    pub baseline_score: f64,
    /// Measured score for the trial variant.
    pub trial_score: f64,
    /// `trial_score - baseline_score`. Stored rather than recomputed so a
    /// projection to/from stash retains the recorded value verbatim.
    pub delta: f64,
    /// Statistical confidence in the delta, on [0.0, 1.0].
    pub confidence: f64,
    pub decision: VerdictDecision,
    /// ISO 8601 timestamp when the verdict was recorded.
    pub archived_at: String,
    #[serde(default)]
    pub notes: String,
}
