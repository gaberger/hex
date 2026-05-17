//! Experiment port — contract for the target-app experimental loop
//! (ADR-2605021400).
//!
//! Implemented by SpacetimeDB-backed adapters (P3 of ADR-2605021400) and,
//! optionally, a stash-projecting adapter that mirrors hypotheses/objectives
//! into the consolidation pipeline (ADR-2604261430).

use async_trait::async_trait;

use crate::domain::experiment::{
    ExperimentError, Hypothesis, HypothesisId, HypothesisStatus, Objective, ObjectiveId,
    ObjectiveStatus, Verdict, VerdictId,
};

/// The experiment port — exposes CRUD-like operations over Objective,
/// Hypothesis, and Verdict for a single project.
#[async_trait]
pub trait IExperimentPort: Send + Sync {
    // ── Objectives ─────────────────────────────────────────
    async fn objective_create(
        &self,
        project_id: &str,
        obj: Objective,
    ) -> Result<ObjectiveId, ExperimentError>;

    async fn objective_get(
        &self,
        id: &ObjectiveId,
    ) -> Result<Option<Objective>, ExperimentError>;

    async fn objective_list(
        &self,
        project_id: &str,
    ) -> Result<Vec<Objective>, ExperimentError>;

    async fn objective_update_status(
        &self,
        id: &ObjectiveId,
        status: ObjectiveStatus,
    ) -> Result<(), ExperimentError>;

    // ── Hypotheses ─────────────────────────────────────────
    async fn hypothesis_create(
        &self,
        project_id: &str,
        h: Hypothesis,
    ) -> Result<HypothesisId, ExperimentError>;

    async fn hypothesis_get(
        &self,
        id: &HypothesisId,
    ) -> Result<Option<Hypothesis>, ExperimentError>;

    async fn hypothesis_list_for_objective(
        &self,
        target: &ObjectiveId,
    ) -> Result<Vec<Hypothesis>, ExperimentError>;

    async fn hypothesis_update_status(
        &self,
        id: &HypothesisId,
        status: HypothesisStatus,
    ) -> Result<(), ExperimentError>;

    // ── Verdicts ───────────────────────────────────────────
    async fn verdict_record(
        &self,
        project_id: &str,
        v: Verdict,
    ) -> Result<VerdictId, ExperimentError>;

    async fn verdict_get(
        &self,
        id: &VerdictId,
    ) -> Result<Option<Verdict>, ExperimentError>;

    async fn verdict_list_for_objective(
        &self,
        obj: &ObjectiveId,
    ) -> Result<Vec<Verdict>, ExperimentError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns `ExperimentError::Backend("dummy")` from every method.
    /// Exists only to assert the trait is dyn-compatible at compile time.
    struct DummyExperimentAdapter;

    #[async_trait]
    impl IExperimentPort for DummyExperimentAdapter {
        async fn objective_create(
            &self,
            _project_id: &str,
            _obj: Objective,
        ) -> Result<ObjectiveId, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn objective_get(
            &self,
            _id: &ObjectiveId,
        ) -> Result<Option<Objective>, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn objective_list(
            &self,
            _project_id: &str,
        ) -> Result<Vec<Objective>, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn objective_update_status(
            &self,
            _id: &ObjectiveId,
            _status: ObjectiveStatus,
        ) -> Result<(), ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn hypothesis_create(
            &self,
            _project_id: &str,
            _h: Hypothesis,
        ) -> Result<HypothesisId, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn hypothesis_get(
            &self,
            _id: &HypothesisId,
        ) -> Result<Option<Hypothesis>, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn hypothesis_list_for_objective(
            &self,
            _target: &ObjectiveId,
        ) -> Result<Vec<Hypothesis>, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn hypothesis_update_status(
            &self,
            _id: &HypothesisId,
            _status: HypothesisStatus,
        ) -> Result<(), ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn verdict_record(
            &self,
            _project_id: &str,
            _v: Verdict,
        ) -> Result<VerdictId, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn verdict_get(
            &self,
            _id: &VerdictId,
        ) -> Result<Option<Verdict>, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
        async fn verdict_list_for_objective(
            &self,
            _obj: &ObjectiveId,
        ) -> Result<Vec<Verdict>, ExperimentError> {
            Err(ExperimentError::Backend("dummy".into()))
        }
    }

    #[test]
    fn experiment_port_is_dyn_safe() {
        let adapter = DummyExperimentAdapter;
        let _erased: &dyn IExperimentPort = &adapter;
    }
}
