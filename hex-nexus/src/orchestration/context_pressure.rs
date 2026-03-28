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
