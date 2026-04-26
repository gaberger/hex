//! ShadowRouter for `IInferencePort` (ADR-2604261500 P6,
//! wp-substrate-shadow-promotion P3.1).
//!
//! Responsibilities:
//! 1. Resolve the live binding for the inference port from the
//!    `SpacetimeRuntimeComposition`.
//! 2. If a Shadow ticket is registered for that port, sample uniform random
//!    against `traffic_fraction`; on hit, fire the incumbent and the
//!    candidate concurrently (`tokio::join!`), record a `shadow_sample`
//!    row with both per-call metrics, and return the incumbent's response
//!    to the caller (caller-visible behaviour is unchanged during shadow).
//! 3. If no Shadow ticket is open or the sample misses, route to the
//!    incumbent only.
//!
//! Concrete to `IInferencePort` for now (substrate's first port). The
//! generic abstraction lands in P10 alongside the second-port migration —
//! premature generalization here would buy nothing today and is exactly
//! what ADR-2604261500's "proof of concept before generalization" mitigation
//! warns against.
//!
//! `MetricsRng` is a `Box<dyn Fn() -> f32>` — production code wires it to
//! `rand::random`, tests inject deterministic values. Avoids leaking `rand`
//! into the abstraction.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use hex_core::composition::{AdapterId, PortId};
use hex_core::ports::inference::{
    IInferencePort, InferenceError, InferenceMetrics, InferenceRequest, InferenceResponse,
};
use tokio::sync::{Mutex, RwLock};

use crate::adapters::spacetime_composition::{AsyncRuntimeComposition, SpacetimeRuntimeComposition};
use crate::ports::state::ISwapTicketStatePort;

/// In-memory record of an active shadow window for a port. Populated when
/// the substrate transitions a ticket from Candidate to Shadow (P4 will
/// drive this transition via the promotion judge); cleared when the
/// ticket reaches a terminal state.
#[derive(Debug, Clone)]
pub struct ActiveShadowTicket {
    pub ticket_id: String,
    pub candidate_adapter_id: AdapterId,
    pub traffic_fraction: f32,
}

type SampleRng = Arc<dyn Fn() -> f32 + Send + Sync>;

pub struct ShadowRouter {
    comp: Arc<SpacetimeRuntimeComposition>,
    state: Arc<dyn ISwapTicketStatePort>,
    handles: RwLock<BTreeMap<AdapterId, Arc<dyn IInferencePort>>>,
    active_tickets: RwLock<BTreeMap<PortId, ActiveShadowTicket>>,
    call_seqs: Mutex<BTreeMap<String, u64>>,
    rng: SampleRng,
    /// Per-adapter last-routed-at, used by the L4 shrinkage daemon to
    /// decide which non-bound, non-shadowing handles can be unregistered.
    /// Process-local — restart resets the survey window, which is fine
    /// because shrinkage is a soft-pressure tool, not a correctness gate.
    routed_at: RwLock<BTreeMap<AdapterId, Instant>>,
}

impl ShadowRouter {
    pub fn new(
        comp: Arc<SpacetimeRuntimeComposition>,
        state: Arc<dyn ISwapTicketStatePort>,
    ) -> Self {
        Self {
            comp,
            state,
            handles: RwLock::new(BTreeMap::new()),
            active_tickets: RwLock::new(BTreeMap::new()),
            call_seqs: Mutex::new(BTreeMap::new()),
            rng: Arc::new(|| rand::random::<f32>()),
            routed_at: RwLock::new(BTreeMap::new()),
        }
    }

    /// Inject a deterministic sampler (test seam). Production never calls
    /// this — the constructor wires `rand::random` by default.
    pub fn with_rng(mut self, rng: SampleRng) -> Self {
        self.rng = rng;
        self
    }

    /// Register a typed `IInferencePort` handle for an adapter id.
    pub async fn register_handle(&self, adapter_id: AdapterId, handle: Arc<dyn IInferencePort>) {
        self.handles.write().await.insert(adapter_id, handle);
    }

    /// Look up a previously-registered handle. Used by the promote
    /// orchestrator to retrieve a candidate's typed handle when a
    /// shadow_green ticket is ready to promote.
    pub async fn get_handle(&self, adapter_id: &AdapterId) -> Option<Arc<dyn IInferencePort>> {
        self.handles.read().await.get(adapter_id).cloned()
    }

    /// Has the active-shadow cache for `port` been populated by a prior
    /// `begin_shadow`? The promote orchestrator uses this to decide
    /// whether to call `end_shadow` after promotion.
    pub async fn has_active_shadow(&self, port: &PortId) -> bool {
        self.active_tickets.read().await.contains_key(port)
    }

    /// Number of registered handles (any state). Used by the
    /// substrate-status endpoint for operator visibility.
    pub async fn handle_count(&self) -> usize {
        self.handles.read().await.len()
    }

    /// Number of active in-memory shadow tickets (one per port). Used by
    /// the substrate-status endpoint.
    pub async fn active_shadow_count(&self) -> usize {
        self.active_tickets.read().await.len()
    }

    /// Compute which registered handles are eligible for L4 shrinkage:
    /// not currently bound to any port (per the in-memory composition),
    /// not the candidate of any active shadow ticket, and either never
    /// routed to or last routed before `now - idle_window`. Returns the
    /// adapter ids the shrinkage daemon should `unregister_handle`.
    pub async fn shrinkable_adapters(
        &self,
        idle_window: std::time::Duration,
    ) -> Vec<AdapterId> {
        let now = Instant::now();
        let bound: std::collections::BTreeSet<AdapterId> = self
            .comp
            .snapshot()
            .bindings
            .into_values()
            .collect();
        let active: std::collections::BTreeSet<AdapterId> = self
            .active_tickets
            .read()
            .await
            .values()
            .map(|t| t.candidate_adapter_id.clone())
            .collect();
        let routed_at = self.routed_at.read().await;
        let handles = self.handles.read().await;
        handles
            .keys()
            .filter(|id| !bound.contains(*id) && !active.contains(*id))
            .filter(|id| match routed_at.get(*id) {
                Some(at) => now.duration_since(*at) >= idle_window,
                None => true, // never routed to → eligible
            })
            .cloned()
            .collect()
    }

    /// Remove a handle. Returns true if a handle was present and removed.
    /// The shrinkage daemon calls this after `shrinkable_adapters`.
    pub async fn unregister_handle(&self, adapter_id: &AdapterId) -> bool {
        let removed = self.handles.write().await.remove(adapter_id).is_some();
        self.routed_at.write().await.remove(adapter_id);
        removed
    }

    /// Bump the per-adapter last-routed-at timestamp. Internal helper
    /// called from `route` for both incumbent and candidate.
    async fn mark_routed(&self, adapter_id: &AdapterId) {
        self.routed_at
            .write()
            .await
            .insert(adapter_id.clone(), Instant::now());
    }

    /// Begin shadowing for a port. Caller is the substrate's swap orchestrator
    /// (workplan P4) — once the swap_ticket transitions to "shadow" in STDB
    /// this method registers the in-memory routing entry the router uses to
    /// decide whether each call is mirrored.
    pub async fn begin_shadow(&self, port: PortId, ticket: ActiveShadowTicket) {
        self.active_tickets.write().await.insert(port, ticket);
    }

    /// Stop shadowing — called when the ticket reaches a terminal state
    /// (ShadowGreen → Promoted, ShadowRed, RolledBack).
    pub async fn end_shadow(&self, port: &PortId) {
        self.active_tickets.write().await.remove(port);
    }

    async fn next_seq(&self, ticket_id: &str) -> u64 {
        let mut seqs = self.call_seqs.lock().await;
        let seq = seqs.entry(ticket_id.to_string()).or_insert(0);
        *seq += 1;
        *seq
    }

    fn metrics_from(resp: &InferenceResponse, error: bool) -> InferenceMetrics {
        InferenceMetrics {
            model_used: resp.model_used.clone(),
            latency_ms: resp.latency_ms,
            input_tokens: resp.input_tokens,
            output_tokens: resp.output_tokens,
            error,
        }
    }

    fn metrics_for_error() -> InferenceMetrics {
        InferenceMetrics {
            model_used: String::new(),
            latency_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            error: true,
        }
    }

    pub async fn route(
        &self,
        port: PortId,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let live_id = self
            .comp
            .binding_id(&port)
            .ok_or_else(|| InferenceError::UnknownProvider(port.0.clone()))?;
        let live_handle = {
            let handles = self.handles.read().await;
            handles
                .get(&live_id)
                .cloned()
                .ok_or_else(|| InferenceError::UnknownProvider(live_id.0.clone()))?
        };

        let active = self.active_tickets.read().await.get(&port).cloned();
        let Some(active) = active else {
            self.mark_routed(&live_id).await;
            return live_handle.complete(request).await;
        };

        let draw = (self.rng)();
        if draw >= active.traffic_fraction {
            self.mark_routed(&live_id).await;
            return live_handle.complete(request).await;
        }

        let candidate_handle = {
            let handles = self.handles.read().await;
            handles.get(&active.candidate_adapter_id).cloned()
        };

        let Some(candidate_handle) = candidate_handle else {
            // Candidate handle is missing — silently fall through to the
            // incumbent. The substrate would normally not let a swap reach
            // Shadow without a registered candidate handle, but a missing
            // handle should never make a live call fail.
            tracing::warn!(
                port = ?port,
                ticket = %active.ticket_id,
                candidate = ?active.candidate_adapter_id,
                "shadow: candidate handle missing, falling back to incumbent only",
            );
            self.mark_routed(&live_id).await;
            return live_handle.complete(request).await;
        };

        // Mirror — both calls fire concurrently. The incumbent's result is
        // always the one returned to the caller; the candidate's result
        // contributes only to telemetry.
        self.mark_routed(&live_id).await;
        self.mark_routed(&active.candidate_adapter_id).await;
        let req_clone = request.clone();
        let (incumbent_res, candidate_res) =
            tokio::join!(live_handle.complete(request), candidate_handle.complete(req_clone));

        let incumbent_metrics = match &incumbent_res {
            Ok(r) => Self::metrics_from(r, false),
            Err(_) => Self::metrics_for_error(),
        };
        let candidate_metrics = match &candidate_res {
            Ok(r) => Self::metrics_from(r, false),
            Err(_) => Self::metrics_for_error(),
        };

        // Day-one agreement model: the candidate "agrees" if both calls
        // succeeded. Real semantic equivalence (response text comparison,
        // per-criterion checks) is the promotion judge's job (P4) — it
        // reads these samples and applies SuccessCriterion logic.
        let agreed = matches!((&incumbent_res, &candidate_res), (Ok(_), Ok(_)));
        let reason = if agreed {
            String::new()
        } else {
            match (&incumbent_res, &candidate_res) {
                (Err(e), _) => format!("incumbent error: {}", e),
                (_, Err(e)) => format!("candidate error: {}", e),
                _ => String::new(),
            }
        };

        let call_seq = self.next_seq(&active.ticket_id).await;
        let inc_json = serde_json::to_string(&serde_json::json!({
            "model_used": incumbent_metrics.model_used,
            "latency_ms": incumbent_metrics.latency_ms,
            "input_tokens": incumbent_metrics.input_tokens,
            "output_tokens": incumbent_metrics.output_tokens,
            "error": incumbent_metrics.error,
        }))
        .unwrap_or_else(|_| "{}".into());
        let cand_json = serde_json::to_string(&serde_json::json!({
            "model_used": candidate_metrics.model_used,
            "latency_ms": candidate_metrics.latency_ms,
            "input_tokens": candidate_metrics.input_tokens,
            "output_tokens": candidate_metrics.output_tokens,
            "error": candidate_metrics.error,
        }))
        .unwrap_or_else(|_| "{}".into());

        if let Err(e) = self
            .state
            .shadow_sample_record(
                &active.ticket_id,
                call_seq,
                &live_id.0,
                &active.candidate_adapter_id.0,
                &inc_json,
                &cand_json,
                agreed,
                &reason,
                &Utc::now().to_rfc3339(),
            )
            .await
        {
            // Telemetry failure must not affect caller-visible behaviour.
            tracing::warn!(
                ticket = %active.ticket_id,
                error = %e,
                "shadow_sample_record failed; sample dropped",
            );
        }

        incumbent_res
    }
}

/// `IInferencePort` adapter that delegates `complete()` calls through a
/// `ShadowRouter`. Lets consumers opt into substrate-mediated inference
/// without changing their call shape: hand them a `ShadowRouterInferenceAdapter`
/// instead of a raw `Arc<dyn IInferencePort>` and they keep calling
/// `.complete()` exactly as before. Stream / health / capabilities pass
/// straight through to the fallback (substrate doesn't intercept these
/// today — the router only mirrors `complete` calls).
pub struct ShadowRouterInferenceAdapter {
    router: Arc<ShadowRouter>,
    fallback: Arc<dyn IInferencePort>,
}

impl ShadowRouterInferenceAdapter {
    pub fn new(router: Arc<ShadowRouter>, fallback: Arc<dyn IInferencePort>) -> Self {
        Self { router, fallback }
    }
}

#[async_trait::async_trait]
impl IInferencePort for ShadowRouterInferenceAdapter {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<hex_core::ports::inference::InferenceResponse, InferenceError> {
        self.router.route(PortId::new("inference"), request).await
    }

    async fn stream(
        &self,
        request: InferenceRequest,
    ) -> Result<
        Box<
            dyn hex_core::ports::inference::futures_stream::Stream<
                    Item = hex_core::ports::inference::StreamChunk,
                > + Send
                + Unpin,
        >,
        InferenceError,
    > {
        self.fallback.stream(request).await
    }

    async fn health(
        &self,
    ) -> Result<hex_core::ports::inference::HealthStatus, InferenceError> {
        self.fallback.health().await
    }

    fn capabilities(&self) -> hex_core::ports::inference::InferenceCapabilities {
        self.fallback.capabilities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use hex_core::composition::{AdapterManifest, CompositionSwap, InMemoryComposition, PortRegistry};
    use hex_core::ports::inference::mock::MockInferencePort;
    use hex_core::ports::inference::Priority;

    use crate::ports::state::StateError;

    #[derive(Default)]
    struct MockSwapState {
        samples: StdMutex<Vec<(String, u64, bool)>>, // (ticket_id, call_seq, agreed)
    }

    #[async_trait]
    impl ISwapTicketStatePort for MockSwapState {
        async fn swap_ticket_create(
            &self,
            _id: &str,
            _project_id: &str,
            _port_id: &str,
            _incumbent: &str,
            _candidate: &str,
            _manifest_json: &str,
            _fraction: f32,
            _window: u64,
            _criteria_json: &str,
            _ts: &str,
        ) -> Result<(), StateError> {
            Ok(())
        }
        async fn swap_ticket_transition(
            &self,
            _id: &str,
            _new_state: &str,
            _ts: &str,
        ) -> Result<(), StateError> {
            Ok(())
        }
        async fn swap_ticket_set_shadow_started(
            &self,
            _id: &str,
            _ts: &str,
        ) -> Result<(), StateError> {
            Ok(())
        }
        async fn swap_ticket_set_config(
            &self,
            _id: &str,
            _criteria: &str,
            _fraction: f32,
            _window: u64,
            _ts: &str,
        ) -> Result<(), StateError> {
            Ok(())
        }
        async fn shadow_sample_record(
            &self,
            ticket_id: &str,
            call_seq: u64,
            _incumbent: &str,
            _candidate: &str,
            _inc_metrics: &str,
            _cand_metrics: &str,
            agreed: bool,
            _reason: &str,
            _ts: &str,
        ) -> Result<(), StateError> {
            self.samples
                .lock()
                .unwrap()
                .push((ticket_id.to_string(), call_seq, agreed));
            Ok(())
        }

        async fn shadow_tickets_due(&self, _now: &str) -> Result<Vec<crate::ports::state::SwapTicketRecord>, StateError> {
            Ok(vec![])
        }

        async fn shadow_samples_for(&self, _ticket_id: &str) -> Result<Vec<crate::ports::state::ShadowSampleRecord>, StateError> {
            Ok(vec![])
        }

        async fn shadow_green_tickets(&self) -> Result<Vec<crate::ports::state::SwapTicketRecord>, StateError> {
            Ok(vec![])
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

    /// Wire up: comp with mock-a bound, candidate mock-b proposed but not
    /// yet shadowed. Router has handles for both.
    async fn build_router(rng: SampleRng) -> (Arc<ShadowRouter>, Arc<MockSwapState>, uuid::Uuid) {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("mock-a"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let in_mem = InMemoryComposition::new(reg);
        let state = Arc::new(MockSwapState::default());
        let comp = Arc::new(SpacetimeRuntimeComposition::new(
            in_mem,
            state.clone(),
            "test-project",
        ));
        let ticket = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: manifest("mock-b"),
            })
            .await
            .expect("propose ok");

        let router = Arc::new(ShadowRouter::new(comp, state.clone()).with_rng(rng));
        let mock_a: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("hello from A"));
        let mock_b: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("hello from B"));
        router.register_handle(AdapterId::new("mock-a"), mock_a).await;
        router.register_handle(AdapterId::new("mock-b"), mock_b).await;
        (router, state, ticket.id)
    }

    #[tokio::test]
    async fn route_calls_incumbent_only_when_no_shadow_ticket_open() {
        // Always-shadow RNG, but no active ticket → no shadow path taken.
        let (router, state, _ticket_id) = build_router(Arc::new(|| 0.0)).await;
        let resp = router
            .route(PortId::new("inference"), build_request())
            .await
            .expect("complete");
        match resp.content.first().unwrap() {
            hex_core::domain::messages::ContentBlock::Text { text } => {
                assert_eq!(text, "hello from A");
            }
            other => panic!("unexpected content block: {:?}", other),
        }
        assert_eq!(state.samples.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn route_mirrors_to_candidate_when_ticket_active_and_sample_hits() {
        // Always-shadow RNG (draw=0.0 < fraction=1.0) → both adapters called.
        let (router, state, ticket_id) = build_router(Arc::new(|| 0.0)).await;
        router
            .begin_shadow(
                PortId::new("inference"),
                ActiveShadowTicket {
                    ticket_id: ticket_id.to_string(),
                    candidate_adapter_id: AdapterId::new("mock-b"),
                    traffic_fraction: 1.0,
                },
            )
            .await;

        let resp = router
            .route(PortId::new("inference"), build_request())
            .await
            .expect("complete");
        // Caller still sees the incumbent's response.
        match resp.content.first().unwrap() {
            hex_core::domain::messages::ContentBlock::Text { text } => {
                assert_eq!(text, "hello from A");
            }
            other => panic!("unexpected content block: {:?}", other),
        }
        // One shadow_sample row recorded, with this ticket id and seq=1.
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].0, ticket_id.to_string());
        assert_eq!(samples[0].1, 1);
        assert!(samples[0].2, "both adapters succeeded → agreed=true");
    }

    #[tokio::test]
    async fn route_skips_shadow_when_sample_misses() {
        // RNG always returns 1.0 (>= any fraction in [0,1]) → never shadow.
        let (router, state, ticket_id) = build_router(Arc::new(|| 1.0)).await;
        router
            .begin_shadow(
                PortId::new("inference"),
                ActiveShadowTicket {
                    ticket_id: ticket_id.to_string(),
                    candidate_adapter_id: AdapterId::new("mock-b"),
                    traffic_fraction: 0.5,
                },
            )
            .await;
        let _ = router
            .route(PortId::new("inference"), build_request())
            .await
            .expect("complete");
        assert_eq!(state.samples.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn shadow_router_inference_adapter_delegates_to_router() {
        // Wrapper consumers (workplan_executor, agent_manager) hand the
        // wrapped Arc to ScaffoldedDispatch / similar — they keep calling
        // .complete() and the substrate intercepts transparently. This
        // test confirms the wrapper actually goes through the router.
        let (router, state, _ticket_id) = build_router(Arc::new(|| 0.0)).await;
        // The router has mock-a as the live binding. The fallback we
        // hand to the adapter is irrelevant for routed calls (the router
        // resolves its own live binding) — so use a sentinel that would
        // panic if invoked unexpectedly.
        struct PanicInference;
        #[async_trait]
        impl IInferencePort for PanicInference {
            async fn complete(
                &self,
                _: InferenceRequest,
            ) -> Result<hex_core::ports::inference::InferenceResponse, InferenceError> {
                panic!("fallback should not be invoked when the router is wired");
            }
            async fn stream(
                &self,
                _: InferenceRequest,
            ) -> Result<
                Box<
                    dyn hex_core::ports::inference::futures_stream::Stream<
                            Item = hex_core::ports::inference::StreamChunk,
                        > + Send
                        + Unpin,
                >,
                InferenceError,
            > {
                Err(InferenceError::ProviderUnavailable("test".into()))
            }
            async fn health(
                &self,
            ) -> Result<hex_core::ports::inference::HealthStatus, InferenceError> {
                Err(InferenceError::ProviderUnavailable("test".into()))
            }
            fn capabilities(
                &self,
            ) -> hex_core::ports::inference::InferenceCapabilities {
                hex_core::ports::inference::InferenceCapabilities {
                    models: vec![],
                    supports_tool_use: false,
                    supports_thinking: false,
                    supports_caching: false,
                    supports_streaming: false,
                    max_context_tokens: 0,
                    cost_per_mtok_input: 0.0,
                    cost_per_mtok_output: 0.0,
                }
            }
        }
        let adapter = ShadowRouterInferenceAdapter::new(
            router,
            Arc::new(PanicInference) as Arc<dyn IInferencePort>,
        );
        let resp = adapter
            .complete(build_request())
            .await
            .expect("router serves the call");
        match resp.content.first().unwrap() {
            hex_core::domain::messages::ContentBlock::Text { text } => {
                assert_eq!(text, "hello from A", "router resolved live binding mock-a");
            }
            other => panic!("unexpected: {:?}", other),
        }
        // No samples recorded because no shadow ticket is active.
        assert_eq!(state.samples.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn end_shadow_clears_active_ticket() {
        let (router, state, ticket_id) = build_router(Arc::new(|| 0.0)).await;
        router
            .begin_shadow(
                PortId::new("inference"),
                ActiveShadowTicket {
                    ticket_id: ticket_id.to_string(),
                    candidate_adapter_id: AdapterId::new("mock-b"),
                    traffic_fraction: 1.0,
                },
            )
            .await;
        router.end_shadow(&PortId::new("inference")).await;
        let _ = router
            .route(PortId::new("inference"), build_request())
            .await
            .expect("complete");
        // No samples recorded after end_shadow.
        assert_eq!(state.samples.lock().unwrap().len(), 0);
    }
}
