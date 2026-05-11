//! Substrate end-to-end (ADR-2026-04-26-1500 P6, wp-substrate-shadow-promotion P6.1).
//!
//! Drives a swap through the full circuit with no manual `mark_shadow_green`
//! and no daemon dispatch:
//!
//!     propose_swap (STDB persist + in-memory candidate)
//!         → begin shadow (STDB transition candidate→shadow + start timestamp +
//!           in-memory routing cache)
//!         → ShadowRouter routes N requests, mirroring traffic to candidate,
//!           recording N shadow_sample rows
//!         → PromotionJudge.tick() reads samples, evaluates criteria,
//!           transitions ticket to shadow_green or shadow_red
//!         → on shadow_green, mirror the verdict into the in-memory live
//!           registry (mark_shadow_green) and call promote_async (STDB
//!           transition shadow_green→promoted + in-memory swap)
//!
//! The "mirror verdict from STDB into in-memory + call promote" step is the
//! one piece of production orchestration that has not yet landed (it's the
//! "promote orchestrator" the substrate will run on a separate tick). Here
//! we inline it with a comment so the next workplan can lift it into
//! `sched_service`.
//!
//! State is held in `FakeStdbState` — an in-memory equivalent of the
//! `swap_ticket` + `shadow_sample` STDB tables, with the same state-machine
//! validation as the WASM reducers in `hexflo-coordination/src/lib.rs`.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use hex_core::composition::{
    AdapterId, AdapterManifest, CompositionSwap, InMemoryComposition, PortId, PortRegistry,
};
use hex_core::ports::adapter_generator::SuccessCriterion;
use hex_core::ports::inference::mock::MockInferencePort;
use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};
use tokio::sync::Mutex;

use hex_nexus::adapters::spacetime_composition::{
    AsyncRuntimeComposition, SpacetimeRuntimeComposition,
};
use hex_nexus::orchestration::promotion_judge::PromotionJudge;
use hex_nexus::orchestration::shadow_router::{ActiveShadowTicket, ShadowRouter};
use hex_nexus::ports::state::{
    ISwapTicketStatePort, ShadowSampleRecord, StateError, SwapTicketRecord,
};

/// In-memory facsimile of the hexflo-coordination swap_ticket +
/// shadow_sample tables, with the same state-machine validation. Mirrors
/// the production WASM reducers closely enough that this test exercises
/// the same logic the production deployment will run.
#[derive(Default)]
struct FakeStdbState {
    tickets: Mutex<BTreeMap<String, SwapTicketRecord>>,
    samples: Mutex<Vec<ShadowSampleRecord>>,
    next_sample_id: Mutex<u64>,
}

fn allowed(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("candidate", "shadow")
            | ("shadow", "shadow_green")
            | ("shadow", "shadow_red")
            | ("shadow_green", "promoted")
            | ("promoted", "rolled_back")
    )
}

#[async_trait]
impl ISwapTicketStatePort for FakeStdbState {
    async fn swap_ticket_create(
        &self,
        id: &str,
        project_id: &str,
        port_id: &str,
        incumbent: &str,
        candidate: &str,
        manifest_json: &str,
        fraction: f32,
        window: u64,
        criteria_json: &str,
        timestamp: &str,
    ) -> Result<(), StateError> {
        let mut t = self.tickets.lock().await;
        if t.contains_key(id) {
            return Err(StateError::Storage("ticket already exists".into()));
        }
        if !(0.0..=1.0).contains(&fraction) {
            return Err(StateError::Storage("fraction out of range".into()));
        }
        t.insert(
            id.to_string(),
            SwapTicketRecord {
                id: id.into(),
                project_id: project_id.into(),
                port_id: port_id.into(),
                incumbent_adapter_id: incumbent.into(),
                candidate_adapter_id: candidate.into(),
                candidate_manifest_json: manifest_json.into(),
                state: "candidate".into(),
                shadow_traffic_fraction: fraction,
                shadow_window_seconds: window,
                shadow_started_at: String::new(),
                success_criteria_json: criteria_json.into(),
                created_at: timestamp.into(),
                updated_at: timestamp.into(),
            },
        );
        Ok(())
    }

    async fn swap_ticket_transition(
        &self,
        id: &str,
        new_state: &str,
        timestamp: &str,
    ) -> Result<(), StateError> {
        let mut t = self.tickets.lock().await;
        let rec = t
            .get_mut(id)
            .ok_or_else(|| StateError::Storage("ticket not found".into()))?;
        if !allowed(&rec.state, new_state) {
            return Err(StateError::Storage(format!(
                "transition {} -> {} not allowed",
                rec.state, new_state
            )));
        }
        rec.state = new_state.to_string();
        rec.updated_at = timestamp.to_string();
        Ok(())
    }

    async fn swap_ticket_set_shadow_started(
        &self,
        id: &str,
        timestamp: &str,
    ) -> Result<(), StateError> {
        let mut t = self.tickets.lock().await;
        let rec = t
            .get_mut(id)
            .ok_or_else(|| StateError::Storage("ticket not found".into()))?;
        if rec.state != "shadow" {
            return Err(StateError::Storage("cannot set shadow_started_at outside shadow state".into()));
        }
        rec.shadow_started_at = timestamp.to_string();
        rec.updated_at = timestamp.to_string();
        Ok(())
    }

    async fn swap_ticket_set_config(
        &self,
        id: &str,
        criteria_json: &str,
        fraction: f32,
        window: u64,
        timestamp: &str,
    ) -> Result<(), StateError> {
        let mut t = self.tickets.lock().await;
        let rec = t
            .get_mut(id)
            .ok_or_else(|| StateError::Storage("ticket not found".into()))?;
        if !matches!(rec.state.as_str(), "candidate" | "shadow") {
            return Err(StateError::Storage(format!("cannot update config in state {}", rec.state)));
        }
        rec.success_criteria_json = criteria_json.to_string();
        rec.shadow_traffic_fraction = fraction;
        rec.shadow_window_seconds = window;
        rec.updated_at = timestamp.to_string();
        Ok(())
    }

    async fn shadow_sample_record(
        &self,
        ticket_id: &str,
        call_seq: u64,
        incumbent_adapter_id: &str,
        candidate_adapter_id: &str,
        incumbent_metrics_json: &str,
        candidate_metrics_json: &str,
        agreed: bool,
        reason: &str,
        timestamp: &str,
    ) -> Result<(), StateError> {
        let t = self.tickets.lock().await;
        if !t.contains_key(ticket_id) {
            return Err(StateError::Storage("sample for unknown ticket".into()));
        }
        drop(t);
        let mut next = self.next_sample_id.lock().await;
        *next += 1;
        let id = *next;
        drop(next);
        self.samples.lock().await.push(ShadowSampleRecord {
            id,
            ticket_id: ticket_id.into(),
            call_seq,
            incumbent_adapter_id: incumbent_adapter_id.into(),
            candidate_adapter_id: candidate_adapter_id.into(),
            incumbent_metrics_json: incumbent_metrics_json.into(),
            candidate_metrics_json: candidate_metrics_json.into(),
            agreed,
            reason: reason.into(),
            recorded_at: timestamp.into(),
        });
        Ok(())
    }

    async fn shadow_tickets_due(&self, _now: &str) -> Result<Vec<SwapTicketRecord>, StateError> {
        Ok(self
            .tickets
            .lock()
            .await
            .values()
            .filter(|t| t.state == "shadow")
            .cloned()
            .collect())
    }

    async fn shadow_samples_for(&self, ticket_id: &str) -> Result<Vec<ShadowSampleRecord>, StateError> {
        Ok(self
            .samples
            .lock()
            .await
            .iter()
            .filter(|s| s.ticket_id == ticket_id)
            .cloned()
            .collect())
    }

    async fn shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, StateError> {
        Ok(self
            .tickets
            .lock()
            .await
            .values()
            .filter(|t| t.state == "shadow_green")
            .cloned()
            .collect())
    }
}

fn build_request() -> InferenceRequest {
    InferenceRequest {
        model: "mock".into(),
        system_prompt: String::new(),
        messages: vec![],
        tools: vec![],
        max_tokens: 16,
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::Normal,
        grammar: None,
    }
}

fn manifest(adapter: &str) -> AdapterManifest {
    AdapterManifest {
        adapter_id: AdapterId::new(adapter),
        port: PortId::new("inference"),
        version: "0.1.0".into(),
        deps: vec![],
    }
}

#[tokio::test]
async fn substrate_end_to_end_promotes_candidate_when_criteria_pass() {
    let state = Arc::new(FakeStdbState::default());

    // ── Composition: mock-a as incumbent. ─────────────────
    let mut reg = PortRegistry::new();
    reg.bind(
        PortId::new("inference"),
        AdapterId::new("mock-a"),
        Arc::new(()) as Arc<dyn Any + Send + Sync>,
    );
    let comp = Arc::new(SpacetimeRuntimeComposition::new(
        InMemoryComposition::new(reg),
        state.clone(),
        "test-project",
    ));

    // ── Router: incumbent + candidate handles, deterministic always-shadow RNG. ──
    let router = Arc::new(
        ShadowRouter::new(comp.clone(), state.clone())
            .with_rng(Arc::new(|| 0.0)),
    );
    let mock_a: Arc<dyn IInferencePort> =
        Arc::new(MockInferencePort::with_response("hello from A"));
    let mock_b: Arc<dyn IInferencePort> =
        Arc::new(MockInferencePort::with_response("hello from B"));
    router.register_handle(AdapterId::new("mock-a"), mock_a).await;
    router
        .register_handle(AdapterId::new("mock-b"), mock_b.clone())
        .await;

    // ── Propose. ──────────────────────────────────────────
    let ticket = comp
        .propose_swap_async(CompositionSwap {
            port: PortId::new("inference"),
            new_adapter_id: AdapterId::new("mock-b"),
            manifest: manifest("mock-b"),
        })
        .await
        .expect("propose ok");

    // ── Begin shadow. ─────────────────────────────────────
    // (In production this is one orchestration step; here we call the
    // STDB transition + start timestamp + criteria-attach + router-cache
    // all explicitly so the test reads as the substrate's full procedure.)
    let now = Utc::now().to_rfc3339();
    state
        .swap_ticket_transition(&ticket.id.to_string(), "shadow", &now)
        .await
        .expect("transition to shadow");
    state
        .swap_ticket_set_shadow_started(&ticket.id.to_string(), &now)
        .await
        .expect("stamp shadow_started");
    {
        // Attach success criteria the judge will evaluate. In production,
        // these come from the AdapterSpec / IAdapterGenerator output —
        // the substrate orchestrator writes them on the ticket at the
        // candidate→shadow transition.
        let criteria = vec![
            SuccessCriterion::ResponseEquivalence { tolerance: 0.05 },
            SuccessCriterion::ErrorRateBelow(0.1),
        ];
        let mut tickets = state.tickets.lock().await;
        let rec = tickets.get_mut(&ticket.id.to_string()).unwrap();
        rec.success_criteria_json = serde_json::to_string(&criteria).unwrap();
    }
    router
        .begin_shadow(
            PortId::new("inference"),
            ActiveShadowTicket {
                ticket_id: ticket.id.to_string(),
                candidate_adapter_id: AdapterId::new("mock-b"),
                traffic_fraction: 1.0,
            },
        )
        .await;

    // ── Drive 20 mirrored requests. ───────────────────────
    for _ in 0..20 {
        let resp = router
            .route(PortId::new("inference"), build_request())
            .await
            .expect("route ok");
        // Caller-visible behaviour during shadow is unchanged: incumbent's response.
        match resp.content.first().unwrap() {
            hex_core::domain::messages::ContentBlock::Text { text } => {
                assert_eq!(text, "hello from A");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
    assert_eq!(state.samples.lock().await.len(), 20);

    // ── Judge: ticks AFTER the window has elapsed. ────────
    let judge = PromotionJudge::new(state.clone());
    // Simulate clock advancing past the 5-minute window.
    let later = Utc::now() + Duration::minutes(10);
    // Window in the ticket defaulted to 300s in propose_swap_async — but
    // we never set one explicitly so any value should work; we just need
    // the judge's `now` to be past `shadow_started_at + window`. With a
    // 300s window and `later` 10 minutes after `now`, this is satisfied.
    let report = judge.tick_at(later).await;
    assert_eq!(report.due, 1, "ticket past window");
    assert_eq!(report.promoted_to_green, vec![ticket.id.to_string()]);
    assert!(report.marked_red.is_empty());

    // STDB ticket is now shadow_green.
    {
        let t = state.tickets.lock().await;
        assert_eq!(t.get(&ticket.id.to_string()).unwrap().state, "shadow_green");
    }

    // ── Promote orchestrator: mirror the verdict from STDB into the
    // in-memory live registry, then call promote_async. The hex-core
    // `RuntimeComposition` is sync and doesn't subscribe to STDB events;
    // a small "promote orchestrator" tick will own this in production.
    // PortRegistry tracks Arc<dyn Any>; the typed IInferencePort handle
    // lives in the router's separate map (composition + router are
    // intentionally separated — composition tracks "what's bound", router
    // tracks "how to call it"). A unit handle is sufficient here.
    comp.in_memory()
        .stage_handle(ticket.id, Arc::new(()) as Arc<dyn Any + Send + Sync>)
        .expect("stage handle");
    comp.in_memory()
        .mark_shadow_green(ticket.id)
        .expect("mark shadow_green in memory");
    comp.promote_async(ticket.id).await.expect("promote ok");

    // STDB ticket is now promoted.
    {
        let t = state.tickets.lock().await;
        assert_eq!(t.get(&ticket.id.to_string()).unwrap().state, "promoted");
    }
    // Live binding in the in-memory registry has flipped.
    assert_eq!(
        comp.binding_id(&PortId::new("inference")),
        Some(AdapterId::new("mock-b"))
    );

    // ── Stop shadowing — terminal state. Subsequent route() calls go to
    // the new live binding (mock-b) and no further samples are recorded.
    router.end_shadow(&PortId::new("inference")).await;
    let baseline = state.samples.lock().await.len();
    let resp = router
        .route(PortId::new("inference"), build_request())
        .await
        .expect("post-promote route");
    match resp.content.first().unwrap() {
        hex_core::domain::messages::ContentBlock::Text { text } => {
            assert_eq!(text, "hello from B", "post-promote, candidate is live");
        }
        other => panic!("unexpected: {:?}", other),
    }
    assert_eq!(state.samples.lock().await.len(), baseline);
}

#[tokio::test]
async fn substrate_end_to_end_marks_red_when_candidate_errors_too_often() {
    let state = Arc::new(FakeStdbState::default());

    let mut reg = PortRegistry::new();
    reg.bind(
        PortId::new("inference"),
        AdapterId::new("mock-a"),
        Arc::new(()) as Arc<dyn Any + Send + Sync>,
    );
    let comp = Arc::new(SpacetimeRuntimeComposition::new(
        InMemoryComposition::new(reg),
        state.clone(),
        "test-project",
    ));

    let router = Arc::new(
        ShadowRouter::new(comp.clone(), state.clone())
            .with_rng(Arc::new(|| 0.0)),
    );
    let mock_a: Arc<dyn IInferencePort> =
        Arc::new(MockInferencePort::with_response("hello"));
    let unreachable: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::unreachable());
    router.register_handle(AdapterId::new("mock-a"), mock_a).await;
    router
        .register_handle(AdapterId::new("broken"), unreachable)
        .await;

    let ticket = comp
        .propose_swap_async(CompositionSwap {
            port: PortId::new("inference"),
            new_adapter_id: AdapterId::new("broken"),
            manifest: manifest("broken"),
        })
        .await
        .expect("propose ok");

    let now = Utc::now().to_rfc3339();
    state
        .swap_ticket_transition(&ticket.id.to_string(), "shadow", &now)
        .await
        .unwrap();
    state
        .swap_ticket_set_shadow_started(&ticket.id.to_string(), &now)
        .await
        .unwrap();
    {
        let criteria = vec![SuccessCriterion::ErrorRateBelow(0.0)];
        let mut tickets = state.tickets.lock().await;
        let rec = tickets.get_mut(&ticket.id.to_string()).unwrap();
        rec.success_criteria_json = serde_json::to_string(&criteria).unwrap();
    }
    router
        .begin_shadow(
            PortId::new("inference"),
            ActiveShadowTicket {
                ticket_id: ticket.id.to_string(),
                candidate_adapter_id: AdapterId::new("broken"),
                traffic_fraction: 1.0,
            },
        )
        .await;

    for _ in 0..10 {
        let _ = router
            .route(PortId::new("inference"), build_request())
            .await
            .expect("incumbent succeeds even when candidate errors");
    }
    // 10 real samples. Every candidate metric snapshot has error=true
    // because the mock returns ProviderUnavailable.
    assert_eq!(state.samples.lock().await.len(), 10);

    // Judge runs after the window.
    let judge = PromotionJudge::new(state.clone());
    let later = Utc::now() + Duration::minutes(10);
    let report = judge.tick_at(later).await;
    assert_eq!(report.marked_red.len(), 1);
    assert_eq!(report.marked_red[0].0, ticket.id.to_string());
    assert!(report.marked_red[0].1.contains("ErrorRateBelow"));

    // STDB ticket is shadow_red. Promote attempt must fail (state machine
    // rejects shadow_red → promoted).
    {
        let t = state.tickets.lock().await;
        assert_eq!(t.get(&ticket.id.to_string()).unwrap().state, "shadow_red");
    }
    let promote_err = state
        .swap_ticket_transition(&ticket.id.to_string(), "promoted", &Utc::now().to_rfc3339())
        .await
        .expect_err("STDB rejects shadow_red → promoted");
    assert!(promote_err.to_string().contains("not allowed"));

    // Live binding is unchanged — incumbent is still mock-a.
    assert_eq!(
        comp.binding_id(&PortId::new("inference")),
        Some(AdapterId::new("mock-a"))
    );

    // Sentinel sample with the rejection reason was recorded by the judge.
    // (The 10 routed samples also carry non-empty reasons because the
    // candidate errored on every call — distinguish by criterion name.)
    let total_samples = state.samples.lock().await.len();
    assert_eq!(total_samples, 11, "10 routed + 1 sentinel from judge");
    let sentinel = state
        .samples
        .lock()
        .await
        .iter()
        .find(|s| s.reason.contains("ErrorRateBelow"))
        .cloned()
        .expect("judge sentinel naming the failed criterion");
    assert_eq!(sentinel.ticket_id, ticket.id.to_string());
}
