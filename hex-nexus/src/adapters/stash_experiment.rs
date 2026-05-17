//! Stash projection decorator over [`IExperimentPort`].
//!
//! Wraps an inner `IExperimentPort` (typically [`super::in_memory_experiment::InMemoryExperimentAdapter`]
//! today, [`super::spacetime_experiment::SpacetimeExperimentAdapter`] tomorrow)
//! plus an [`IConsolidationMemoryPort`]. Writes go to inner first; on success
//! they are mirrored best-effort into stash so the consolidation pipeline
//! (ADR-2604261430) can reason about hypotheses, objectives, and rollback
//! failures.
//!
//! Projection semantics:
//!
//! | Inner write              | Stash projection          |
//! |--------------------------|---------------------------|
//! | objective_create         | remember(name, namespace) |
//! | hypothesis_create        | remember(content, namespace) |
//! | verdict_record(rollback) | remember(notes+reason, namespace) |
//!
//! `IConsolidationMemoryPort` does not have native `create_goal` or
//! `create_hypothesis` methods (those land in P8 of ADR-2604261430). Until
//! then, projection uses [`IConsolidationMemoryPort::remember`] with a
//! convention-bound namespace per record kind. When the richer surface lands,
//! this adapter swaps the calls without changing the trait shape.
//!
//! Projection failures are logged via `tracing::warn!` and never surfaced —
//! the inner write has already succeeded; failing the call would lose data.

use std::sync::Arc;

use async_trait::async_trait;

use hex_core::domain::experiment::{
    ExperimentError, Hypothesis, HypothesisId, HypothesisStatus, Objective, ObjectiveId,
    ObjectiveStatus, Verdict, VerdictDecision, VerdictId,
};
use hex_core::ports::consolidation_memory::IConsolidationMemoryPort;
use hex_core::ports::experiment::IExperimentPort;

/// Decorator that projects writes into a [`IConsolidationMemoryPort`].
pub struct StashExperimentAdapter<Inner> {
    inner: Inner,
    stash: Arc<dyn IConsolidationMemoryPort>,
    enabled: bool,
}

impl<Inner> StashExperimentAdapter<Inner> {
    pub fn new(inner: Inner, stash: Arc<dyn IConsolidationMemoryPort>, enabled: bool) -> Self {
        Self { inner, stash, enabled }
    }
}

fn objective_namespace(project_id: &str) -> String {
    format!("hex/{project_id}/objectives")
}
fn hypothesis_namespace(project_id: &str) -> String {
    format!("hex/{project_id}/hypotheses")
}
fn failure_namespace(project_id: &str) -> String {
    format!("hex/{project_id}/failures")
}

#[async_trait]
impl<Inner> IExperimentPort for StashExperimentAdapter<Inner>
where
    Inner: IExperimentPort + Send + Sync,
{
    // ── Writes — delegate to inner, then project best-effort ──────────────
    async fn objective_create(
        &self,
        project_id: &str,
        obj: Objective,
    ) -> Result<ObjectiveId, ExperimentError> {
        let id = self.inner.objective_create(project_id, obj.clone()).await?;
        if self.enabled {
            let payload = format!("{}: {} (target={} {})", obj.name, obj.description, obj.target_value, obj.unit);
            if let Err(e) = self
                .stash
                .remember(&payload, &objective_namespace(project_id))
                .await
            {
                tracing::warn!(
                    objective_id = %id.0,
                    project_id,
                    error = ?e,
                    "stash projection failed for objective_create",
                );
            }
        }
        Ok(id)
    }

    async fn hypothesis_create(
        &self,
        project_id: &str,
        h: Hypothesis,
    ) -> Result<HypothesisId, ExperimentError> {
        let id = self.inner.hypothesis_create(project_id, h.clone()).await?;
        if self.enabled {
            let payload = format!(
                "{}\n\nverification_plan: {}",
                h.content, h.verification_plan
            );
            if let Err(e) = self
                .stash
                .remember(&payload, &hypothesis_namespace(project_id))
                .await
            {
                tracing::warn!(
                    hypothesis_id = %id.0,
                    project_id,
                    error = ?e,
                    "stash projection failed for hypothesis_create",
                );
            }
        }
        Ok(id)
    }

    async fn verdict_record(
        &self,
        project_id: &str,
        v: Verdict,
    ) -> Result<VerdictId, ExperimentError> {
        let id = self.inner.verdict_record(project_id, v.clone()).await?;
        if self.enabled {
            // Only project rollbacks — those are the failures stash's
            // pipeline tracks. Graduate/Hold/Inconclusive don't carry
            // independent learning signal in the consolidation pipeline yet.
            if let VerdictDecision::Rollback { reason } = &v.decision {
                let payload = if v.notes.is_empty() {
                    format!("rollback: {reason}")
                } else {
                    format!("{}\n\nrollback reason: {}", v.notes, reason)
                };
                if let Err(e) = self
                    .stash
                    .remember(&payload, &failure_namespace(project_id))
                    .await
                {
                    tracing::warn!(
                        verdict_id = %id.0,
                        project_id,
                        error = ?e,
                        "stash projection failed for verdict_record",
                    );
                }
            }
        }
        Ok(id)
    }

    async fn objective_update_status(
        &self,
        id: &ObjectiveId,
        status: ObjectiveStatus,
    ) -> Result<(), ExperimentError> {
        // Status transitions are not projected today — they're inferable from
        // the underlying objective record. If P8 of ADR-2604261430 introduces
        // a typed goal-status surface in the consolidation port, swap to it
        // here.
        self.inner.objective_update_status(id, status).await
    }

    async fn hypothesis_update_status(
        &self,
        id: &HypothesisId,
        status: HypothesisStatus,
    ) -> Result<(), ExperimentError> {
        self.inner.hypothesis_update_status(id, status).await
    }

    // ── Reads — pure delegation; stash is not source of truth for queries ──
    async fn objective_get(
        &self,
        id: &ObjectiveId,
    ) -> Result<Option<Objective>, ExperimentError> {
        self.inner.objective_get(id).await
    }

    async fn objective_list(
        &self,
        project_id: &str,
    ) -> Result<Vec<Objective>, ExperimentError> {
        self.inner.objective_list(project_id).await
    }

    async fn hypothesis_get(
        &self,
        id: &HypothesisId,
    ) -> Result<Option<Hypothesis>, ExperimentError> {
        self.inner.hypothesis_get(id).await
    }

    async fn hypothesis_list_for_objective(
        &self,
        target: &ObjectiveId,
    ) -> Result<Vec<Hypothesis>, ExperimentError> {
        self.inner.hypothesis_list_for_objective(target).await
    }

    async fn verdict_get(
        &self,
        id: &VerdictId,
    ) -> Result<Option<Verdict>, ExperimentError> {
        self.inner.verdict_get(id).await
    }

    async fn verdict_list_for_objective(
        &self,
        obj: &ObjectiveId,
    ) -> Result<Vec<Verdict>, ExperimentError> {
        self.inner.verdict_list_for_objective(obj).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::in_memory_experiment::InMemoryExperimentAdapter;
    use hex_core::domain::consolidation::{
        CausalChain, CausalDirection, ConsolidationError, ConsolidationReport, Contradiction,
        Episode, EpisodeId, Fact, Relationship,
    };
    use hex_core::domain::experiment::{
        ComparisonOperator, ObjectivePriority, VerdictDecision,
    };
    use std::sync::Mutex;

    /// Records every method call as a string for assertion. Returns Ok by
    /// default; flip `fail_remember` to make `remember` return Err.
    #[derive(Default)]
    struct MockConsolidation {
        calls: Mutex<Vec<String>>,
        fail_remember: bool,
    }

    #[async_trait]
    impl IConsolidationMemoryPort for MockConsolidation {
        async fn remember(
            &self,
            content: &str,
            namespace: &str,
        ) -> Result<EpisodeId, ConsolidationError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("remember:{namespace}:{content}"));
            if self.fail_remember {
                Err(ConsolidationError::Backend("mock failure".into()))
            } else {
                Ok(EpisodeId("e-mock".into()))
            }
        }
        async fn recall(
            &self,
            _q: &str,
            _ns: &[String],
            _l: u32,
        ) -> Result<Vec<Episode>, ConsolidationError> {
            self.calls.lock().unwrap().push("recall".into());
            Ok(vec![])
        }
        async fn forget(
            &self,
            _: &str,
            _: &[String],
        ) -> Result<u32, ConsolidationError> {
            Ok(0)
        }
        async fn consolidate(
            &self,
            _: &[String],
        ) -> Result<ConsolidationReport, ConsolidationError> {
            unreachable!()
        }
        async fn query_facts(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<Fact>, ConsolidationError> {
            Ok(vec![])
        }
        async fn query_relationships(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<Relationship>, ConsolidationError> {
            Ok(vec![])
        }
        async fn list_contradictions(
            &self,
            _: &str,
        ) -> Result<Vec<Contradiction>, ConsolidationError> {
            Ok(vec![])
        }
        async fn resolve_contradiction(
            &self,
            _: &str,
            _: &str,
        ) -> Result<(), ConsolidationError> {
            Ok(())
        }
        async fn trace_causal_chain(
            &self,
            _: &str,
            _: CausalDirection,
            _: u8,
        ) -> Result<CausalChain, ConsolidationError> {
            unreachable!()
        }
    }

    fn obj(id: &str) -> Objective {
        Objective {
            id: ObjectiveId(id.into()),
            name: format!("obj-{id}"),
            description: "desc".into(),
            parent: None,
            priority: ObjectivePriority::Medium,
            target_value: 100.0,
            comparison: ComparisonOperator::LessThan,
            unit: "ms".into(),
            status: ObjectiveStatus::Active,
            created_at: "t".into(),
            updated_at: "t".into(),
        }
    }

    fn hyp(id: &str, target: &str) -> Hypothesis {
        Hypothesis {
            id: HypothesisId(id.into()),
            content: format!("hyp-{id}"),
            target_objective: ObjectiveId(target.into()),
            predicted_delta: -10.0,
            predicted_confidence: 0.7,
            verification_plan: "plan".into(),
            adr_id: None,
            status: HypothesisStatus::Untested,
            created_at: "t".into(),
        }
    }

    fn verdict(id: &str, hyp_id: &str, obj_id: &str, decision: VerdictDecision) -> Verdict {
        Verdict {
            id: VerdictId(id.into()),
            trial_id: "trial".into(),
            hypothesis_id: HypothesisId(hyp_id.into()),
            objective_id: ObjectiveId(obj_id.into()),
            baseline_score: 130.0,
            trial_score: 102.0,
            delta: -28.0,
            confidence: 0.81,
            decision,
            archived_at: "t".into(),
            notes: "notes-body".into(),
        }
    }

    fn build_adapter(
        enabled: bool,
        fail_remember: bool,
    ) -> (StashExperimentAdapter<InMemoryExperimentAdapter>, Arc<MockConsolidation>) {
        let mock = Arc::new(MockConsolidation {
            calls: Mutex::new(vec![]),
            fail_remember,
        });
        let inner = InMemoryExperimentAdapter::new();
        let adapter = StashExperimentAdapter::new(inner, mock.clone(), enabled);
        (adapter, mock)
    }

    #[tokio::test]
    async fn projects_objective_create() {
        let (a, mock) = build_adapter(true, false);
        a.objective_create("p1", obj("o1")).await.unwrap();
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("remember:hex/p1/objectives:"));
    }

    #[tokio::test]
    async fn projects_hypothesis_create_after_objective_exists() {
        let (a, mock) = build_adapter(true, false);
        a.objective_create("p1", obj("o1")).await.unwrap();
        a.hypothesis_create("p1", hyp("h1", "o1")).await.unwrap();
        let calls = mock.calls.lock().unwrap();
        // 2 calls: one for objective_create, one for hypothesis_create
        assert_eq!(calls.len(), 2);
        assert!(calls[1].starts_with("remember:hex/p1/hypotheses:"));
        assert!(calls[1].contains("verification_plan: plan"));
    }

    #[tokio::test]
    async fn projects_rollback_verdict_only() {
        let (a, mock) = build_adapter(true, false);
        a.objective_create("p1", obj("o1")).await.unwrap();
        a.hypothesis_create("p1", hyp("h1", "o1")).await.unwrap();

        // Graduate — must NOT add a remember call.
        a.verdict_record(
            "p1",
            verdict("v1", "h1", "o1", VerdictDecision::Graduate),
        )
        .await
        .unwrap();
        let after_graduate = mock.calls.lock().unwrap().len();

        // Rollback — must add exactly one remember call into the failures namespace.
        a.verdict_record(
            "p1",
            verdict(
                "v2",
                "h1",
                "o1",
                VerdictDecision::Rollback {
                    reason: "regression".into(),
                },
            ),
        )
        .await
        .unwrap();
        let calls = mock.calls.lock().unwrap();
        assert_eq!(calls.len(), after_graduate + 1);
        assert!(calls[after_graduate].starts_with("remember:hex/p1/failures:"));
        assert!(calls[after_graduate].contains("rollback reason: regression"));
    }

    #[tokio::test]
    async fn reads_do_not_touch_stash() {
        let (a, mock) = build_adapter(true, false);
        a.objective_create("p1", obj("o1")).await.unwrap();
        let _ = a.objective_get(&ObjectiveId("o1".into())).await.unwrap();
        let _ = a.objective_list("p1").await.unwrap();
        let _ = a
            .hypothesis_list_for_objective(&ObjectiveId("o1".into()))
            .await
            .unwrap();
        let calls = mock.calls.lock().unwrap();
        // Only the objective_create projected. Reads added nothing.
        assert_eq!(calls.len(), 1);
    }

    #[tokio::test]
    async fn projection_failure_does_not_surface() {
        let (a, _mock) = build_adapter(true, /* fail_remember */ true);
        // Inner write succeeds, projection fails — caller should still see Ok.
        a.objective_create("p1", obj("o1")).await.unwrap();
    }

    #[tokio::test]
    async fn disabled_projection_is_a_noop() {
        let (a, mock) = build_adapter(/* enabled */ false, false);
        a.objective_create("p1", obj("o1")).await.unwrap();
        a.hypothesis_create("p1", hyp("h1", "o1")).await.unwrap();
        a.verdict_record(
            "p1",
            verdict(
                "v1",
                "h1",
                "o1",
                VerdictDecision::Rollback {
                    reason: "x".into(),
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(mock.calls.lock().unwrap().len(), 0);
    }
}
