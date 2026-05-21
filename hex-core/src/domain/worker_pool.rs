//! Worker-pool domain types — runtime consumer availability for the dispatcher.
//!
//! ADR-2026-05-19-0900 §1 (worker-pool invariant): the dispatcher publishes
//! work to a logical pool; the pool guarantees a live consumer or fails
//! fast. The 2026-05-19 postmortem captured the cost of NOT having this
//! invariant: 30+ re-enqueue cycles, no agent listening, no signal to
//! the operator that work was disappearing into a void.

use serde::{Deserialize, Serialize};

/// What the dispatcher learns when it asks "is there a consumer for
/// role X?". The decision is binary at the gate (Alive vs anything-else
/// blocks dispatch), but the third variant carries forensics for the
/// inbox notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsumerStatus {
    /// At least one worker_process row for `role` has a `last_heartbeat`
    /// within the configured TTL and status != "stopping". Safe to
    /// dispatch; the gate opens.
    Alive { worker_count: u32 },
    /// One or more worker_process rows exist but their last_heartbeat
    /// is older than the TTL. Carries the staleness age so the inbox
    /// notification can say "youngest worker is N seconds old, expected
    /// ≤ M" — actionable for the operator deciding whether to restart
    /// a daemon or wait.
    Degraded {
        worker_count: u32,
        oldest_heartbeat_age_secs: u64,
    },
    /// No worker_process rows for `role`. The dispatcher must refuse to
    /// publish; otherwise we recreate the 2026-05-19 invisible churn.
    None,
}

impl ConsumerStatus {
    /// True when the gate opens — the dispatcher can claim work for
    /// this role. Centralized so call sites can't drift apart.
    pub fn dispatch_allowed(&self) -> bool {
        matches!(self, ConsumerStatus::Alive { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_allowed_only_on_alive() {
        assert!(ConsumerStatus::Alive { worker_count: 1 }.dispatch_allowed());
        assert!(!ConsumerStatus::Degraded { worker_count: 1, oldest_heartbeat_age_secs: 120 }.dispatch_allowed());
        assert!(!ConsumerStatus::None.dispatch_allowed());
    }
}
