//! Target-app experimental loop representations (ADR-2605021400).
//!
//! These types support the self-reorganizing-app loop:
//!
//! 1. An [`Objective`] describes what a target app is trying to maximize/minimize.
//! 2. A [`Hypothesis`] predicts that some change will move an Objective by Δ.
//! 3. A [`Verdict`] records the measured outcome and the graduate/rollback decision.
//!
//! This is **Phase 1** (the loop-closing trio). Persona / Workload / Trial /
//! Failure land in **Phase 2** per ADR-2605021400 §Implementation P5–P8.

pub mod hypothesis;
pub mod objective;
pub mod verdict;

pub use hypothesis::{Hypothesis, HypothesisId, HypothesisStatus};
pub use objective::{
    ComparisonOperator, Objective, ObjectiveId, ObjectivePriority, ObjectiveStatus,
};
pub use verdict::{Verdict, VerdictDecision, VerdictId};

use thiserror::Error;

/// Errors surfaced by the experiment domain and the future `IExperimentPort`
/// (ADR-2605021400 §Implementation P2).
#[derive(Debug, Error)]
pub enum ExperimentError {
    #[error("objective not found: {0:?}")]
    ObjectiveNotFound(ObjectiveId),

    #[error("hypothesis not found: {0:?}")]
    HypothesisNotFound(HypothesisId),

    #[error("verdict {0:?} is already archived")]
    VerdictAlreadyArchived(VerdictId),

    #[error("invalid predicted delta {value}: {reason}")]
    InvalidPredictedDelta { value: f64, reason: String },

    #[error("backend error: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serde() {
        let obj = Objective {
            id: ObjectiveId("obj-1".into()),
            name: "P95 chat latency".into(),
            description: "End-to-end p95 latency for the chat endpoint".into(),
            parent: None,
            priority: ObjectivePriority::High,
            target_value: 100.0,
            comparison: ComparisonOperator::LessThan,
            unit: "ms".into(),
            status: ObjectiveStatus::Active,
            created_at: "2026-05-02T14:30:00Z".into(),
            updated_at: "2026-05-02T14:30:00Z".into(),
        };
        let json = serde_json::to_string(&obj).expect("serialize objective");
        let back: Objective = serde_json::from_str(&json).expect("deserialize objective");
        assert_eq!(obj, back);

        let hyp = Hypothesis {
            id: HypothesisId("hyp-1".into()),
            content: "Adding a CDN reduces p95 latency".into(),
            target_objective: obj.id.clone(),
            predicted_delta: -25.0,
            predicted_confidence: 0.7,
            verification_plan: "1 week A/B at 10% traffic".into(),
            adr_id: Some("ADR-9999999999".into()),
            status: HypothesisStatus::Untested,
            created_at: "2026-05-02T14:30:00Z".into(),
        };
        let json = serde_json::to_string(&hyp).expect("serialize hypothesis");
        let back: Hypothesis = serde_json::from_str(&json).expect("deserialize hypothesis");
        assert_eq!(hyp, back);

        let verdict = Verdict {
            id: VerdictId("v-1".into()),
            trial_id: "trial-stub-1".into(),
            hypothesis_id: hyp.id.clone(),
            objective_id: obj.id.clone(),
            baseline_score: 130.0,
            trial_score: 102.0,
            delta: -28.0,
            confidence: 0.81,
            decision: VerdictDecision::Graduate,
            archived_at: "2026-05-09T14:30:00Z".into(),
            notes: "Welch's t-test, p=0.02".into(),
        };
        let json = serde_json::to_string(&verdict).expect("serialize verdict");
        let back: Verdict = serde_json::from_str(&json).expect("deserialize verdict");
        assert_eq!(verdict, back);
    }

    #[test]
    fn ids_are_distinct_types() {
        // Compile-time guard: the three Id newtypes are NOT interchangeable.
        let _o: ObjectiveId = ObjectiveId("x".into());
        let _h: HypothesisId = HypothesisId("x".into());
        let _v: VerdictId = VerdictId("x".into());
        // The following lines, if uncommented, MUST fail to compile. They are
        // kept as documentation, not as live tests:
        //   let _: ObjectiveId = HypothesisId("x".into());
        //   let _: HypothesisId = VerdictId("x".into());
    }

    #[test]
    fn hypothesis_status_default_is_untested() {
        assert!(matches!(HypothesisStatus::default(), HypothesisStatus::Untested));
    }
}
