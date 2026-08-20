//! L4 Shrinkage daemon — re-bound from "delete dead code" to "delete
//! adapters the substrate has not routed traffic to within an idle
//! window" (ADR-2026-04-26-1311 L4, ADR-2026-04-26-1500 C6, this turn's L4 ship).
//!
//! On each tick the daemon asks the `ShadowRouter` for handles eligible
//! for shrinkage (registered, not bound, not an active shadow candidate,
//! either never-routed-to or routed before `now - idle_window`) and
//! unregisters each. The unregistered handle's `Arc` is dropped if no
//! other reference exists — that's the substrate's structural pressure
//! against accretion: an adapter that doesn't earn its keep through
//! routed traffic gets evicted from the runtime.
//!
//! Process-local timestamps. A nexus restart resets the survey window,
//! which is fine because shrinkage is a soft-pressure tool, not a
//! correctness gate. The next tick after restart treats every handle as
//! "never routed" → eligible after one idle_window passes.

use std::sync::Arc;
use std::time::Duration;

use crate::orchestration::shadow_router::ShadowRouter;

pub struct ShrinkageDaemon {
    router: Arc<ShadowRouter>,
    idle_window: Duration,
}

#[derive(Debug, Default)]
pub struct ShrinkageTickReport {
    pub considered: usize,
    pub shrunk: Vec<String>,
}

impl ShrinkageDaemon {
    pub fn new(router: Arc<ShadowRouter>, idle_window: Duration) -> Self {
        Self { router, idle_window }
    }

    pub async fn tick(&self) -> ShrinkageTickReport {
        let mut report = ShrinkageTickReport::default();
        let candidates = self.router.shrinkable_adapters(self.idle_window).await;
        report.considered = candidates.len();
        for adapter_id in candidates {
            if self.router.unregister_handle(&adapter_id).await {
                report.shrunk.push(adapter_id.0);
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use hex_core::composition::{
        AdapterId, AdapterManifest, CompositionSwap, InMemoryComposition, PortId, PortRegistry,
    };
    use hex_core::ports::inference::mock::MockInferencePort;
    use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};

    use crate::adapters::spacetime_composition::{
        AsyncRuntimeComposition, SpacetimeRuntimeComposition,
    };
    use crate::orchestration::shadow_router::{ActiveShadowTicket, ShadowRouter};
    use crate::ports::state::{
        ISwapTicketStatePort, ShadowSampleRecord, StateError, SwapTicketRecord,
    };

    /// Permissive stub state — accepts every reducer call.
    #[derive(Default)]
    struct PermissiveState {
        _sentinel: StdMutex<()>,
    }

    #[async_trait]
    impl ISwapTicketStatePort for PermissiveState {
        async fn swap_ticket_create(
            &self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str,
            _: f32, _: u64, _: &str, _: &str,
        ) -> Result<(), StateError> { Ok(()) }
        async fn swap_ticket_transition(&self, _: &str, _: &str, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn swap_ticket_set_shadow_started(&self, _: &str, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn swap_ticket_set_config(&self, _: &str, _: &str, _: f32, _: u64, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn shadow_sample_record(
            &self, _: &str, _: u64, _: &str, _: &str, _: &str, _: &str, _: bool, _: &str, _: &str,
        ) -> Result<(), StateError> { Ok(()) }
        async fn shadow_tickets_due(&self, _: &str) -> Result<Vec<SwapTicketRecord>, StateError> { Ok(vec![]) }
        async fn shadow_samples_for(&self, _: &str) -> Result<Vec<ShadowSampleRecord>, StateError> { Ok(vec![]) }
        async fn shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, StateError> { Ok(vec![]) }
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

    /// Setup with mock-a as live binding and mock-b registered as a
    /// "leftover" candidate (registered but neither bound nor an active
    /// shadow). Returns the router so tests can drive it.
    async fn setup() -> Arc<ShadowRouter> {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("mock-a"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let state = Arc::new(PermissiveState::default());
        let comp = Arc::new(SpacetimeRuntimeComposition::new(
            InMemoryComposition::new(reg),
            state.clone(),
            "test-project",
        ));
        let router = Arc::new(ShadowRouter::new(comp, state));
        let mock_a: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("a"));
        let mock_b: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("b"));
        router.register_handle(AdapterId::new("mock-a"), mock_a).await;
        router.register_handle(AdapterId::new("mock-b"), mock_b).await;
        router
    }

    #[tokio::test]
    async fn shrinkage_daemon_evicts_never_routed_unbound_handles() {
        let router = setup().await;
        // mock-b is registered but neither bound nor part of an active
        // shadow nor ever routed to. With a 0-duration window it is
        // immediately eligible.
        let daemon = ShrinkageDaemon::new(router.clone(), Duration::from_secs(0));
        let report = daemon.tick().await;
        assert_eq!(report.shrunk, vec!["mock-b".to_string()]);
        // Re-tick — already gone, nothing to shrink.
        let second = daemon.tick().await;
        assert!(second.shrunk.is_empty());
    }

    #[tokio::test]
    async fn shrinkage_daemon_does_not_evict_bound_adapter() {
        let router = setup().await;
        // mock-a is the live binding. Even with idle_window=0 it must NOT
        // be shrinkable.
        let daemon = ShrinkageDaemon::new(router.clone(), Duration::from_secs(0));
        let report = daemon.tick().await;
        assert!(!report.shrunk.contains(&"mock-a".to_string()));
    }

    #[tokio::test]
    async fn shrinkage_daemon_does_not_evict_active_shadow_candidate() {
        let router = setup().await;
        router
            .begin_shadow(
                PortId::new("inference"),
                ActiveShadowTicket {
                    ticket_id: "t1".into(),
                    candidate_adapter_id: AdapterId::new("mock-b"),
                    traffic_fraction: 1.0,
                },
            )
            .await;
        let daemon = ShrinkageDaemon::new(router.clone(), Duration::from_secs(0));
        let report = daemon.tick().await;
        // mock-b is an active shadow candidate → exempt.
        assert!(!report.shrunk.contains(&"mock-b".to_string()));
        assert!(!report.shrunk.contains(&"mock-a".to_string()));
    }

    #[tokio::test]
    async fn shrinkage_daemon_respects_idle_window() {
        let router = setup().await;
        // Touch mock-b's routed_at by routing a call to the live (mock-a)
        // — that bumps mock-a's timestamp, NOT mock-b. mock-b remains
        // never-routed → eligible at any window.
        let _ = router
            .route(PortId::new("inference"), build_request())
            .await
            .unwrap();
        // 1-day window: mock-a was just routed (live). mock-a is bound so
        // immune anyway. mock-b is never-routed → still eligible regardless
        // of window.
        let daemon = ShrinkageDaemon::new(router.clone(), Duration::from_secs(86400));
        let report = daemon.tick().await;
        assert_eq!(report.shrunk, vec!["mock-b".to_string()]);
    }

    #[tokio::test]
    async fn shrinkage_daemon_keeps_recently_routed_unbound_handle() {
        // Set up a scenario where mock-b is unbound but was recently
        // routed to (via end_shadow leftover). Daemon with a long window
        // must keep it.
        let router = setup().await;
        // Drive a shadow → end_shadow flow so mock-b is routed-to (during
        // shadow) but then no longer the active candidate.
        router
            .begin_shadow(
                PortId::new("inference"),
                ActiveShadowTicket {
                    ticket_id: "t1".into(),
                    candidate_adapter_id: AdapterId::new("mock-b"),
                    traffic_fraction: 1.0,
                },
            )
            .await;
        // Force always-shadow RNG so the call mirrors and mock-b is touched.
        let router = Arc::new(
            ShadowRouter::new(
                Arc::new(SpacetimeRuntimeComposition::new(
                    {
                        let mut reg = PortRegistry::new();
                        reg.bind(
                            PortId::new("inference"),
                            AdapterId::new("mock-a"),
                            Arc::new(()) as Arc<dyn Any + Send + Sync>,
                        );
                        InMemoryComposition::new(reg)
                    },
                    Arc::new(PermissiveState::default()),
                    "test-project",
                )),
                Arc::new(PermissiveState::default()),
            )
            .with_rng(Arc::new(|| 0.0)),
        );
        let mock_a: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("a"));
        let mock_b: Arc<dyn IInferencePort> =
            Arc::new(MockInferencePort::with_response("b"));
        router.register_handle(AdapterId::new("mock-a"), mock_a).await;
        router.register_handle(AdapterId::new("mock-b"), mock_b).await;
        router
            .begin_shadow(
                PortId::new("inference"),
                ActiveShadowTicket {
                    ticket_id: "t1".into(),
                    candidate_adapter_id: AdapterId::new("mock-b"),
                    traffic_fraction: 1.0,
                },
            )
            .await;
        // Route once — mirrors → both mock-a + mock-b get touched.
        let _ = router
            .route(PortId::new("inference"), build_request())
            .await
            .unwrap();
        router.end_shadow(&PortId::new("inference")).await;

        // 1-day window. mock-b is now unbound and not active, but its
        // routed_at is fresh → must NOT be shrunk.
        let daemon = ShrinkageDaemon::new(router.clone(), Duration::from_secs(86400));
        let report = daemon.tick().await;
        assert!(
            !report.shrunk.contains(&"mock-b".to_string()),
            "recently-routed unbound handle must not be shrunk under a long window; got {:?}",
            report.shrunk
        );

        // 0-second window: now mock-b is eligible because the elapsed
        // time since routed_at is >= 0.
        let aggressive = ShrinkageDaemon::new(router, Duration::from_secs(0));
        let report = aggressive.tick().await;
        assert!(report.shrunk.contains(&"mock-b".to_string()));
    }

    #[tokio::test]
    async fn unregister_returns_false_for_unknown_handle() {
        let router = setup().await;
        let removed = router.unregister_handle(&AdapterId::new("never-registered")).await;
        assert!(!removed);
    }
}
