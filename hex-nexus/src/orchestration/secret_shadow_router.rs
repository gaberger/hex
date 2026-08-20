//! `SecretShadowRouter` — second-port substrate migration
//! (ADR-2026-04-26-1500 P10 first concrete second port).
//!
//! Mirrors `ISecretPort::resolve_secret` calls between the live secret
//! adapter and a candidate. Uses the port-agnostic primitives from
//! `shadow_decision` (decision logic, RNG, routing tracker) so the only
//! per-port code here is:
//!
//! 1. The trait this router dispatches against (`ISecretPort`).
//! 2. The handle map type (`Arc<dyn ISecretPort>`).
//! 3. The metrics shape recorded into `shadow_sample` (today: just `error`
//!    bool — secrets are short strings so latency comparison isn't the
//!    interesting signal; correctness is).
//! 4. The agreement model: `agreed = (incumbent_value == candidate_value)`
//!    when both succeed; `agreed = false` if either errors. This is
//!    *stricter* than the inference shadow router (which counts both-Ok
//!    as agreement). Secrets returning the same key from two adapters
//!    must produce the same value or something is wrong.
//!
//! Wiring into a consumer (e.g. agent_manager's secret resolution path)
//! follows the same pattern as the inference opt-in: hand the consumer
//! an `Arc<dyn ISecretPort>` that internally goes through the router. A
//! later turn ships the wrapper adapter analogous to
//! `ShadowRouterInferenceAdapter`. Today this module ships the router
//! itself + tests; consumer rewires are deferred behind an explicit go.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use hex_core::composition::{AdapterId, PortId};
use hex_core::ports::secret::{ISecretPort, SecretError};
use tokio::sync::{Mutex, RwLock};

use crate::adapters::spacetime_composition::{
    AsyncRuntimeComposition, SpacetimeRuntimeComposition,
};
use crate::orchestration::shadow_decision::{
    default_rng, shadow_decision, ActiveTicket, AdapterRoutingTracker, SampleRng, ShadowDecision,
};
use crate::ports::state::ISwapTicketStatePort;

pub struct SecretShadowRouter {
    comp: Arc<SpacetimeRuntimeComposition>,
    state: Arc<dyn ISwapTicketStatePort>,
    handles: RwLock<BTreeMap<AdapterId, Arc<dyn ISecretPort>>>,
    active_tickets: RwLock<BTreeMap<PortId, ActiveTicket>>,
    call_seqs: Mutex<BTreeMap<String, u64>>,
    rng: SampleRng,
    tracker: AdapterRoutingTracker,
}

impl SecretShadowRouter {
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
            rng: default_rng(),
            tracker: AdapterRoutingTracker::new(),
        }
    }

    pub fn with_rng(mut self, rng: SampleRng) -> Self {
        self.rng = rng;
        self
    }

    pub async fn register_handle(&self, adapter_id: AdapterId, handle: Arc<dyn ISecretPort>) {
        self.handles.write().await.insert(adapter_id, handle);
    }

    pub async fn begin_shadow(&self, port: PortId, ticket: ActiveTicket) {
        self.active_tickets.write().await.insert(port, ticket);
    }

    pub async fn end_shadow(&self, port: &PortId) {
        self.active_tickets.write().await.remove(port);
    }

    /// Number of registered handles. Used by /api/substrate/status for
    /// per-port operator visibility — symmetric with `ShadowRouter`'s
    /// method of the same name.
    pub async fn handle_count(&self) -> usize {
        self.handles.read().await.len()
    }

    /// Number of active in-memory shadow tickets (one per port). Symmetric
    /// with `ShadowRouter`'s method of the same name.
    pub async fn active_shadow_count(&self) -> usize {
        self.active_tickets.read().await.len()
    }

    async fn next_seq(&self, ticket_id: &str) -> u64 {
        let mut seqs = self.call_seqs.lock().await;
        let seq = seqs.entry(ticket_id.to_string()).or_insert(0);
        *seq += 1;
        *seq
    }

    pub async fn resolve_secret(&self, port: PortId, key: &str) -> Result<String, SecretError> {
        let live_id = self
            .comp
            .binding_id(&port)
            .ok_or_else(|| SecretError::VaultUnavailable(format!("port {} unbound", port.0)))?;
        let live_handle = {
            let handles = self.handles.read().await;
            handles
                .get(&live_id)
                .cloned()
                .ok_or_else(|| SecretError::VaultUnavailable(format!("handle {} missing", live_id.0)))?
        };

        let active = self.active_tickets.read().await.get(&port).cloned();
        let decision = shadow_decision(active.as_ref(), (self.rng)());

        match decision {
            ShadowDecision::LiveOnly => {
                self.tracker.mark_routed(&live_id).await;
                live_handle.resolve_secret(key).await
            }
            ShadowDecision::Mirror {
                ticket_id,
                candidate_adapter_id,
            } => {
                let candidate_handle = {
                    let h = self.handles.read().await;
                    h.get(&candidate_adapter_id).cloned()
                };
                let Some(candidate_handle) = candidate_handle else {
                    tracing::warn!(
                        ticket = %ticket_id,
                        candidate = %candidate_adapter_id.0,
                        "secret-shadow: candidate handle missing; live-only fallback"
                    );
                    self.tracker.mark_routed(&live_id).await;
                    return live_handle.resolve_secret(key).await;
                };

                self.tracker.mark_routed(&live_id).await;
                self.tracker.mark_routed(&candidate_adapter_id).await;
                let key_clone = key.to_string();
                let (incumbent_res, candidate_res) = tokio::join!(
                    live_handle.resolve_secret(key),
                    candidate_handle.resolve_secret(&key_clone),
                );

                // Strict agreement: both Ok AND values match. For secrets
                // this is the right call — a vault that returns a
                // different value for the same key is a divergence we
                // want flagged.
                let (agreed, reason) = match (&incumbent_res, &candidate_res) {
                    (Ok(a), Ok(b)) if a == b => (true, String::new()),
                    (Ok(_), Ok(_)) => (false, "value mismatch between incumbent and candidate".into()),
                    (Err(e), _) => (false, format!("incumbent error: {}", e)),
                    (_, Err(e)) => (false, format!("candidate error: {}", e)),
                };

                let call_seq = self.next_seq(&ticket_id).await;
                // Deliberately avoid logging secret VALUES — record only
                // ok/err shape so the dashboard surface stays scrubbable.
                let inc_json = serde_json::to_string(&serde_json::json!({
                    "ok": incumbent_res.is_ok(),
                }))
                .unwrap_or_else(|_| "{}".into());
                let cand_json = serde_json::to_string(&serde_json::json!({
                    "ok": candidate_res.is_ok(),
                }))
                .unwrap_or_else(|_| "{}".into());

                if let Err(e) = self
                    .state
                    .shadow_sample_record(
                        &ticket_id,
                        call_seq,
                        &live_id.0,
                        &candidate_adapter_id.0,
                        &inc_json,
                        &cand_json,
                        agreed,
                        &reason,
                        &Utc::now().to_rfc3339(),
                    )
                    .await
                {
                    tracing::warn!(error = %e, "secret-shadow: shadow_sample_record failed; sample dropped");
                }

                incumbent_res
            }
        }
    }
}

/// `ISecretPort` adapter that delegates `resolve_secret` calls through a
/// `SecretShadowRouter`. Consumers hold an `Arc<dyn ISecretPort>` and
/// keep calling `.resolve_secret()` exactly as before; the substrate
/// intercept is invisible. `claim_secrets`/`grant_secret`/`revoke_secret`
/// pass straight through to the fallback — substrate doesn't intercept
/// these today (only the read path is shadow-meaningful).
///
/// Mirrors `ShadowRouterInferenceAdapter` for `IInferencePort`. Consumer
/// opt-in pattern: hand the consumer this wrapper instead of a raw
/// secret adapter and they're substrate-aware without any call-site
/// changes.
pub struct ShadowRouterSecretAdapter {
    router: Arc<SecretShadowRouter>,
    fallback: Arc<dyn ISecretPort>,
    port: PortId,
}

impl ShadowRouterSecretAdapter {
    pub fn new(
        router: Arc<SecretShadowRouter>,
        fallback: Arc<dyn ISecretPort>,
        port: PortId,
    ) -> Self {
        Self { router, fallback, port }
    }
}

#[async_trait::async_trait]
impl ISecretPort for ShadowRouterSecretAdapter {
    async fn resolve_secret(&self, key: &str) -> Result<String, SecretError> {
        self.router.resolve_secret(self.port.clone(), key).await
    }

    async fn claim_secrets(
        &self,
        agent_id: &str,
    ) -> Result<hex_core::domain::secret_grant::ClaimResult, SecretError> {
        self.fallback.claim_secrets(agent_id).await
    }

    async fn grant_secret(
        &self,
        grant: &hex_core::domain::secret_grant::SecretGrant,
    ) -> Result<(), SecretError> {
        self.fallback.grant_secret(grant).await
    }

    async fn revoke_secret(&self, agent_id: &str, key: &str) -> Result<(), SecretError> {
        self.fallback.revoke_secret(agent_id, key).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    use async_trait::async_trait;
    use hex_core::composition::{AdapterId, AdapterManifest, CompositionSwap, InMemoryComposition, PortRegistry};
    use hex_core::domain::secret_grant::{ClaimResult, SecretGrant};

    use crate::adapters::spacetime_composition::AsyncRuntimeComposition;
    use crate::ports::state::{ShadowSampleRecord, StateError, SwapTicketRecord};

    /// Test secret port — returns canned values per key.
    struct StubSecret {
        values: std::sync::Mutex<std::collections::HashMap<String, String>>,
        fail: bool,
    }
    impl StubSecret {
        fn with(values: &[(&str, &str)]) -> Self {
            let mut m = std::collections::HashMap::new();
            for (k, v) in values {
                m.insert((*k).into(), (*v).into());
            }
            Self {
                values: std::sync::Mutex::new(m),
                fail: false,
            }
        }
        fn failing() -> Self {
            Self {
                values: std::sync::Mutex::new(std::collections::HashMap::new()),
                fail: true,
            }
        }
    }
    #[async_trait]
    impl ISecretPort for StubSecret {
        async fn resolve_secret(&self, key: &str) -> Result<String, SecretError> {
            if self.fail {
                return Err(SecretError::VaultUnavailable("stub failing".into()));
            }
            self.values
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .ok_or_else(|| SecretError::NotFound(key.into()))
        }
        async fn claim_secrets(&self, _: &str) -> Result<ClaimResult, SecretError> {
            Ok(ClaimResult {
                secrets: std::collections::HashMap::new(),
                expires_in: 0,
            })
        }
        async fn grant_secret(&self, _: &SecretGrant) -> Result<(), SecretError> {
            Ok(())
        }
        async fn revoke_secret(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct PermissiveState {
        samples: std::sync::Mutex<Vec<(String, u64, bool, String)>>, // (ticket, seq, agreed, reason)
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
            &self, ticket_id: &str, seq: u64, _: &str, _: &str, _: &str, _: &str, agreed: bool, reason: &str, _: &str,
        ) -> Result<(), StateError> {
            self.samples.lock().unwrap().push((ticket_id.into(), seq, agreed, reason.into()));
            Ok(())
        }
        async fn shadow_tickets_due(&self, _: &str) -> Result<Vec<SwapTicketRecord>, StateError> { Ok(vec![]) }
        async fn shadow_samples_for(&self, _: &str) -> Result<Vec<ShadowSampleRecord>, StateError> { Ok(vec![]) }
        async fn shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, StateError> { Ok(vec![]) }
    }

    async fn setup(rng: SampleRng) -> (Arc<SecretShadowRouter>, Arc<PermissiveState>) {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("secret"),
            AdapterId::new("env-vault"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let state = Arc::new(PermissiveState::default());
        let comp = Arc::new(SpacetimeRuntimeComposition::new(
            InMemoryComposition::new(reg),
            state.clone(),
            "test-project",
        ));
        // Propose so the in-memory composition has a candidate to mirror to.
        let _ticket = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("secret"),
                new_adapter_id: AdapterId::new("hashicorp-vault"),
                manifest: AdapterManifest {
                    adapter_id: AdapterId::new("hashicorp-vault"),
                    port: PortId::new("secret"),
                    version: "0.1.0".into(),
                    deps: vec![],
                },
            })
            .await
            .unwrap();
        let router = Arc::new(SecretShadowRouter::new(comp, state.clone()).with_rng(rng));
        let env: Arc<dyn ISecretPort> = Arc::new(StubSecret::with(&[("API_KEY", "secret-value-from-env")]));
        let vault: Arc<dyn ISecretPort> = Arc::new(StubSecret::with(&[("API_KEY", "secret-value-from-env")])); // matching
        router.register_handle(AdapterId::new("env-vault"), env).await;
        router.register_handle(AdapterId::new("hashicorp-vault"), vault).await;
        (router, state)
    }

    #[tokio::test]
    async fn live_only_when_no_active_ticket() {
        let (router, state) = setup(Arc::new(|| 0.0)).await;
        let v = router
            .resolve_secret(PortId::new("secret"), "API_KEY")
            .await
            .unwrap();
        assert_eq!(v, "secret-value-from-env");
        assert!(state.samples.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mirrors_and_records_agreed_when_values_match() {
        let (router, state) = setup(Arc::new(|| 0.0)).await;
        router
            .begin_shadow(
                PortId::new("secret"),
                ActiveTicket {
                    ticket_id: "tx".into(),
                    candidate_adapter_id: AdapterId::new("hashicorp-vault"),
                    traffic_fraction: 1.0,
                },
            )
            .await;
        let v = router
            .resolve_secret(PortId::new("secret"), "API_KEY")
            .await
            .unwrap();
        assert_eq!(v, "secret-value-from-env", "caller sees incumbent's value");
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].0, "tx");
        assert_eq!(samples[0].1, 1);
        assert!(samples[0].2, "values matched → agreed=true");
    }

    #[tokio::test]
    async fn mirrors_and_records_disagreement_on_value_mismatch() {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("secret"),
            AdapterId::new("env-vault"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let state = Arc::new(PermissiveState::default());
        let comp = Arc::new(SpacetimeRuntimeComposition::new(
            InMemoryComposition::new(reg),
            state.clone(),
            "test-project",
        ));
        comp.propose_swap_async(CompositionSwap {
            port: PortId::new("secret"),
            new_adapter_id: AdapterId::new("hashicorp-vault"),
            manifest: AdapterManifest {
                adapter_id: AdapterId::new("hashicorp-vault"),
                port: PortId::new("secret"),
                version: "0.1.0".into(),
                deps: vec![],
            },
        })
        .await
        .unwrap();
        let router = Arc::new(SecretShadowRouter::new(comp, state.clone()).with_rng(Arc::new(|| 0.0)));
        // Different values for the same key — vault has been rotated but
        // env still has the old value. Substrate must catch it.
        let env: Arc<dyn ISecretPort> = Arc::new(StubSecret::with(&[("API_KEY", "old-value")]));
        let vault: Arc<dyn ISecretPort> = Arc::new(StubSecret::with(&[("API_KEY", "new-rotated-value")]));
        router.register_handle(AdapterId::new("env-vault"), env).await;
        router.register_handle(AdapterId::new("hashicorp-vault"), vault).await;
        router
            .begin_shadow(
                PortId::new("secret"),
                ActiveTicket {
                    ticket_id: "rx".into(),
                    candidate_adapter_id: AdapterId::new("hashicorp-vault"),
                    traffic_fraction: 1.0,
                },
            )
            .await;
        let v = router
            .resolve_secret(PortId::new("secret"), "API_KEY")
            .await
            .unwrap();
        // Caller still sees incumbent's old value (no caller-visible behaviour change during shadow).
        assert_eq!(v, "old-value");
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), 1);
        assert!(!samples[0].2, "values differed → agreed=false");
        assert!(samples[0].3.contains("value mismatch"));
    }

    #[tokio::test]
    async fn rng_above_fraction_skips_shadow() {
        let (router, state) = setup(Arc::new(|| 1.0)).await;
        router
            .begin_shadow(
                PortId::new("secret"),
                ActiveTicket {
                    ticket_id: "ty".into(),
                    candidate_adapter_id: AdapterId::new("hashicorp-vault"),
                    traffic_fraction: 0.5,
                },
            )
            .await;
        router
            .resolve_secret(PortId::new("secret"), "API_KEY")
            .await
            .unwrap();
        assert!(state.samples.lock().unwrap().is_empty());
    }

    // ── ShadowRouterSecretAdapter tests ─────────────────────

    #[tokio::test]
    async fn wrapper_adapter_delegates_resolve_through_router() {
        let (router, _state) = setup(Arc::new(|| 0.0)).await;
        // Wrapper's fallback should NOT be invoked when the router has
        // a live binding — the router resolves and returns from its own
        // handle map. Use a panicking fallback to prove it.
        struct PanicSecret;
        #[async_trait]
        impl ISecretPort for PanicSecret {
            async fn resolve_secret(&self, _: &str) -> Result<String, SecretError> {
                panic!("fallback should not be invoked when router is wired");
            }
            async fn claim_secrets(&self, _: &str) -> Result<ClaimResult, SecretError> {
                Ok(ClaimResult {
                    secrets: std::collections::HashMap::new(),
                    expires_in: 0,
                })
            }
            async fn grant_secret(&self, _: &SecretGrant) -> Result<(), SecretError> {
                Ok(())
            }
            async fn revoke_secret(&self, _: &str, _: &str) -> Result<(), SecretError> {
                Ok(())
            }
        }
        let adapter = ShadowRouterSecretAdapter::new(
            router,
            Arc::new(PanicSecret) as Arc<dyn ISecretPort>,
            PortId::new("secret"),
        );
        let v = adapter.resolve_secret("API_KEY").await.unwrap();
        assert_eq!(v, "secret-value-from-env");
    }

    #[tokio::test]
    async fn wrapper_adapter_passes_grant_through_to_fallback() {
        // grant_secret/revoke_secret/claim_secrets are NOT routed through
        // the substrate (only resolve_secret is shadow-meaningful). They
        // must hit the fallback unchanged.
        let (router, _state) = setup(Arc::new(|| 0.0)).await;
        struct CountingSecret {
            grants: std::sync::Mutex<u32>,
        }
        #[async_trait]
        impl ISecretPort for CountingSecret {
            async fn resolve_secret(&self, _: &str) -> Result<String, SecretError> {
                Err(SecretError::NotFound("unused".into()))
            }
            async fn claim_secrets(&self, _: &str) -> Result<ClaimResult, SecretError> {
                Ok(ClaimResult {
                    secrets: std::collections::HashMap::new(),
                    expires_in: 0,
                })
            }
            async fn grant_secret(&self, _: &SecretGrant) -> Result<(), SecretError> {
                *self.grants.lock().unwrap() += 1;
                Ok(())
            }
            async fn revoke_secret(&self, _: &str, _: &str) -> Result<(), SecretError> {
                Ok(())
            }
        }
        let counter = Arc::new(CountingSecret {
            grants: std::sync::Mutex::new(0),
        });
        let adapter = ShadowRouterSecretAdapter::new(
            router,
            counter.clone() as Arc<dyn ISecretPort>,
            PortId::new("secret"),
        );
        let grant = SecretGrant {
            agent_id: "a".into(),
            secret_key: "K".into(),
            purpose: hex_core::domain::secret_grant::GrantPurpose::Llm,
            granted_at: chrono::Utc::now().to_rfc3339(),
            expires_at: chrono::Utc::now().to_rfc3339(),
            claimed: false,
        };
        adapter.grant_secret(&grant).await.unwrap();
        assert_eq!(*counter.grants.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn caller_sees_incumbent_error_when_incumbent_fails() {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("secret"),
            AdapterId::new("env-vault"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let state = Arc::new(PermissiveState::default());
        let comp = Arc::new(SpacetimeRuntimeComposition::new(
            InMemoryComposition::new(reg),
            state.clone(),
            "test-project",
        ));
        comp.propose_swap_async(CompositionSwap {
            port: PortId::new("secret"),
            new_adapter_id: AdapterId::new("hashicorp-vault"),
            manifest: AdapterManifest {
                adapter_id: AdapterId::new("hashicorp-vault"),
                port: PortId::new("secret"),
                version: "0.1.0".into(),
                deps: vec![],
            },
        })
        .await
        .unwrap();
        let router = Arc::new(SecretShadowRouter::new(comp, state.clone()).with_rng(Arc::new(|| 0.0)));
        // Incumbent fails (e.g. env var unset). Caller must see incumbent's
        // error — that's the substrate's caller-visible-behaviour invariant.
        let env: Arc<dyn ISecretPort> = Arc::new(StubSecret::failing());
        let vault: Arc<dyn ISecretPort> = Arc::new(StubSecret::with(&[("API_KEY", "from-vault")]));
        router.register_handle(AdapterId::new("env-vault"), env).await;
        router.register_handle(AdapterId::new("hashicorp-vault"), vault).await;
        router
            .begin_shadow(
                PortId::new("secret"),
                ActiveTicket {
                    ticket_id: "tz".into(),
                    candidate_adapter_id: AdapterId::new("hashicorp-vault"),
                    traffic_fraction: 1.0,
                },
            )
            .await;
        let r = router
            .resolve_secret(PortId::new("secret"), "API_KEY")
            .await;
        assert!(r.is_err(), "caller must see incumbent's error during shadow");
        let samples = state.samples.lock().unwrap();
        assert_eq!(samples.len(), 1);
        assert!(!samples[0].2);
        assert!(samples[0].3.contains("incumbent error"));
    }
}
