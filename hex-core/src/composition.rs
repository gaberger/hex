//! C2 — Hot-swappable composition root API (per ADR-2026-04-26-1500).
//!
//! `RuntimeComposition` is the trait by which adapters are registered, swapped,
//! promoted, and rolled back at runtime. `InMemoryComposition` is the day-one
//! implementation that wraps a static binding map; the shadow-test promotion
//! protocol (C5, ADR P6) and STDB-backed persistence land in follow-on work.
//!
//! `PortRegistry` holds trait objects (`Arc<dyn Any + Send + Sync>`) and is
//! therefore deliberately *not* `Serialize`. The serializable view of a
//! composition is `CompositionSnapshot`, which captures the binding map
//! (`PortId -> AdapterId`) without the live adapter handles.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PortId(pub String);

impl PortId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AdapterId(pub String);

impl AdapterId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum SwapState {
    Candidate,
    Shadow,
    ShadowGreen,
    ShadowRed,
    Promoted,
    RolledBack,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AdapterManifest {
    pub adapter_id: AdapterId,
    pub port: PortId,
    pub version: String,
    pub deps: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CompositionSwap {
    pub port: PortId,
    pub new_adapter_id: AdapterId,
    pub manifest: AdapterManifest,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SwapTicket {
    pub id: Uuid,
    pub port: PortId,
    pub state: SwapState,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CompositionSnapshot {
    pub bindings: BTreeMap<PortId, AdapterId>,
    pub taken_at: DateTime<Utc>,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum SwapError {
    #[error("port not registered: {0:?}")]
    PortUnknown(PortId),
    #[error("a swap is already in flight for port {0:?}")]
    SwapInFlight(PortId),
    #[error("ticket {0} not found")]
    TicketUnknown(Uuid),
    #[error("ticket {0} is not eligible for promotion (state must be ShadowGreen)")]
    NotEligibleForPromotion(Uuid),
    #[error("no prior binding to roll back to for port {0:?}")]
    NothingToRollBack(PortId),
}

/// Live registry of port -> adapter handle. Not serializable (holds trait
/// objects). For a serializable view, use `CompositionSnapshot`.
#[derive(Default)]
pub struct PortRegistry {
    bindings: BTreeMap<PortId, (AdapterId, Arc<dyn Any + Send + Sync>)>,
}

impl PortRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bind(&mut self, port: PortId, adapter_id: AdapterId, handle: Arc<dyn Any + Send + Sync>) {
        self.bindings.insert(port, (adapter_id, handle));
    }

    pub fn resolve(&self, port: &PortId) -> Option<&Arc<dyn Any + Send + Sync>> {
        self.bindings.get(port).map(|(_id, h)| h)
    }

    pub fn binding_id(&self, port: &PortId) -> Option<&AdapterId> {
        self.bindings.get(port).map(|(id, _h)| id)
    }

    pub fn snapshot_ids(&self) -> BTreeMap<PortId, AdapterId> {
        self.bindings
            .iter()
            .map(|(p, (id, _h))| (p.clone(), id.clone()))
            .collect()
    }
}

pub trait RuntimeComposition: Send + Sync {
    fn snapshot(&self) -> CompositionSnapshot;
    fn propose_swap(&self, swap: CompositionSwap) -> Result<SwapTicket, SwapError>;
    fn promote(&self, ticket_id: Uuid) -> Result<(), SwapError>;
    fn rollback(&self, ticket_id: Uuid) -> Result<(), SwapError>;
    fn binding_id(&self, port: &PortId) -> Option<AdapterId>;
}

struct TicketRecord {
    ticket: SwapTicket,
    swap: CompositionSwap,
    new_handle: Option<Arc<dyn Any + Send + Sync>>,
    prior: Option<(AdapterId, Arc<dyn Any + Send + Sync>)>,
}

/// Day-one `RuntimeComposition` implementation: in-memory, single-writer, no
/// shadow-routing yet. Single-writer per port is enforced by rejecting a
/// `propose_swap` whose port already has a Candidate / Shadow / ShadowGreen
/// ticket open.
pub struct InMemoryComposition {
    registry: RwLock<PortRegistry>,
    tickets: Mutex<BTreeMap<Uuid, TicketRecord>>,
}

impl InMemoryComposition {
    pub fn new(registry: PortRegistry) -> Self {
        Self {
            registry: RwLock::new(registry),
            tickets: Mutex::new(BTreeMap::new()),
        }
    }

    /// Stage the actual adapter handle for a previously proposed ticket.
    /// Kept off the trait so `RuntimeComposition` does not have to carry a
    /// generic over adapter handle type.
    pub fn stage_handle(&self, ticket_id: Uuid, handle: Arc<dyn Any + Send + Sync>) -> Result<(), SwapError> {
        let mut tickets = self.tickets.lock().expect("tickets mutex poisoned");
        let rec = tickets
            .get_mut(&ticket_id)
            .ok_or(SwapError::TicketUnknown(ticket_id))?;
        rec.new_handle = Some(handle);
        Ok(())
    }

    /// Force-mark a ticket `ShadowGreen`. The real shadow-test judge (C5 / P6)
    /// will own this transition; for day one we expose it so tests and the
    /// initial migration can drive promotions deterministically.
    pub fn mark_shadow_green(&self, ticket_id: Uuid) -> Result<(), SwapError> {
        let mut tickets = self.tickets.lock().expect("tickets mutex poisoned");
        let rec = tickets
            .get_mut(&ticket_id)
            .ok_or(SwapError::TicketUnknown(ticket_id))?;
        rec.ticket.state = SwapState::ShadowGreen;
        Ok(())
    }

    fn port_has_open_ticket(tickets: &BTreeMap<Uuid, TicketRecord>, port: &PortId) -> bool {
        tickets.values().any(|rec| {
            rec.ticket.port == *port
                && matches!(
                    rec.ticket.state,
                    SwapState::Candidate | SwapState::Shadow | SwapState::ShadowGreen
                )
        })
    }
}

impl RuntimeComposition for InMemoryComposition {
    fn snapshot(&self) -> CompositionSnapshot {
        let registry = self.registry.read().expect("registry rwlock poisoned");
        CompositionSnapshot {
            bindings: registry.snapshot_ids(),
            taken_at: Utc::now(),
        }
    }

    fn propose_swap(&self, swap: CompositionSwap) -> Result<SwapTicket, SwapError> {
        let mut tickets = self.tickets.lock().expect("tickets mutex poisoned");
        if Self::port_has_open_ticket(&tickets, &swap.port) {
            return Err(SwapError::SwapInFlight(swap.port));
        }
        let ticket = SwapTicket {
            id: Uuid::new_v4(),
            port: swap.port.clone(),
            state: SwapState::Candidate,
            created_at: Utc::now(),
        };
        tickets.insert(
            ticket.id,
            TicketRecord {
                ticket: ticket.clone(),
                swap,
                new_handle: None,
                prior: None,
            },
        );
        Ok(ticket)
    }

    fn promote(&self, ticket_id: Uuid) -> Result<(), SwapError> {
        let mut tickets = self.tickets.lock().expect("tickets mutex poisoned");
        let rec = tickets
            .get_mut(&ticket_id)
            .ok_or(SwapError::TicketUnknown(ticket_id))?;
        if rec.ticket.state != SwapState::ShadowGreen {
            return Err(SwapError::NotEligibleForPromotion(ticket_id));
        }
        let new_handle = rec
            .new_handle
            .as_ref()
            .cloned()
            .ok_or_else(|| SwapError::PortUnknown(rec.ticket.port.clone()))?;

        let mut registry = self.registry.write().expect("registry rwlock poisoned");
        let port = rec.ticket.port.clone();
        let new_id = rec.swap.new_adapter_id.clone();

        let prior = registry
            .bindings
            .get(&port)
            .map(|(id, h)| (id.clone(), h.clone()));
        rec.prior = prior;

        registry.bind(port, new_id, new_handle);
        rec.ticket.state = SwapState::Promoted;
        Ok(())
    }

    fn rollback(&self, ticket_id: Uuid) -> Result<(), SwapError> {
        let mut tickets = self.tickets.lock().expect("tickets mutex poisoned");
        let rec = tickets
            .get_mut(&ticket_id)
            .ok_or(SwapError::TicketUnknown(ticket_id))?;
        if rec.ticket.state != SwapState::Promoted {
            return Err(SwapError::NotEligibleForPromotion(ticket_id));
        }
        let port = rec.ticket.port.clone();
        let mut registry = self.registry.write().expect("registry rwlock poisoned");
        match rec.prior.take() {
            Some((id, handle)) => {
                registry.bind(port, id, handle);
                rec.ticket.state = SwapState::RolledBack;
                Ok(())
            }
            None => Err(SwapError::NothingToRollBack(port)),
        }
    }

    fn binding_id(&self, port: &PortId) -> Option<AdapterId> {
        let registry = self.registry.read().expect("registry rwlock poisoned");
        registry.binding_id(port).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(port: &str, adapter: &str) -> AdapterManifest {
        AdapterManifest {
            adapter_id: AdapterId::new(adapter),
            port: PortId::new(port),
            version: "0.1.0".into(),
            deps: vec![],
        }
    }

    #[test]
    fn snapshot_reflects_initial_bindings() {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("anthropic-v1"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let comp = InMemoryComposition::new(reg);
        let snap = comp.snapshot();
        assert_eq!(snap.bindings.get(&PortId::new("inference")), Some(&AdapterId::new("anthropic-v1")));
    }

    #[test]
    fn propose_swap_rejects_concurrent_ticket_on_same_port() {
        let comp = InMemoryComposition::new(PortRegistry::new());
        let port = PortId::new("inference");
        let swap = CompositionSwap {
            port: port.clone(),
            new_adapter_id: AdapterId::new("openai-v1"),
            manifest: manifest("inference", "openai-v1"),
        };
        comp.propose_swap(swap.clone()).expect("first proposes");
        let err = comp.propose_swap(swap).expect_err("second rejected");
        assert_eq!(err, SwapError::SwapInFlight(port));
    }

    #[test]
    fn promote_requires_shadow_green_and_swaps_atomically() {
        let mut reg = PortRegistry::new();
        let port = PortId::new("inference");
        reg.bind(
            port.clone(),
            AdapterId::new("anthropic-v1"),
            Arc::new("anthropic") as Arc<dyn Any + Send + Sync>,
        );
        let comp = InMemoryComposition::new(reg);

        let ticket = comp
            .propose_swap(CompositionSwap {
                port: port.clone(),
                new_adapter_id: AdapterId::new("openai-v1"),
                manifest: manifest("inference", "openai-v1"),
            })
            .expect("propose");

        let err = comp.promote(ticket.id).expect_err("not eligible");
        assert!(matches!(err, SwapError::NotEligibleForPromotion(_)));

        comp.stage_handle(ticket.id, Arc::new("openai") as Arc<dyn Any + Send + Sync>)
            .unwrap();
        comp.mark_shadow_green(ticket.id).unwrap();
        comp.promote(ticket.id).expect("promotes");

        assert_eq!(comp.binding_id(&port), Some(AdapterId::new("openai-v1")));
    }

    #[test]
    fn rollback_restores_prior_binding() {
        let mut reg = PortRegistry::new();
        let port = PortId::new("inference");
        reg.bind(
            port.clone(),
            AdapterId::new("anthropic-v1"),
            Arc::new("anthropic") as Arc<dyn Any + Send + Sync>,
        );
        let comp = InMemoryComposition::new(reg);

        let ticket = comp
            .propose_swap(CompositionSwap {
                port: port.clone(),
                new_adapter_id: AdapterId::new("openai-v1"),
                manifest: manifest("inference", "openai-v1"),
            })
            .unwrap();
        comp.stage_handle(ticket.id, Arc::new("openai") as Arc<dyn Any + Send + Sync>)
            .unwrap();
        comp.mark_shadow_green(ticket.id).unwrap();
        comp.promote(ticket.id).unwrap();
        assert_eq!(comp.binding_id(&port), Some(AdapterId::new("openai-v1")));

        comp.rollback(ticket.id).expect("rolls back");
        assert_eq!(comp.binding_id(&port), Some(AdapterId::new("anthropic-v1")));
    }

    // ── Multi-port generalization tests (substrate ADR P10 foundation) ──
    // RuntimeComposition is port-agnostic by construction (operates on
    // PortId values, not on a hardcoded set of port types). These tests
    // prove that two ports can coexist, that swaps on one port don't
    // disturb another, and that concurrent shadows on different ports
    // are independent — the foundation a future ICoordinationPort
    // migration (or any second-port migration per substrate ADR P10)
    // builds on without changing this crate.

    #[test]
    fn two_ports_can_be_bound_concurrently() {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("inference-default"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        reg.bind(
            PortId::new("coordination"),
            AdapterId::new("coordination-default"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let comp = InMemoryComposition::new(reg);
        let snap = comp.snapshot();
        assert_eq!(snap.bindings.len(), 2);
        assert_eq!(
            snap.bindings.get(&PortId::new("inference")),
            Some(&AdapterId::new("inference-default"))
        );
        assert_eq!(
            snap.bindings.get(&PortId::new("coordination")),
            Some(&AdapterId::new("coordination-default"))
        );
    }

    #[test]
    fn swap_on_one_port_does_not_disturb_another() {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("inference-v1"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        reg.bind(
            PortId::new("coordination"),
            AdapterId::new("coordination-v1"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let comp = InMemoryComposition::new(reg);

        // Swap inference → swap_v2; coordination must stay v1 throughout.
        let ticket = comp
            .propose_swap(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("inference-v2"),
                manifest: AdapterManifest {
                    adapter_id: AdapterId::new("inference-v2"),
                    port: PortId::new("inference"),
                    version: "0.1.0".into(),
                    deps: vec![],
                },
            })
            .unwrap();
        comp.stage_handle(ticket.id, Arc::new(()) as Arc<dyn Any + Send + Sync>)
            .unwrap();
        comp.mark_shadow_green(ticket.id).unwrap();
        comp.promote(ticket.id).unwrap();

        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("inference-v2"))
        );
        assert_eq!(
            comp.binding_id(&PortId::new("coordination")),
            Some(AdapterId::new("coordination-v1")),
            "coordination port must be untouched by an inference-port swap"
        );
    }

    #[test]
    fn concurrent_swaps_on_different_ports_both_succeed() {
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("inf-v1"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        reg.bind(
            PortId::new("coordination"),
            AdapterId::new("coord-v1"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let comp = InMemoryComposition::new(reg);

        // Single-writer-per-port is a per-port constraint, NOT a global
        // constraint — proposing a swap on each of two ports must both
        // succeed, even simultaneously in flight.
        let t_inf = comp
            .propose_swap(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("inf-v2"),
                manifest: AdapterManifest {
                    adapter_id: AdapterId::new("inf-v2"),
                    port: PortId::new("inference"),
                    version: "0.1.0".into(),
                    deps: vec![],
                },
            })
            .expect("inference propose ok");
        let t_coord = comp
            .propose_swap(CompositionSwap {
                port: PortId::new("coordination"),
                new_adapter_id: AdapterId::new("coord-v2"),
                manifest: AdapterManifest {
                    adapter_id: AdapterId::new("coord-v2"),
                    port: PortId::new("coordination"),
                    version: "0.1.0".into(),
                    deps: vec![],
                },
            })
            .expect("coordination propose ok");
        assert_ne!(t_inf.id, t_coord.id);
        // Per-port single-writer enforced — second propose on inference
        // while t_inf is still open returns SwapInFlight.
        let dup = comp.propose_swap(CompositionSwap {
            port: PortId::new("inference"),
            new_adapter_id: AdapterId::new("inf-v3"),
            manifest: AdapterManifest {
                adapter_id: AdapterId::new("inf-v3"),
                port: PortId::new("inference"),
                version: "0.1.0".into(),
                deps: vec![],
            },
        });
        assert!(matches!(dup, Err(SwapError::SwapInFlight(_))));
    }

    #[test]
    fn rollback_on_one_port_isolates_from_another_ports_promotion() {
        // Promote on coordination, rollback on inference — the rollback
        // must restore inference's original binding without affecting
        // coordination's freshly-promoted binding.
        let mut reg = PortRegistry::new();
        reg.bind(
            PortId::new("inference"),
            AdapterId::new("inf-v1"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        reg.bind(
            PortId::new("coordination"),
            AdapterId::new("coord-v1"),
            Arc::new(()) as Arc<dyn Any + Send + Sync>,
        );
        let comp = InMemoryComposition::new(reg);

        // Promote both.
        for (port, new) in [
            ("inference", "inf-v2"),
            ("coordination", "coord-v2"),
        ] {
            let t = comp
                .propose_swap(CompositionSwap {
                    port: PortId::new(port),
                    new_adapter_id: AdapterId::new(new),
                    manifest: AdapterManifest {
                        adapter_id: AdapterId::new(new),
                        port: PortId::new(port),
                        version: "0.1.0".into(),
                        deps: vec![],
                    },
                })
                .unwrap();
            comp.stage_handle(t.id, Arc::new(()) as Arc<dyn Any + Send + Sync>)
                .unwrap();
            comp.mark_shadow_green(t.id).unwrap();
            comp.promote(t.id).unwrap();
        }

        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("inf-v2"))
        );
        assert_eq!(
            comp.binding_id(&PortId::new("coordination")),
            Some(AdapterId::new("coord-v2"))
        );

        // Roll back inference only — find its ticket id by proposing a
        // brand-new swap then rolling back the previous promoted one. We
        // captured ticket ids inline above; re-do for clarity:
        let inf_t = comp
            .propose_swap(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("inf-v3"),
                manifest: AdapterManifest {
                    adapter_id: AdapterId::new("inf-v3"),
                    port: PortId::new("inference"),
                    version: "0.1.0".into(),
                    deps: vec![],
                },
            })
            .unwrap();
        comp.stage_handle(inf_t.id, Arc::new(()) as Arc<dyn Any + Send + Sync>)
            .unwrap();
        comp.mark_shadow_green(inf_t.id).unwrap();
        comp.promote(inf_t.id).unwrap();
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("inf-v3"))
        );
        // Rollback the v3 promotion — inference returns to v2.
        comp.rollback(inf_t.id).unwrap();
        assert_eq!(
            comp.binding_id(&PortId::new("inference")),
            Some(AdapterId::new("inf-v2"))
        );
        // Coordination is unaffected.
        assert_eq!(
            comp.binding_id(&PortId::new("coordination")),
            Some(AdapterId::new("coord-v2"))
        );
    }

    #[test]
    fn snapshot_includes_all_bound_ports() {
        let mut reg = PortRegistry::new();
        for (port, adapter) in [
            ("inference", "inf"),
            ("coordination", "coord"),
            ("storage", "stg"),
            ("agent-runtime", "agent"),
        ] {
            reg.bind(
                PortId::new(port),
                AdapterId::new(adapter),
                Arc::new(()) as Arc<dyn Any + Send + Sync>,
            );
        }
        let comp = InMemoryComposition::new(reg);
        let snap = comp.snapshot();
        assert_eq!(snap.bindings.len(), 4);
        assert!(snap.bindings.contains_key(&PortId::new("storage")));
        assert!(snap.bindings.contains_key(&PortId::new("agent-runtime")));
    }

    #[test]
    fn rollback_rejects_non_promoted_ticket() {
        let comp = InMemoryComposition::new(PortRegistry::new());
        let ticket = comp
            .propose_swap(CompositionSwap {
                port: PortId::new("inference"),
                new_adapter_id: AdapterId::new("openai-v1"),
                manifest: manifest("inference", "openai-v1"),
            })
            .unwrap();
        let err = comp.rollback(ticket.id).expect_err("not promoted");
        assert!(matches!(err, SwapError::NotEligibleForPromotion(_)));
    }
}
