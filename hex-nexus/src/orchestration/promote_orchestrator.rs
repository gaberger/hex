//! PromoteOrchestrator — closes the loop from PromotionJudge → live
//! binding flip (ADR-2026-04-26-1500 P6, wp-substrate-inference-consumer-rewires
//! P5).
//!
//! On each `tick()` the orchestrator:
//! 1. Reads STDB swap_tickets that are eligible for promotion. Today the
//!    `ISwapTicketStatePort.shadow_tickets_due` query only returns
//!    state="shadow"; we extend with a small client-side fetch via the
//!    same SQL endpoint as the judge uses (workplan follow-up: a dedicated
//!    `shadow_green_tickets` read method on the port).
//! 2. For each shadow_green ticket: looks up the candidate's typed
//!    `IInferencePort` handle from the `ShadowRouter` (registered when
//!    the swap was proposed). Skips with a warn if the handle is missing
//!    — next tick retries.
//! 3. Mirrors the STDB verdict into the in-memory live registry:
//!    `in_memory.stage_handle()` + `in_memory.mark_shadow_green()`.
//! 4. Calls `comp.promote_async()`. STDB transitions shadow_green→promoted;
//!    in-memory live binding flips from incumbent to candidate.
//! 5. Calls `router.end_shadow(port)` to stop mirroring.
//!
//! Idempotent — a re-tick on a promoted ticket finds nothing in
//! shadow_green and is a no-op. STDB state-machine rejection is also a
//! no-op (orchestrator catches, logs, moves on).

use std::any::Any;
use std::sync::Arc;

use hex_core::composition::{AdapterId, PortId};
use uuid::Uuid;

use crate::adapters::spacetime_composition::{AsyncRuntimeComposition, SpacetimeRuntimeComposition};
use crate::orchestration::adr_conformance::AdrConformanceChecker;
use crate::orchestration::shadow_router::ShadowRouter;
use crate::ports::state::{ISwapTicketStatePort, SwapTicketRecord};

pub struct PromoteOrchestrator {
    state: Arc<dyn ISwapTicketStatePort>,
    comp: Arc<SpacetimeRuntimeComposition>,
    router: Arc<ShadowRouter>,
    /// L5 ADR conformance checker — gates promotion against Accepted-ADR
    /// rules. None for tests that don't care about L5; production wires
    /// it from `state.adrs_dir` (default `docs/adrs/`).
    conformance: Option<AdrConformanceChecker>,
}

#[derive(Debug, Default)]
pub struct PromoteTickReport {
    pub considered: usize,
    pub promoted: Vec<String>,
    pub skipped_missing_handle: Vec<String>,
    pub blocked_by_l5: Vec<(String, String)>, // (ticket_id, joined violations)
    pub errors: Vec<(String, String)>,
}

impl PromoteOrchestrator {
    pub fn new(
        state: Arc<dyn ISwapTicketStatePort>,
        comp: Arc<SpacetimeRuntimeComposition>,
        router: Arc<ShadowRouter>,
    ) -> Self {
        Self {
            state,
            comp,
            router,
            conformance: None,
        }
    }

    /// Attach an L5 ADR conformance checker. Production sched_service
    /// wiring constructs this against `docs/adrs/` so promotions are
    /// gated by Accepted-ADR rules. Tests can omit it for unit-scope
    /// promote-flow exercises.
    pub fn with_conformance(mut self, checker: AdrConformanceChecker) -> Self {
        self.conformance = Some(checker);
        self
    }

    async fn fetch_shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, String> {
        self.state.shadow_green_tickets().await.map_err(|e| e.to_string())
    }

    pub async fn tick(&self) -> PromoteTickReport {
        let mut report = PromoteTickReport::default();
        let tickets = match self.fetch_shadow_green_tickets().await {
            Ok(t) => t,
            Err(e) => {
                report.errors.push(("<list>".into(), e));
                return report;
            }
        };
        report.considered = tickets.len();
        for ticket in tickets {
            self.promote_one(&ticket, &mut report).await;
        }
        report
    }

    /// Test-seam variant: orchestrator promotes a pre-supplied list of
    /// tickets directly, bypassing the read-side surface. Production
    /// `tick()` uses this once the typed shadow_green read method is in
    /// place. Today this is the path the unit tests exercise.
    pub async fn tick_with_tickets(
        &self,
        tickets: Vec<SwapTicketRecord>,
    ) -> PromoteTickReport {
        let mut report = PromoteTickReport::default();
        report.considered = tickets.len();
        for ticket in tickets {
            self.promote_one(&ticket, &mut report).await;
        }
        report
    }

    async fn promote_one(&self, ticket: &SwapTicketRecord, report: &mut PromoteTickReport) {
        if ticket.state != "shadow_green" {
            return;
        }
        let ticket_uuid = match Uuid::parse_str(&ticket.id) {
            Ok(u) => u,
            Err(e) => {
                report.errors.push((ticket.id.clone(), format!("bad uuid: {}", e)));
                return;
            }
        };

        let candidate_id = AdapterId::new(&ticket.candidate_adapter_id);
        let port = PortId::new(&ticket.port_id);
        // Idempotency guard: if the live binding already matches the
        // candidate, a previous tick already promoted this ticket (or an
        // operator did manually). No-op.
        if self.comp.binding_id(&port) == Some(candidate_id.clone()) {
            return;
        }

        // L5 ADR conformance gate (ADR-2026-04-26-1311 L5 / ADR-2026-04-26-1500 C6).
        // If a checker is wired and the swap violates an Accepted-ADR
        // rule, skip — leave the ticket in shadow_green so the operator
        // can either retract it or extend the relevant ADR. Idempotent
        // (re-tick re-evaluates).
        if let Some(checker) = &self.conformance {
            let violations = checker.check_promotion(ticket);
            if !violations.is_empty() {
                let joined = violations
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" | ");
                tracing::warn!(
                    ticket = %ticket.id,
                    violations = %joined,
                    "L5: promotion blocked by ADR conformance"
                );
                report.blocked_by_l5.push((ticket.id.clone(), joined));
                return;
            }
        }

        let handle = self.router.get_handle(&candidate_id).await;
        let Some(handle) = handle else {
            tracing::warn!(
                ticket = %ticket.id,
                candidate = %ticket.candidate_adapter_id,
                "promote: candidate handle not registered on router; skipping (will retry next tick)",
            );
            report.skipped_missing_handle.push(ticket.id.clone());
            return;
        };

        // The hex-core composition tracks Arc<dyn Any>. The router has the
        // typed handle for actual call routing — composition just needs a
        // marker so the binding flips. Using `Arc::new(handle)` keeps a
        // strong reference (so the handle outlives the swap), but a unit
        // value would also work since composition never dereferences.
        let any_handle: Arc<dyn Any + Send + Sync> = Arc::new(handle);
        if let Err(e) = self.comp.in_memory().stage_handle(ticket_uuid, any_handle) {
            report.errors.push((ticket.id.clone(), format!("stage_handle: {}", e)));
            return;
        }
        if let Err(e) = self.comp.in_memory().mark_shadow_green(ticket_uuid) {
            report.errors.push((ticket.id.clone(), format!("mark_shadow_green: {}", e)));
            return;
        }
        if let Err(e) = self.comp.promote_async(ticket_uuid).await {
            report.errors.push((ticket.id.clone(), format!("promote_async: {}", e)));
            return;
        }

        if self.router.has_active_shadow(&port).await {
            self.router.end_shadow(&port).await;
        }
        report.promoted.push(ticket.id.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use hex_core::composition::{
        AdapterManifest, CompositionSwap, InMemoryComposition, PortRegistry,
    };
    use hex_core::ports::inference::mock::MockInferencePort;
    use hex_core::ports::inference::IInferencePort;

    use crate::ports::state::{ShadowSampleRecord, StateError};

    /// Minimal stub state port — supports just enough for the orchestrator's
    /// transition + sentinel-record calls.
    #[derive(Default)]
    struct StubState {
        transitions: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl ISwapTicketStatePort for StubState {
        async fn swap_ticket_create(
            &self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str,
            _: f32, _: u64, _: &str, _: &str,
        ) -> Result<(), StateError> {
            Ok(())
        }
        async fn swap_ticket_transition(
            &self,
            id: &str,
            new_state: &str,
            _ts: &str,
        ) -> Result<(), StateError> {
            self.transitions.lock().unwrap().push((id.to_string(), new_state.to_string()));
            Ok(())
        }
        async fn swap_ticket_set_shadow_started(&self, _: &str, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn swap_ticket_set_config(&self, _: &str, _: &str, _: f32, _: u64, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn shadow_sample_record(
            &self, _: &str, _: u64, _: &str, _: &str, _: &str, _: &str, _: bool, _: &str, _: &str,
        ) -> Result<(), StateError> {
            Ok(())
        }
        async fn shadow_tickets_due(&self, _: &str) -> Result<Vec<SwapTicketRecord>, StateError> {
            Ok(vec![])
        }
        async fn shadow_samples_for(&self, _: &str) -> Result<Vec<ShadowSampleRecord>, StateError> {
            Ok(vec![])
        }
        async fn shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, StateError> {
            Ok(vec![])
        }
    }

    fn ticket_record(id: &str, state: &str) -> SwapTicketRecord {
        SwapTicketRecord {
            id: id.into(),
            project_id: "test".into(),
            port_id: "inference".into(),
            incumbent_adapter_id: "mock-a".into(),
            candidate_adapter_id: "mock-b".into(),
            candidate_manifest_json: "{}".into(),
            state: state.into(),
            shadow_traffic_fraction: 1.0,
            shadow_window_seconds: 300,
            shadow_started_at: Utc::now().to_rfc3339(),
            success_criteria_json: "[]".into(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    /// Stub state port that ALSO returns canned shadow_green tickets from
    /// `shadow_green_tickets()`. Used by the production-path test below.
    #[derive(Default)]
    struct StubStateWithGreen {
        transitions: Mutex<Vec<(String, String)>>,
        green: Mutex<Vec<SwapTicketRecord>>,
    }

    #[async_trait]
    impl ISwapTicketStatePort for StubStateWithGreen {
        async fn swap_ticket_create(
            &self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str,
            _: f32, _: u64, _: &str, _: &str,
        ) -> Result<(), StateError> { Ok(()) }
        async fn swap_ticket_transition(&self, id: &str, new_state: &str, _: &str) -> Result<(), StateError> {
            self.transitions.lock().unwrap().push((id.into(), new_state.into()));
            Ok(())
        }
        async fn swap_ticket_set_shadow_started(&self, _: &str, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn swap_ticket_set_config(&self, _: &str, _: &str, _: f32, _: u64, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn shadow_sample_record(
            &self, _: &str, _: u64, _: &str, _: &str, _: &str, _: &str, _: bool, _: &str, _: &str,
        ) -> Result<(), StateError> { Ok(()) }
        async fn shadow_tickets_due(&self, _: &str) -> Result<Vec<SwapTicketRecord>, StateError> { Ok(vec![]) }
        async fn shadow_samples_for(&self, _: &str) -> Result<Vec<ShadowSampleRecord>, StateError> { Ok(vec![]) }
        async fn shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, StateError> {
            Ok(self.green.lock().unwrap().clone())
        }
    }

    /// Build composition + router with mock-a as the incumbent live binding,
    /// candidate handle (mock-b) registered. Returns the propose-swap ticket.
    async fn setup() -> (
        Arc<StubState>,
        Arc<SpacetimeRuntimeComposition>,
        Arc<ShadowRouter>,
        Uuid,
    ) {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("mock-a"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let state = Arc::new(StubState::default());
        let comp = Arc::new(SpacetimeRuntimeComposition::new(
            InMemoryComposition::new(reg),
            state.clone(),
            "test-project",
        ));
        // Propose a swap so the in-memory ticket exists with a real Uuid.
        let ticket = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: AdapterManifest {
                    adapter_id: AdapterId::new("mock-b"),
                    port: PortId::new("inference"),
                    version: "0.1.0".into(),
                    deps: vec![],
                },
            })
            .await
            .unwrap();
        let router = Arc::new(ShadowRouter::new(comp.clone(), state.clone()));
        let mock_b: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("hello from B"));
        router.register_handle(AdapterId::new("mock-b"), mock_b).await;
        (state, comp, router, ticket.id)
    }

    #[tokio::test]
    async fn promote_orchestrator_promotes_ticket_with_handle_registered() {
        let (state, comp, router, ticket_uuid) = setup().await;
        let mut rec = ticket_record(&ticket_uuid.to_string(), "shadow_green");
        // The recorded incumbent in STDB might not match in-memory
        // comp's incumbent here — orchestrator only cares about the
        // in-memory ticket existing and the router having the candidate
        // handle. The mismatch is fine for this unit-scope test.
        rec.candidate_adapter_id = "mock-b".into();
        let orch = PromoteOrchestrator::new(state.clone(), comp.clone(), router.clone());

        let report = orch.tick_with_tickets(vec![rec]).await;
        assert_eq!(report.promoted, vec![ticket_uuid.to_string()]);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
        // Live binding flipped to mock-b.
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-b"))
        );
        // STDB transition recorded (shadow_green → promoted).
        let txs = state.transitions.lock().unwrap().clone();
        assert!(txs.iter().any(|(id, st)| id == &ticket_uuid.to_string() && st == "promoted"));
    }

    #[tokio::test]
    async fn promote_orchestrator_skips_when_handle_missing() {
        let (state, comp, router, ticket_uuid) = setup().await;
        let mut rec = ticket_record(&ticket_uuid.to_string(), "shadow_green");
        rec.candidate_adapter_id = "never-registered".into();
        let orch = PromoteOrchestrator::new(state.clone(), comp.clone(), router.clone());

        let report = orch.tick_with_tickets(vec![rec]).await;
        assert!(report.promoted.is_empty());
        assert_eq!(report.skipped_missing_handle, vec![ticket_uuid.to_string()]);
        // Live binding unchanged.
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-a"))
        );
    }

    #[tokio::test]
    async fn promote_orchestrator_ignores_non_shadow_green_tickets() {
        let (state, comp, router, ticket_uuid) = setup().await;
        let rec = ticket_record(&ticket_uuid.to_string(), "shadow"); // not shadow_green
        let orch = PromoteOrchestrator::new(state.clone(), comp.clone(), router.clone());

        let report = orch.tick_with_tickets(vec![rec]).await;
        assert!(report.promoted.is_empty());
        assert!(report.skipped_missing_handle.is_empty());
        assert!(report.errors.is_empty());
        // Live binding unchanged.
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-a"))
        );
    }

    #[tokio::test]
    async fn promote_orchestrator_tick_drives_promotion_via_production_read_path() {
        // Builds with the StubStateWithGreen variant so tick() — which
        // calls fetch_shadow_green_tickets → state.shadow_green_tickets —
        // gets a real return. Exercises the production code path, not
        // the tick_with_tickets test seam.
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("mock-a"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let state = Arc::new(StubStateWithGreen::default());
        let comp = Arc::new(SpacetimeRuntimeComposition::new(
            InMemoryComposition::new(reg),
            state.clone(),
            "test-project",
        ));
        let ticket = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: AdapterManifest {
                    adapter_id: AdapterId::new("mock-b"),
                    port: PortId::new("inference"),
                    version: "0.1.0".into(),
                    deps: vec![],
                },
            })
            .await
            .unwrap();
        let router = Arc::new(ShadowRouter::new(comp.clone(), state.clone()));
        let mock_b: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("hello from B"));
        router.register_handle(AdapterId::new("mock-b"), mock_b).await;

        // Seed the production read with the ticket in shadow_green.
        state.green.lock().unwrap().push(SwapTicketRecord {
            id: ticket.id.to_string(),
            project_id: "test".into(),
            port_id: "inference".into(),
            incumbent_adapter_id: "mock-a".into(),
            candidate_adapter_id: "mock-b".into(),
            candidate_manifest_json: "{}".into(),
            state: "shadow_green".into(),
            shadow_traffic_fraction: 1.0,
            shadow_window_seconds: 300,
            shadow_started_at: Utc::now().to_rfc3339(),
            success_criteria_json: "[]".into(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        });

        let orch = PromoteOrchestrator::new(state.clone(), comp.clone(), router.clone());
        let report = orch.tick().await; // production path
        assert_eq!(report.considered, 1);
        assert_eq!(report.promoted, vec![ticket.id.to_string()]);
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-b"))
        );
        assert!(state.transitions.lock().unwrap().iter().any(|(_, st)| st == "promoted"));
    }

    #[tokio::test]
    async fn l5_conformance_blocks_promotion_when_violation_present() {
        use crate::orchestration::adr_conformance::AdrConformanceChecker;
        let (state, comp, router, ticket_uuid) = setup().await;
        // Use the deprecated-version canary so the conformance checker's
        // R1 fires regardless of the (empty) ADR registry.
        let mut rec = ticket_record(&ticket_uuid.to_string(), "shadow_green");
        rec.candidate_adapter_id = "mock-b".into();
        rec.candidate_manifest_json = r#"{"version":"deprecated"}"#.into();

        // Empty temp dir for the registry — R1 doesn't need any ADRs.
        let empty_dir = tempfile::tempdir().unwrap();
        let checker = AdrConformanceChecker::new(empty_dir.path());

        let orch = PromoteOrchestrator::new(state.clone(), comp.clone(), router.clone())
            .with_conformance(checker);

        let report = orch.tick_with_tickets(vec![rec]).await;
        assert!(report.promoted.is_empty(), "must not promote");
        assert_eq!(report.blocked_by_l5.len(), 1);
        assert!(report.blocked_by_l5[0].1.contains("deprecated"));
        // Live binding stayed at mock-a.
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-a"))
        );
    }

    #[tokio::test]
    async fn promote_orchestrator_is_idempotent_on_repeated_tick() {
        let (state, comp, router, ticket_uuid) = setup().await;
        let mut rec = ticket_record(&ticket_uuid.to_string(), "shadow_green");
        rec.candidate_adapter_id = "mock-b".into();
        let orch = PromoteOrchestrator::new(state.clone(), comp.clone(), router.clone());

        // First tick: promotes.
        let first = orch.tick_with_tickets(vec![rec.clone()]).await;
        assert_eq!(first.promoted.len(), 1);

        // Second tick on the same shadow_green-shaped record: the
        // orchestrator's idempotency guard sees the live binding already
        // matches the candidate and short-circuits. No promotion, no
        // error, no double-tick effects.
        let second = orch.tick_with_tickets(vec![rec]).await;
        assert!(second.promoted.is_empty());
        assert!(second.errors.is_empty(), "errors: {:?}", second.errors);
        // Live binding still mock-b — unchanged by the no-op tick.
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-b"))
        );
    }
}
