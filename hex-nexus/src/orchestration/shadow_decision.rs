//! Port-agnostic shadow-routing decision (substrate ADR P10 foundation).
//!
//! `ShadowRouter` is `IInferencePort`-specific because it dispatches
//! `complete()`. The *decision* of whether to mirror — and the
//! *bookkeeping* of per-adapter last-routed timestamps — is the same
//! across ports. Pulling the shared pieces here lets a future
//! `SecretShadowRouter` (or any other per-port router) reuse them
//! without copy-paste.
//!
//! Today this module contains:
//! - [`ShadowDecision`] — the typed verdict the per-port router acts on.
//! - [`shadow_decision`] — the per-call logic: given an optional active
//!   ticket and an RNG draw, return whether to mirror and to which
//!   candidate id.
//! - [`AdapterRoutingTracker`] — per-adapter last-routed-at map shared
//!   by per-port routers and the L4 shrinkage daemon.
//!
//! `ShadowRouter` (IInferencePort) keeps its own copies for now to avoid
//! a refactor risk; new port migrations adopt this helper directly. A
//! later cleanup can have ShadowRouter consume `AdapterRoutingTracker`
//! too.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use hex_core::composition::AdapterId;
use tokio::sync::RwLock;

/// Per-call mirroring verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadowDecision {
    /// Live only — no active ticket, or RNG missed the fraction.
    LiveOnly,
    /// Mirror to this candidate adapter for ticket id.
    Mirror {
        ticket_id: String,
        candidate_adapter_id: AdapterId,
    },
}

/// In-memory active-shadow record (port-agnostic shape; per-port routers
/// hold a `BTreeMap<PortId, ActiveTicket>`).
#[derive(Debug, Clone)]
pub struct ActiveTicket {
    pub ticket_id: String,
    pub candidate_adapter_id: AdapterId,
    pub traffic_fraction: f32,
}

/// Compute whether a single call should mirror to a candidate. RNG draw
/// passed in so callers can inject deterministic values in tests.
pub fn shadow_decision(active: Option<&ActiveTicket>, rng_draw: f32) -> ShadowDecision {
    match active {
        None => ShadowDecision::LiveOnly,
        Some(t) if rng_draw >= t.traffic_fraction => ShadowDecision::LiveOnly,
        Some(t) => ShadowDecision::Mirror {
            ticket_id: t.ticket_id.clone(),
            candidate_adapter_id: t.candidate_adapter_id.clone(),
        },
    }
}

/// Per-adapter last-routed-at tracker. Future per-port routers store one
/// of these and call `mark_routed` after every successful dispatch.
pub struct AdapterRoutingTracker {
    routed_at: RwLock<BTreeMap<AdapterId, Instant>>,
}

impl AdapterRoutingTracker {
    pub fn new() -> Self {
        Self {
            routed_at: RwLock::new(BTreeMap::new()),
        }
    }

    pub async fn mark_routed(&self, adapter_id: &AdapterId) {
        self.routed_at
            .write()
            .await
            .insert(adapter_id.clone(), Instant::now());
    }

    pub async fn last_routed_at(&self, adapter_id: &AdapterId) -> Option<Instant> {
        self.routed_at.read().await.get(adapter_id).copied()
    }

    pub async fn forget(&self, adapter_id: &AdapterId) {
        self.routed_at.write().await.remove(adapter_id);
    }

    /// Compute shrinkage candidates: registered adapter ids that are not
    /// in `bound`, not in `active`, and either never-routed or last
    /// routed before `now - idle_window`. The caller supplies the three
    /// sets so this function stays storage-agnostic.
    pub async fn compute_shrinkable(
        &self,
        registered: &[AdapterId],
        bound: &std::collections::BTreeSet<AdapterId>,
        active: &std::collections::BTreeSet<AdapterId>,
        idle_window: std::time::Duration,
    ) -> Vec<AdapterId> {
        let now = Instant::now();
        let routed_at = self.routed_at.read().await;
        registered
            .iter()
            .filter(|id| !bound.contains(*id) && !active.contains(*id))
            .filter(|id| match routed_at.get(*id) {
                Some(at) => now.duration_since(*at) >= idle_window,
                None => true,
            })
            .cloned()
            .collect()
    }
}

impl Default for AdapterRoutingTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait shape future per-port shadow routers implement. Documented as a
/// pattern; not enforced via type system because async traits over generic
/// per-port call shapes don't compose cleanly without dynamic dispatch.
/// The substrate's `ShadowRouter` for `IInferencePort` is the canonical
/// instance — new ports follow the same shape. See ADR-2604261800 for
/// the strategy-level integration model.
pub trait PortShadowRouterShape {
    // Marker only — see module docs.
}

// ── Wiring helpers used by per-port routers ─────────────────────────────

pub type SampleRng = Arc<dyn Fn() -> f32 + Send + Sync>;

/// Default RNG factory — `rand::random::<f32>()`. Production routers use
/// this; tests inject deterministic closures.
pub fn default_rng() -> SampleRng {
    Arc::new(|| rand::random::<f32>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: &str, candidate: &str, fraction: f32) -> ActiveTicket {
        ActiveTicket {
            ticket_id: id.into(),
            candidate_adapter_id: AdapterId::new(candidate),
            traffic_fraction: fraction,
        }
    }

    // ── shadow_decision ────────────────────────────────────

    #[test]
    fn no_active_ticket_returns_live_only() {
        assert_eq!(shadow_decision(None, 0.0), ShadowDecision::LiveOnly);
        assert_eq!(shadow_decision(None, 0.999), ShadowDecision::LiveOnly);
    }

    #[test]
    fn rng_above_fraction_returns_live_only() {
        let t = ticket("t1", "candidate-x", 0.5);
        assert_eq!(shadow_decision(Some(&t), 0.5), ShadowDecision::LiveOnly);
        assert_eq!(shadow_decision(Some(&t), 0.99), ShadowDecision::LiveOnly);
    }

    #[test]
    fn rng_below_fraction_returns_mirror() {
        let t = ticket("t1", "candidate-x", 0.5);
        match shadow_decision(Some(&t), 0.0) {
            ShadowDecision::Mirror { ticket_id, candidate_adapter_id } => {
                assert_eq!(ticket_id, "t1");
                assert_eq!(candidate_adapter_id, AdapterId::new("candidate-x"));
            }
            other => panic!("expected Mirror, got {:?}", other),
        }
    }

    #[test]
    fn fraction_one_always_mirrors() {
        let t = ticket("t1", "candidate-x", 1.0);
        // Any rng < 1.0 mirrors. rng == 1.0 wouldn't be returned by
        // rand::random::<f32>() in practice (range is [0, 1)) but check
        // the boundary.
        assert!(matches!(shadow_decision(Some(&t), 0.0), ShadowDecision::Mirror { .. }));
        assert!(matches!(shadow_decision(Some(&t), 0.999), ShadowDecision::Mirror { .. }));
        assert_eq!(shadow_decision(Some(&t), 1.0), ShadowDecision::LiveOnly);
    }

    #[test]
    fn fraction_zero_never_mirrors() {
        let t = ticket("t1", "candidate-x", 0.0);
        assert_eq!(shadow_decision(Some(&t), 0.0), ShadowDecision::LiveOnly);
    }

    // ── AdapterRoutingTracker ──────────────────────────────

    #[tokio::test]
    async fn tracker_records_and_returns_routed_at() {
        let t = AdapterRoutingTracker::new();
        let id = AdapterId::new("a");
        assert!(t.last_routed_at(&id).await.is_none());
        t.mark_routed(&id).await;
        assert!(t.last_routed_at(&id).await.is_some());
    }

    #[tokio::test]
    async fn tracker_forget_removes_entry() {
        let t = AdapterRoutingTracker::new();
        let id = AdapterId::new("a");
        t.mark_routed(&id).await;
        t.forget(&id).await;
        assert!(t.last_routed_at(&id).await.is_none());
    }

    #[tokio::test]
    async fn shrinkable_excludes_bound_and_active() {
        let t = AdapterRoutingTracker::new();
        let registered = vec![
            AdapterId::new("a"),
            AdapterId::new("b"),
            AdapterId::new("c"),
            AdapterId::new("d"),
        ];
        let mut bound = std::collections::BTreeSet::new();
        bound.insert(AdapterId::new("a"));
        let mut active = std::collections::BTreeSet::new();
        active.insert(AdapterId::new("b"));
        // c + d are eligible (not bound, not active, never routed).
        let result = t
            .compute_shrinkable(&registered, &bound, &active, std::time::Duration::from_secs(0))
            .await;
        assert!(result.contains(&AdapterId::new("c")));
        assert!(result.contains(&AdapterId::new("d")));
        assert!(!result.contains(&AdapterId::new("a")));
        assert!(!result.contains(&AdapterId::new("b")));
    }

    #[tokio::test]
    async fn shrinkable_respects_idle_window_for_recently_routed() {
        let t = AdapterRoutingTracker::new();
        let id = AdapterId::new("a");
        t.mark_routed(&id).await;
        let registered = vec![id.clone()];
        let bound = std::collections::BTreeSet::new();
        let active = std::collections::BTreeSet::new();

        // Long window: just-routed adapter is NOT shrinkable.
        let r = t
            .compute_shrinkable(
                &registered,
                &bound,
                &active,
                std::time::Duration::from_secs(86400),
            )
            .await;
        assert!(r.is_empty());

        // Zero window: anything elapsed is eligible (the just-routed
        // adapter has elapsed >= 0).
        let r = t
            .compute_shrinkable(&registered, &bound, &active, std::time::Duration::from_secs(0))
            .await;
        assert_eq!(r, vec![id]);
    }
}
