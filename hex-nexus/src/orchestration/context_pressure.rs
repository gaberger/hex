//! Context window pressure tracking (ADR-2603281000 P1).
//!
//! Tracks token consumption across agent sessions and signals when the context
//! window is approaching capacity so the supervisor can trigger summarisation
//! or hand-off to a fresh agent.

/// Tracks context pressure for an active agent session.
#[derive(Debug, Default)]
pub struct ContextPressureTracker {
    /// Total tokens consumed in the current session.
    pub tokens_used: u64,
    /// Hard ceiling for the active model (default: 200_000).
    pub tokens_limit: u64,
}

impl ContextPressureTracker {
    /// Create a tracker with the default 200k token ceiling.
    pub fn new() -> Self {
        Self {
            tokens_used: 0,
            tokens_limit: 200_000,
        }
    }

    /// Record additional token consumption.
    pub fn record(&mut self, tokens: u64) {
        self.tokens_used = self.tokens_used.saturating_add(tokens);
    }

    /// Fraction of context used (0.0–1.0).
    pub fn pressure(&self) -> f64 {
        if self.tokens_limit == 0 {
            return 0.0;
        }
        self.tokens_used as f64 / self.tokens_limit as f64
    }

    /// Returns `true` when context usage exceeds 80%.
    pub fn is_high(&self) -> bool {
        self.pressure() >= 0.8
    }

    /// Returns `true` when context usage exceeds 95%.
    pub fn is_critical(&self) -> bool {
        self.pressure() >= 0.95
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_at_zero() {
        let t = ContextPressureTracker::new();
        assert_eq!(t.tokens_used, 0);
        assert_eq!(t.pressure(), 0.0);
    }

    #[test]
    fn record_accumulates_tokens() {
        let mut t = ContextPressureTracker::new();
        t.record(50_000);
        t.record(50_000);
        assert_eq!(t.tokens_used, 100_000);
    }

    #[test]
    fn pressure_fraction_correct() {
        let mut t = ContextPressureTracker::new();
        t.record(100_000); // 50% of 200k
        assert!((t.pressure() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn is_high_triggers_at_80_pct() {
        let mut t = ContextPressureTracker::new();
        t.record(159_999); // just under 80%
        assert!(!t.is_high());
        t.record(1); // 160_000 / 200_000 = 80%
        assert!(t.is_high());
    }

    #[test]
    fn is_critical_triggers_at_95_pct() {
        let mut t = ContextPressureTracker::new();
        t.record(189_999); // just under 95%
        assert!(!t.is_critical());
        t.record(1); // 190_000 / 200_000 = 95%
        assert!(t.is_critical());
    }

    #[test]
    fn record_saturates_at_u64_max() {
        let mut t = ContextPressureTracker::new();
        t.record(u64::MAX);
        t.record(1); // should not overflow
        assert_eq!(t.tokens_used, u64::MAX);
    }

    #[test]
    fn zero_limit_returns_zero_pressure() {
        let t = ContextPressureTracker {
            tokens_used: 1000,
            tokens_limit: 0,
        };
        assert_eq!(t.pressure(), 0.0);
    }
}
