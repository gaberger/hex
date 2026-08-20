//! C3 — First-class per-port telemetry (per ADR-2026-04-26-1500).
//!
//! Every port trait declares its metrics shape via `PortTelemetry`. Adapter
//! authors use `PortTelemetry::emit` instead of free-form logging; the
//! substrate routes samples to whatever sink the composition root registers
//! at boot. The default sink is a no-op so unconfigured tests and embedded
//! consumers do not pay for telemetry that nobody is reading.
//!
//! The proc-macro derive (workplan P2.2) is deferred — adapter authors hand-
//! impl the trait until a consumer surfaces enough adapters for the derive
//! to pay for itself.

use crate::composition::AdapterId;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::sync::OnceLock;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Counter {
    pub value: u64,
}

impl Counter {
    pub fn incr(&mut self, by: u64) {
        self.value = self.value.saturating_add(by);
    }
}

/// Lightweight in-crate histogram — we keep raw samples and let the sink
/// decide bucketing. The substrate-level metrics adapter (out of scope here)
/// is what folds samples into Prometheus / OTel / STDB tables.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Histogram {
    pub samples: Vec<f64>,
}

impl Histogram {
    pub fn observe(&mut self, value: f64) {
        self.samples.push(value);
    }
}

/// Port-side declaration of what telemetry an adapter implementing this port
/// must emit. The associated `Metrics` type is the per-call sample shape.
pub trait PortTelemetry {
    type Metrics: Send + Sync + 'static;

    fn emit(adapter_id: AdapterId, sample: Self::Metrics) {
        if let Some(sink) = TELEMETRY_SINK.get() {
            sink(adapter_id, &sample);
        }
    }
}

/// Type-erased telemetry sink. The composition root calls `register_sink`
/// once at boot. Subsequent `register_sink` calls are rejected (returns
/// `false`) — the sink is intentionally append-once to keep the substrate's
/// observation surface stable for the life of the process.
type SinkFn = Box<dyn Fn(AdapterId, &dyn Any) + Send + Sync>;

static TELEMETRY_SINK: OnceLock<SinkFn> = OnceLock::new();

pub fn register_sink<F>(sink: F) -> bool
where
    F: Fn(AdapterId, &dyn Any) + Send + Sync + 'static,
{
    TELEMETRY_SINK.set(Box::new(sink)).is_ok()
}

#[cfg(test)]
pub(crate) fn _is_sink_registered() -> bool {
    TELEMETRY_SINK.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyPort;

    #[derive(Debug, Clone)]
    #[allow(dead_code)]
    struct DummyMetrics {
        latency_ms: u64,
    }

    impl PortTelemetry for DummyPort {
        type Metrics = DummyMetrics;
    }

    /// emit() with no registered sink must be a no-op (no panic, no side
    /// effects). Tests can exercise this safely because the OnceLock is per-
    /// process and we don't attempt to register here.
    #[test]
    fn emit_is_noop_without_registered_sink() {
        // Cannot assert "no-op" by observation in isolation, but we can
        // confirm no panic and no side effect.
        DummyPort::emit(AdapterId::new("dummy"), DummyMetrics { latency_ms: 7 });
    }

    /// Counter / Histogram primitives behave as expected. These are the
    /// types port authors compose into their `Metrics` structs.
    #[test]
    fn counter_increments_saturating() {
        let mut c = Counter::default();
        c.incr(3);
        c.incr(u64::MAX);
        assert_eq!(c.value, u64::MAX);
    }

    #[test]
    fn histogram_records_samples_in_order() {
        let mut h = Histogram::default();
        h.observe(1.0);
        h.observe(2.5);
        assert_eq!(h.samples, vec![1.0, 2.5]);
    }
}
