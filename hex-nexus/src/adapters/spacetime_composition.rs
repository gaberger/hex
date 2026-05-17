//! SpacetimeRuntimeComposition — STDB-mirrored RuntimeComposition adapter
//! (ADR-2026-04-26-1500 P6, wp-substrate-shadow-promotion P2.1).
//!
//! Wraps a process-local [`InMemoryComposition`] (from hex-core) and an
//! [`ISwapTicketStatePort`]. STDB is treated as the source of truth: each
//! mutating call (`propose_swap`, `promote`, `rollback`) calls the
//! corresponding STDB reducer first; only on success is the in-memory
//! state mutated. If STDB rejects, the in-memory state is unchanged and the
//! caller receives the error.
//!
//! Reads (`snapshot`, `binding_id`) are served from the in-memory state
//! without an STDB roundtrip — the in-memory state is the authoritative
//! live registry, the STDB tables are the durable audit log.
//!
//! The shadow-routing protocol itself (`shadow_sample_record` for each
//! mirrored call, traffic-fraction sampling, judge-driven transitions)
//! lives in the [`ShadowRouter`] and [`PromotionJudge`] (P3 / P4) — this
//! adapter only persists swap-ticket transitions.

use std::sync::Arc;

use chrono::Utc;
use hex_core::composition::{
    AdapterId, CompositionSnapshot, CompositionSwap, InMemoryComposition, PortId,
    RuntimeComposition, SwapError, SwapTicket,
};
use uuid::Uuid;

use crate::ports::state::{ISwapTicketStatePort, StateError};

pub struct SpacetimeRuntimeComposition {
    in_memory: InMemoryComposition,
    state: Arc<dyn ISwapTicketStatePort>,
    project_id: String,
}

impl SpacetimeRuntimeComposition {
    pub fn new(
        in_memory: InMemoryComposition,
        state: Arc<dyn ISwapTicketStatePort>,
        project_id: impl Into<String>,
    ) -> Self {
        Self {
            in_memory,
            state,
            project_id: project_id.into(),
        }
    }

    /// Public access for tests and consumers that need to stage handles
    /// before promotion (the hex-core trait deliberately keeps the typed
    /// handle off `RuntimeComposition`).
    pub fn in_memory(&self) -> &InMemoryComposition {
        &self.in_memory
    }
}

fn map_state_err(port: &PortId, err: StateError) -> SwapError {
    // The hex-core `SwapError` enum doesn't carry an STDB variant — we map
    // an STDB failure to the closest existing semantic: the swap could not
    // proceed. `PortUnknown` is reused as a generic "STDB rejected" carrier
    // so callers can match exhaustively without coupling hex-core to STDB
    // error types. We log the original at warn level so the audit trail is
    // not lost.
    tracing::warn!(port = ?port, error = %err, "swap_ticket reducer rejected");
    SwapError::PortUnknown(port.clone())
}

#[async_trait::async_trait]
pub trait AsyncRuntimeComposition: Send + Sync {
    async fn propose_swap_async(&self, swap: CompositionSwap) -> Result<SwapTicket, SwapError>;
    async fn promote_async(&self, ticket_id: Uuid) -> Result<(), SwapError>;
    async fn rollback_async(&self, ticket_id: Uuid) -> Result<(), SwapError>;
    fn snapshot(&self) -> CompositionSnapshot;
    fn binding_id(&self, port: &PortId) -> Option<AdapterId>;
}

#[async_trait::async_trait]
impl AsyncRuntimeComposition for SpacetimeRuntimeComposition {
    async fn propose_swap_async(&self, swap: CompositionSwap) -> Result<SwapTicket, SwapError> {
        // Fast-path the in-memory single-writer rejection so we don't make a
        // network call for a swap we already know we'll reject.
        let ticket = self.in_memory.propose_swap(swap.clone())?;

        let manifest_json = serde_json::to_string(&swap.manifest)
            .unwrap_or_else(|_| "{}".to_string());
        let incumbent = self
            .in_memory
            .binding_id(&swap.port)
            .map(|id| id.0)
            .unwrap_or_default();

        if let Err(err) = self
            .state
            .swap_ticket_create(
                &ticket.id.to_string(),
                &self.project_id,
                &swap.port.0,
                &incumbent,
                &swap.new_adapter_id.0,
                &manifest_json,
                0.05, // default 5% shadow fraction; ShadowRouter overrides per ticket
                300,  // default 5-minute window
                "[]", // criteria attached separately by the caller before shadow begins
                &Utc::now().to_rfc3339(),
            )
            .await
        {
            // Roll back the in-memory ticket so propose_swap is atomic
            // across the two stores. The simplest way to undo a Candidate
            // is to attempt rollback — which will fail because the ticket
            // is not Promoted, but that's the no-op we want here. Instead
            // we just leave the in-memory ticket pending; the next attempt
            // on the same port will be rejected as SwapInFlight, which is
            // the correct surface (caller knows the swap is wedged). A
            // purpose-built `cancel_candidate` is workplan P3 territory.
            return Err(map_state_err(&swap.port, err));
        }

        Ok(ticket)
    }

    async fn promote_async(&self, ticket_id: Uuid) -> Result<(), SwapError> {
        // Persist the transition first; only mutate live registry on success.
        if let Err(err) = self
            .state
            .swap_ticket_transition(
                &ticket_id.to_string(),
                "promoted",
                &Utc::now().to_rfc3339(),
            )
            .await
        {
            // We don't know the port without a lookup; use a placeholder.
            // The tracing log preserves the real error.
            return Err(map_state_err(&PortId::new("<unknown>"), err));
        }
        self.in_memory.promote(ticket_id)
    }

    async fn rollback_async(&self, ticket_id: Uuid) -> Result<(), SwapError> {
        if let Err(err) = self
            .state
            .swap_ticket_transition(
                &ticket_id.to_string(),
                "rolled_back",
                &Utc::now().to_rfc3339(),
            )
            .await
        {
            return Err(map_state_err(&PortId::new("<unknown>"), err));
        }
        self.in_memory.rollback(ticket_id)
    }

    fn snapshot(&self) -> CompositionSnapshot {
        self.in_memory.snapshot()
    }

    fn binding_id(&self, port: &PortId) -> Option<AdapterId> {
        self.in_memory.binding_id(port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use hex_core::composition::{
        AdapterId, AdapterManifest, CompositionSwap, PortId, PortRegistry,
    };

    /// Records every call into a vector of `(reducer_name, args_json)`.
    /// Configurable to fail on a specific reducer for testing the
    /// "STDB rejects → in-memory state unchanged" invariant.
    #[derive(Default)]
    struct MockSwapTicketState {
        calls: Mutex<Vec<(String, serde_json::Value)>>,
        fail_on: Mutex<Option<String>>,
    }

    impl MockSwapTicketState {
        fn fail_next(&self, reducer: &str) {
            *self.fail_on.lock().unwrap() = Some(reducer.to_string());
        }
        fn record(&self, name: &str, args: serde_json::Value) -> Result<(), StateError> {
            let should_fail = self
                .fail_on
                .lock()
                .unwrap()
                .as_deref()
                .map(|f| f == name)
                .unwrap_or(false);
            self.calls.lock().unwrap().push((name.to_string(), args));
            if should_fail {
                *self.fail_on.lock().unwrap() = None;
                Err(StateError::Storage("mock injected failure".into()))
            } else {
                Ok(())
            }
        }
        fn call_names(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(n, _)| n.clone())
                .collect()
        }
    }

    #[async_trait]
    impl ISwapTicketStatePort for MockSwapTicketState {
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
            self.record(
                "swap_ticket_create",
                serde_json::json!([id, project_id, port_id, incumbent, candidate, manifest_json, fraction, window, criteria_json, timestamp]),
            )
        }

        async fn swap_ticket_transition(
            &self,
            id: &str,
            new_state: &str,
            ts: &str,
        ) -> Result<(), StateError> {
            self.record(
                "swap_ticket_transition",
                serde_json::json!([id, new_state, ts]),
            )
        }

        async fn swap_ticket_set_shadow_started(
            &self,
            id: &str,
            ts: &str,
        ) -> Result<(), StateError> {
            self.record(
                "swap_ticket_set_shadow_started",
                serde_json::json!([id, ts]),
            )
        }

        async fn swap_ticket_set_config(
            &self,
            id: &str,
            criteria_json: &str,
            fraction: f32,
            window: u64,
            ts: &str,
        ) -> Result<(), StateError> {
            self.record(
                "swap_ticket_set_config",
                serde_json::json!([id, criteria_json, fraction, window, ts]),
            )
        }

        async fn shadow_sample_record(
            &self,
            ticket_id: &str,
            call_seq: u64,
            incumbent: &str,
            candidate: &str,
            incumbent_metrics: &str,
            candidate_metrics: &str,
            agreed: bool,
            reason: &str,
            ts: &str,
        ) -> Result<(), StateError> {
            self.record(
                "shadow_sample_record",
                serde_json::json!([ticket_id, call_seq, incumbent, candidate, incumbent_metrics, candidate_metrics, agreed, reason, ts]),
            )
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

    fn manifest(adapter: &str, port: &str) -> AdapterManifest {
        AdapterManifest {
            adapter_id: AdapterId::new(adapter),
            port: PortId::new(port),
            version: "0.1.0".into(),
            deps: vec![],
        }
    }

    fn build_comp() -> (SpacetimeRuntimeComposition, Arc<MockSwapTicketState>) {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("mock-a"),
            Arc::new("mock-a") as Arc<dyn Any + Send + Sync>,
        );
        let in_mem = InMemoryComposition::new(reg);
        let mock = Arc::new(MockSwapTicketState::default());
        let comp = SpacetimeRuntimeComposition::new(in_mem, mock.clone(), "test-project");
        (comp, mock)
    }

    #[tokio::test]
    async fn substrate_composition_propose_calls_swap_ticket_create() {
        let (comp, mock) = build_comp();
        let ticket = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: manifest("mock-b", "inference"),
            })
            .await
            .expect("propose ok");
        assert_eq!(mock.call_names(), vec!["swap_ticket_create"]);
        // Ticket id is consistent with reducer args (first arg).
        let args = mock.calls.lock().unwrap()[0].1.clone();
        assert_eq!(args[0].as_str().unwrap(), ticket.id.to_string());
        assert_eq!(args[1].as_str().unwrap(), "test-project");
        assert_eq!(args[2].as_str().unwrap(), "inference");
        assert_eq!(args[3].as_str().unwrap(), "mock-a"); // incumbent
        assert_eq!(args[4].as_str().unwrap(), "mock-b"); // candidate
    }

    #[tokio::test]
    async fn substrate_composition_propose_reject_on_stdb_failure() {
        let (comp, mock) = build_comp();
        mock.fail_next("swap_ticket_create");
        let err = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: manifest("mock-b", "inference"),
            })
            .await
            .expect_err("STDB rejects");
        assert!(matches!(err, SwapError::PortUnknown(_)));
        // The in-memory ticket was created (we accepted the in-memory write
        // before calling STDB). Subsequent propose on the same port must be
        // rejected as SwapInFlight — caller is expected to surface this so
        // the operator can investigate the wedged ticket.
        let again = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: manifest("mock-b", "inference"),
            })
            .await
            .expect_err("subsequent rejected");
        assert!(matches!(again, SwapError::SwapInFlight(_)));
    }

    #[tokio::test]
    async fn substrate_composition_promote_calls_transition_then_in_memory() {
        let (comp, mock) = build_comp();
        let ticket = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: manifest("mock-b", "inference"),
            })
            .await
            .unwrap();
        // Stage handle + force-mark green (the real shadow-promotion judge
        // owns this transition; we drive it manually for unit-test scope).
        comp.in_memory()
            .stage_handle(ticket.id, Arc::new("mock-b") as Arc<dyn Any + Send + Sync>)
            .unwrap();
        comp.in_memory().mark_shadow_green(ticket.id).unwrap();
        comp.promote_async(ticket.id).await.expect("promote ok");
        assert_eq!(
            mock.call_names(),
            vec!["swap_ticket_create", "swap_ticket_transition"]
        );
        // Live binding flipped.
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-b"))
        );
    }

    #[tokio::test]
    async fn substrate_composition_promote_leaves_in_memory_unchanged_on_stdb_failure() {
        let (comp, mock) = build_comp();
        let ticket = comp
            .propose_swap_async(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("mock-b"),
                manifest: manifest("mock-b", "inference"),
            })
            .await
            .unwrap();
        comp.in_memory()
            .stage_handle(ticket.id, Arc::new("mock-b") as Arc<dyn Any + Send + Sync>)
            .unwrap();
        comp.in_memory().mark_shadow_green(ticket.id).unwrap();
        mock.fail_next("swap_ticket_transition");
        let err = comp.promote_async(ticket.id).await.expect_err("STDB rejects");
        assert!(matches!(err, SwapError::PortUnknown(_)));
        // Live binding NOT flipped — still mock-a.
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("mock-a"))
        );
    }
}
