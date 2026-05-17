//! Substrate autopilot — STUB.
//!
//! The full implementation (ADR-2604261500 closing-the-loop) was referenced
//! from `sched_service.rs` but the source file was never committed, leaving
//! the workspace unbuildable. This stub restores `cargo build` parity so
//! unrelated work can land while a follow-up workplan implements the real
//! recommender. Every `tick()` here is a no-op that emits an Abstain so the
//! caller's match-arm logging makes the gap visible in operator logs.
//!
//! TODO(ADR-2604261500): replace with the real autopilot. Tracking workplan
//! should pull recommendations from substrate state via the swap-ticket port
//! and live `inference_port` — see `sched_service::spawn_substrate_autopilot`
//! for the expected call shape.
//!
//! This file intentionally has no logic that depends on substrate
//! invariants — it cannot recommend, propose, or abstain "for the right
//! reason". It exists to compile, nothing more.

use std::sync::Arc;

use hex_core::ports::inference::IInferencePort;

use crate::ports::state::ISwapTicketStatePort;

/// What the autopilot suggests after a tick.
#[derive(Debug, Clone)]
pub enum Recommendation {
    /// Substrate is healthy, no action needed.
    NoAction,
    /// Free-form recommendation text for the operator.
    Recommend { text: String },
    /// A concrete swap proposal serialized as JSON.
    ProposeSwap { json: String },
    /// Pilot declined to recommend; `reason` explains why.
    Abstain { reason: String },
}

/// Output of one autopilot tick.
#[derive(Debug, Clone)]
pub struct Report {
    pub recommendation: Option<Recommendation>,
}

/// Stub autopilot. Holds its dependencies but never reads them.
pub struct SubstrateAutopilot {
    _swap_port: Arc<dyn ISwapTicketStatePort>,
    _inference: Arc<dyn IInferencePort>,
    _model: String,
}

impl SubstrateAutopilot {
    pub fn new(
        swap_port: Arc<dyn ISwapTicketStatePort>,
        inference: Arc<dyn IInferencePort>,
        model: String,
    ) -> Self {
        Self {
            _swap_port: swap_port,
            _inference: inference,
            _model: model,
        }
    }

    pub async fn tick(&self) -> Report {
        Report {
            recommendation: Some(Recommendation::Abstain {
                reason: "substrate_autopilot is a stub — see ADR-2604261500 follow-up workplan"
                    .into(),
            }),
        }
    }
}
