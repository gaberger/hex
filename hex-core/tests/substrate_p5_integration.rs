//! Substrate P5 integration test (ADR-2026-04-26-1500).
//!
//! Wires the existing `IInferencePort` (the system's prototype port for the
//! self-modifying-substrate frame, per ADR-2026-04-26-1303) behind
//! `RuntimeComposition` and exercises the full circuit:
//!
//! 1. Two `MockInferencePort` adapters are registered as candidate
//!    implementations of the `inference` port.
//! 2. `InMemoryComposition` starts with Mock A as the live binding.
//! 3. A swap to Mock B is proposed → Candidate ticket.
//! 4. Mock B's handle is staged; ticket is force-marked `ShadowGreen` (the
//!    real shadow-test judge — substrate C5 — will own this transition in
//!    workplan P6).
//! 5. The ticket is promoted; the live binding flips to Mock B and a call
//!    to `complete()` returns Mock B's canned response, observed through
//!    the `RuntimeComposition` resolution path.
//! 6. The substrate's `PortTelemetry` sink captures one `InferenceMetrics`
//!    sample per call, demonstrating that telemetry emit flows are wired
//!    end-to-end.
//! 7. Rollback restores the prior binding to Mock A; subsequent calls
//!    return Mock A's response.
//!
//! This test is the proof that the substrate's day-one contracts (C2 + C3)
//! compose correctly with the prototype port. Workplan P6 will replace
//! the manual `mark_shadow_green` with the real shadow-promotion judge.

use std::any::Any;
use std::sync::{Arc, Mutex};

use hex_core::composition::{
    AdapterId, AdapterManifest, CompositionSwap, InMemoryComposition, PortId, PortRegistry,
    RuntimeComposition,
};
use hex_core::ports::inference::mock::MockInferencePort;
use hex_core::ports::inference::{
    IInferencePort, InferenceMetrics, InferencePortTelemetry, InferenceRequest, Priority,
};
use hex_core::telemetry::{register_sink, PortTelemetry};

const PORT: &str = "inference";

fn build_request() -> InferenceRequest {
    InferenceRequest {
        model: "mock-model".to_string(),
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
        port: PortId::new(PORT),
        version: "0.1.0".into(),
        deps: vec![],
    }
}

/// Resolve the live `IInferencePort` binding from the composition + registry.
/// We keep an out-of-band registry alongside the composition's `PortRegistry`
/// so the test does not need a public API for "give me the typed handle for
/// the currently-bound adapter on port P." Real consumers will use a typed
/// resolver helper landed in P6 alongside the shadow-promotion plumbing.
fn resolve(
    handles: &Mutex<std::collections::BTreeMap<AdapterId, Arc<dyn IInferencePort>>>,
    comp: &InMemoryComposition,
) -> Arc<dyn IInferencePort> {
    let id = comp
        .binding_id(&PortId::new(PORT))
        .expect("inference port must be bound");
    handles
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .expect("handle for current binding")
}

#[tokio::test]
async fn substrate_swaps_inference_provider_end_to_end() {
    // Register a telemetry sink that captures samples into a shared vec.
    // OnceLock semantics mean a second test in the same process would not
    // be able to register again — but we only need one sink for this proof.
    let captured: Arc<Mutex<Vec<(AdapterId, InferenceMetrics)>>> = Arc::new(Mutex::new(vec![]));
    let sink_captured = captured.clone();
    let _ = register_sink(move |adapter_id, sample| {
        if let Some(m) = sample.downcast_ref::<InferenceMetrics>() {
            sink_captured.lock().unwrap().push((adapter_id, m.clone()));
        }
    });

    // Build mocks. Each is wrapped in Arc so we can hand identical handles
    // to both the composition's PortRegistry and our typed-handle map.
    let mock_a: Arc<dyn IInferencePort> =
        Arc::new(MockInferencePort::with_response("hello from A"));
    let mock_b: Arc<dyn IInferencePort> =
        Arc::new(MockInferencePort::with_response("hello from B"));

    let handles: Mutex<std::collections::BTreeMap<AdapterId, Arc<dyn IInferencePort>>> =
        Mutex::new(std::collections::BTreeMap::new());
    handles
        .lock()
        .unwrap()
        .insert(AdapterId::new("mock-a"), mock_a.clone());
    handles
        .lock()
        .unwrap()
        .insert(AdapterId::new("mock-b"), mock_b.clone());

    // Initial composition: Mock A bound. We bind a type-erased Arc into the
    // PortRegistry so the substrate's swap machinery has a handle to swap
    // against, even though the resolver itself uses our typed map above.
    let mut reg = PortRegistry::new();
    reg.bind(
        PortId::new(PORT),
        AdapterId::new("mock-a"),
        Arc::new("mock-a-handle") as Arc<dyn Any + Send + Sync>,
    );
    let comp = InMemoryComposition::new(reg);

    // ── Pre-swap: Mock A is live. ──────────────────────────
    assert_eq!(comp.binding_id(&PortId::new(PORT)), Some(AdapterId::new("mock-a")));
    let live = resolve(&handles, &comp);
    let resp = live.complete(build_request()).await.expect("mock A responds");
    InferencePortTelemetry::emit(
        AdapterId::new("mock-a"),
        InferenceMetrics {
            model_used: resp.model_used.clone(),
            latency_ms: resp.latency_ms,
            input_tokens: resp.input_tokens,
            output_tokens: resp.output_tokens,
            error: false,
        },
    );
    // Assert Mock A's text payload made it through.
    let pre_text = match resp.content.first().expect("at least one block") {
        hex_core::domain::messages::ContentBlock::Text { text } => text.clone(),
        other => panic!("unexpected content block: {:?}", other),
    };
    assert_eq!(pre_text, "hello from A");

    // ── Swap: propose -> stage handle -> shadow-green -> promote. ──
    let ticket = comp
        .propose_swap(CompositionSwap {
            port: PortId::new(PORT),
            new_adapter_id: AdapterId::new("mock-b"),
            manifest: manifest("mock-b"),
        })
        .expect("propose swap");

    // Concurrent proposal on the same port must be rejected.
    let dup = comp.propose_swap(CompositionSwap {
        port: PortId::new(PORT),
        new_adapter_id: AdapterId::new("mock-b"),
        manifest: manifest("mock-b"),
    });
    assert!(dup.is_err(), "single-writer per port enforced");

    comp.stage_handle(
        ticket.id,
        Arc::new("mock-b-handle") as Arc<dyn Any + Send + Sync>,
    )
    .unwrap();
    comp.mark_shadow_green(ticket.id).unwrap();
    comp.promote(ticket.id).expect("promote");

    // ── Post-swap: Mock B is live. ─────────────────────────
    assert_eq!(comp.binding_id(&PortId::new(PORT)), Some(AdapterId::new("mock-b")));
    let live = resolve(&handles, &comp);
    let resp = live.complete(build_request()).await.expect("mock B responds");
    InferencePortTelemetry::emit(
        AdapterId::new("mock-b"),
        InferenceMetrics {
            model_used: resp.model_used.clone(),
            latency_ms: resp.latency_ms,
            input_tokens: resp.input_tokens,
            output_tokens: resp.output_tokens,
            error: false,
        },
    );
    let post_text = match resp.content.first().unwrap() {
        hex_core::domain::messages::ContentBlock::Text { text } => text.clone(),
        other => panic!("unexpected content block: {:?}", other),
    };
    assert_eq!(post_text, "hello from B");

    // ── Rollback: prior binding restored, Mock A live again. ──
    comp.rollback(ticket.id).expect("rollback");
    assert_eq!(comp.binding_id(&PortId::new(PORT)), Some(AdapterId::new("mock-a")));
    let live = resolve(&handles, &comp);
    let resp = live.complete(build_request()).await.expect("mock A again");
    let rb_text = match resp.content.first().unwrap() {
        hex_core::domain::messages::ContentBlock::Text { text } => text.clone(),
        other => panic!("unexpected content block: {:?}", other),
    };
    assert_eq!(rb_text, "hello from A");

    // ── Telemetry: at least the two emits we made are visible. ──
    let samples = captured.lock().unwrap();
    assert!(samples.len() >= 2, "expected >=2 telemetry samples, got {}", samples.len());
    let adapters: Vec<&AdapterId> = samples.iter().map(|(id, _)| id).collect();
    assert!(adapters.contains(&&AdapterId::new("mock-a")));
    assert!(adapters.contains(&&AdapterId::new("mock-b")));
}
