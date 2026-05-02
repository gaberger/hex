//! In-memory adapter implementing [`IExperimentPort`].
//!
//! Reference impl backing tests, the future CLI / REST surface, and the pilot
//! example app. Holds state in `tokio::sync::RwLock<HashMap<...>>`. The
//! `SpacetimeExperimentAdapter` (wp-experiment-loop-p3b) diffs its behavior
//! against this adapter in its integration test.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use hex_core::domain::experiment::{
    ExperimentError, Hypothesis, HypothesisId, HypothesisStatus, Objective, ObjectiveId,
    ObjectiveStatus, Verdict, VerdictId,
};
use hex_core::ports::experiment::IExperimentPort;

#[derive(Default)]
struct State {
    objectives: HashMap<ObjectiveId, (String /* project_id */, Objective)>,
    hypotheses: HashMap<HypothesisId, (String, Hypothesis)>,
    verdicts: HashMap<VerdictId, (String, Verdict)>,
}

/// In-memory `IExperimentPort` impl.
#[derive(Default, Clone)]
pub struct InMemoryExperimentAdapter {
    state: Arc<RwLock<State>>,
}

impl InMemoryExperimentAdapter {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl IExperimentPort for InMemoryExperimentAdapter {
    // ── Objectives ─────────────────────────────────────────
    async fn objective_create(
        &self,
        project_id: &str,
        obj: Objective,
    ) -> Result<ObjectiveId, ExperimentError> {
        let mut state = self.state.write().await;
        if let Some(parent) = &obj.parent {
            if !state.objectives.contains_key(parent) {
                return Err(ExperimentError::ObjectiveNotFound(parent.clone()));
            }
        }
        let id = obj.id.clone();
        state.objectives.insert(id.clone(), (project_id.to_string(), obj));
        Ok(id)
    }

    async fn objective_get(
        &self,
        id: &ObjectiveId,
    ) -> Result<Option<Objective>, ExperimentError> {
        Ok(self.state.read().await.objectives.get(id).map(|(_, o)| o.clone()))
    }

    async fn objective_list(
        &self,
        project_id: &str,
    ) -> Result<Vec<Objective>, ExperimentError> {
        let state = self.state.read().await;
        let mut out: Vec<Objective> = state
            .objectives
            .values()
            .filter(|(pid, _)| pid == project_id)
            .map(|(_, o)| o.clone())
            .collect();
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(out)
    }

    async fn objective_update_status(
        &self,
        id: &ObjectiveId,
        status: ObjectiveStatus,
    ) -> Result<(), ExperimentError> {
        let mut state = self.state.write().await;
        let (_, obj) = state
            .objectives
            .get_mut(id)
            .ok_or_else(|| ExperimentError::ObjectiveNotFound(id.clone()))?;
        obj.status = status;
        Ok(())
    }

    // ── Hypotheses ─────────────────────────────────────────
    async fn hypothesis_create(
        &self,
        project_id: &str,
        h: Hypothesis,
    ) -> Result<HypothesisId, ExperimentError> {
        let mut state = self.state.write().await;
        if !state.objectives.contains_key(&h.target_objective) {
            return Err(ExperimentError::ObjectiveNotFound(h.target_objective.clone()));
        }
        let id = h.id.clone();
        state.hypotheses.insert(id.clone(), (project_id.to_string(), h));
        Ok(id)
    }

    async fn hypothesis_get(
        &self,
        id: &HypothesisId,
    ) -> Result<Option<Hypothesis>, ExperimentError> {
        Ok(self.state.read().await.hypotheses.get(id).map(|(_, h)| h.clone()))
    }

    async fn hypothesis_list_for_objective(
        &self,
        target: &ObjectiveId,
    ) -> Result<Vec<Hypothesis>, ExperimentError> {
        let state = self.state.read().await;
        let mut out: Vec<Hypothesis> = state
            .hypotheses
            .values()
            .filter(|(_, h)| &h.target_objective == target)
            .map(|(_, h)| h.clone())
            .collect();
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(out)
    }

    async fn hypothesis_update_status(
        &self,
        id: &HypothesisId,
        status: HypothesisStatus,
    ) -> Result<(), ExperimentError> {
        let mut state = self.state.write().await;
        let (_, h) = state
            .hypotheses
            .get_mut(id)
            .ok_or_else(|| ExperimentError::HypothesisNotFound(id.clone()))?;
        h.status = status;
        Ok(())
    }

    // ── Verdicts ───────────────────────────────────────────
    async fn verdict_record(
        &self,
        project_id: &str,
        v: Verdict,
    ) -> Result<VerdictId, ExperimentError> {
        let mut state = self.state.write().await;
        if !state.hypotheses.contains_key(&v.hypothesis_id) {
            return Err(ExperimentError::HypothesisNotFound(v.hypothesis_id.clone()));
        }
        if !state.objectives.contains_key(&v.objective_id) {
            return Err(ExperimentError::ObjectiveNotFound(v.objective_id.clone()));
        }
        let id = v.id.clone();
        state.verdicts.insert(id.clone(), (project_id.to_string(), v));
        Ok(id)
    }

    async fn verdict_get(
        &self,
        id: &VerdictId,
    ) -> Result<Option<Verdict>, ExperimentError> {
        Ok(self.state.read().await.verdicts.get(id).map(|(_, v)| v.clone()))
    }

    async fn verdict_list_for_objective(
        &self,
        obj: &ObjectiveId,
    ) -> Result<Vec<Verdict>, ExperimentError> {
        let state = self.state.read().await;
        let mut out: Vec<Verdict> = state
            .verdicts
            .values()
            .filter(|(_, v)| &v.objective_id == obj)
            .map(|(_, v)| v.clone())
            .collect();
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::domain::experiment::{
        ComparisonOperator, ObjectivePriority, VerdictDecision,
    };

    fn fresh_objective(id: &str) -> Objective {
        Objective {
            id: ObjectiveId(id.into()),
            name: format!("objective {id}"),
            description: String::new(),
            parent: None,
            priority: ObjectivePriority::Medium,
            target_value: 100.0,
            comparison: ComparisonOperator::LessThan,
            unit: "ms".into(),
            status: ObjectiveStatus::Active,
            created_at: "2026-05-02T00:00:00Z".into(),
            updated_at: "2026-05-02T00:00:00Z".into(),
        }
    }

    fn fresh_hypothesis(id: &str, target: &str) -> Hypothesis {
        Hypothesis {
            id: HypothesisId(id.into()),
            content: format!("hypothesis {id}"),
            target_objective: ObjectiveId(target.into()),
            predicted_delta: -10.0,
            predicted_confidence: 0.7,
            verification_plan: "1-week A/B".into(),
            adr_id: None,
            status: HypothesisStatus::Untested,
            created_at: "2026-05-02T00:00:00Z".into(),
        }
    }

    fn fresh_verdict(id: &str, hyp: &str, obj: &str) -> Verdict {
        Verdict {
            id: VerdictId(id.into()),
            trial_id: format!("trial-{id}"),
            hypothesis_id: HypothesisId(hyp.into()),
            objective_id: ObjectiveId(obj.into()),
            baseline_score: 130.0,
            trial_score: 102.0,
            delta: -28.0,
            confidence: 0.81,
            decision: VerdictDecision::Graduate,
            archived_at: "2026-05-09T00:00:00Z".into(),
            notes: String::new(),
        }
    }

    #[tokio::test]
    async fn create_then_get_round_trip() {
        let a = InMemoryExperimentAdapter::new();
        let obj = fresh_objective("o1");
        a.objective_create("p1", obj.clone()).await.unwrap();
        let got = a.objective_get(&ObjectiveId("o1".into())).await.unwrap();
        assert_eq!(got, Some(obj));
    }

    #[tokio::test]
    async fn hypothesis_create_validates_objective_fk() {
        let a = InMemoryExperimentAdapter::new();
        let h = fresh_hypothesis("h1", "missing");
        let err = a.hypothesis_create("p1", h).await.unwrap_err();
        assert!(matches!(err, ExperimentError::ObjectiveNotFound(_)));
    }

    #[tokio::test]
    async fn verdict_record_validates_hypothesis_fk() {
        let a = InMemoryExperimentAdapter::new();
        let obj = fresh_objective("o1");
        a.objective_create("p1", obj).await.unwrap();
        let v = fresh_verdict("v1", "missing", "o1");
        let err = a.verdict_record("p1", v).await.unwrap_err();
        assert!(matches!(err, ExperimentError::HypothesisNotFound(_)));
    }

    #[tokio::test]
    async fn hypothesis_list_for_objective_filters() {
        let a = InMemoryExperimentAdapter::new();
        a.objective_create("p1", fresh_objective("o1")).await.unwrap();
        a.objective_create("p1", fresh_objective("o2")).await.unwrap();
        a.hypothesis_create("p1", fresh_hypothesis("h1", "o1")).await.unwrap();
        a.hypothesis_create("p1", fresh_hypothesis("h2", "o1")).await.unwrap();
        a.hypothesis_create("p1", fresh_hypothesis("h3", "o2")).await.unwrap();

        let for_o1 = a
            .hypothesis_list_for_objective(&ObjectiveId("o1".into()))
            .await
            .unwrap();
        assert_eq!(for_o1.len(), 2);
        assert_eq!(for_o1[0].id.0, "h1");
        assert_eq!(for_o1[1].id.0, "h2");
    }

    #[tokio::test]
    async fn update_status_returns_not_found_on_unknown_id() {
        let a = InMemoryExperimentAdapter::new();
        let err = a
            .objective_update_status(&ObjectiveId("missing".into()), ObjectiveStatus::Achieved)
            .await
            .unwrap_err();
        assert!(matches!(err, ExperimentError::ObjectiveNotFound(_)));
    }

    #[tokio::test]
    async fn objective_list_returns_only_matching_project() {
        let a = InMemoryExperimentAdapter::new();
        a.objective_create("p1", fresh_objective("a")).await.unwrap();
        a.objective_create("p2", fresh_objective("b")).await.unwrap();
        let p1 = a.objective_list("p1").await.unwrap();
        assert_eq!(p1.len(), 1);
        assert_eq!(p1[0].id.0, "a");
    }
}
