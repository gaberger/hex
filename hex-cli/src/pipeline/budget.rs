//! Budget tracking for `hex dev` pipeline (ADR-2603232005).
//!
//! Tracks cumulative cost, tokens, and request counts across inference calls.
//! Provides budget enforcement with soft warnings (80% threshold) and a gate
//! dialog when the budget is exceeded — the user can always override.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::session::DevSession;

// ---------------------------------------------------------------------------
// BudgetStatus
// ---------------------------------------------------------------------------

/// Result of checking the current spend against the configured budget cap.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetStatus {
    /// Spend is within budget (or no budget set).
    Ok,
    /// Spend has crossed the warning threshold. Contains percentage used (0.0–1.0).
    Warning(f64),
    /// Spend has met or exceeded the budget cap.
    Exceeded,
}

// ---------------------------------------------------------------------------
// BudgetSummary
// ---------------------------------------------------------------------------

/// Snapshot of budget state, suitable for display or serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSummary {
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub total_requests: u32,
    pub budget_limit: Option<f64>,
    pub remaining: Option<f64>,
    pub per_phase_cost: HashMap<String, f64>,
    pub per_model_cost: HashMap<String, f64>,
}

// ---------------------------------------------------------------------------
// BudgetTracker
// ---------------------------------------------------------------------------

/// Accumulates cost and token usage across inference calls, with optional
/// budget cap enforcement.
#[derive(Debug, Clone)]
pub struct BudgetTracker {
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub total_requests: u32,
    pub budget_limit: Option<f64>,
    pub per_phase_cost: HashMap<String, f64>,
    pub per_model_cost: HashMap<String, f64>,
}

/// Threshold (fraction of budget) at which we start showing warnings.
const WARNING_THRESHOLD: f64 = 0.80;

impl BudgetTracker {
    /// Create a new tracker with an optional budget ceiling.
    ///
    /// Pass `None` or `Some(0.0)` for unlimited spend.
    pub fn new(budget_limit: Option<f64>) -> Self {
        let limit = budget_limit.filter(|&b| b > 0.0);
        Self {
            total_cost_usd: 0.0,
            total_tokens: 0,
            total_requests: 0,
            budget_limit: limit,
            per_phase_cost: HashMap::new(),
            per_model_cost: HashMap::new(),
        }
    }

    /// Record a single inference call's cost and token usage.
    pub fn record(&mut self, model: &str, phase: &str, cost: f64, tokens: u64) {
        self.total_cost_usd += cost;
        self.total_tokens += tokens;
        self.total_requests += 1;

        *self.per_phase_cost.entry(phase.to_string()).or_insert(0.0) += cost;
        *self.per_model_cost.entry(model.to_string()).or_insert(0.0) += cost;
    }

    /// Check current spend against the budget cap.
    pub fn check_budget(&self) -> BudgetStatus {
        let limit = match self.budget_limit {
            Some(l) if l > 0.0 => l,
            _ => return BudgetStatus::Ok,
        };

        let fraction = self.total_cost_usd / limit;
        if fraction >= 1.0 {
            BudgetStatus::Exceeded
        } else if fraction >= WARNING_THRESHOLD {
            BudgetStatus::Warning(fraction)
        } else {
            BudgetStatus::Ok
        }
    }

    /// How much budget remains, if a limit is set.
    pub fn remaining(&self) -> Option<f64> {
        self.budget_limit.map(|l| (l - self.total_cost_usd).max(0.0))
    }

    /// Produce a serializable summary for display or session persistence.
    pub fn summary(&self) -> BudgetSummary {
        BudgetSummary {
            total_cost_usd: self.total_cost_usd,
            total_tokens: self.total_tokens,
            total_requests: self.total_requests,
            budget_limit: self.budget_limit,
            remaining: self.remaining(),
            per_phase_cost: self.per_phase_cost.clone(),
            per_model_cost: self.per_model_cost.clone(),
        }
    }

    /// Restore a tracker from a resumed session's accumulated cost/tokens.
    pub fn from_session(session: &DevSession, budget_limit: Option<f64>) -> Self {
        let limit = budget_limit.filter(|&b| b > 0.0);
        Self {
            total_cost_usd: session.total_cost_usd,
            total_tokens: session.total_tokens,
            total_requests: 0, // request count is not persisted in session
            budget_limit: limit,
            per_phase_cost: HashMap::new(),
            per_model_cost: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_starts_at_zero() {
        let t = BudgetTracker::new(Some(1.0));
        assert_eq!(t.total_cost_usd, 0.0);
        assert_eq!(t.total_tokens, 0);
        assert_eq!(t.total_requests, 0);
        assert_eq!(t.budget_limit, Some(1.0));
    }

    #[test]
    fn zero_budget_treated_as_unlimited() {
        let t = BudgetTracker::new(Some(0.0));
        assert_eq!(t.budget_limit, None);
        assert_eq!(t.check_budget(), BudgetStatus::Ok);
    }

    #[test]
    fn none_budget_is_unlimited() {
        let t = BudgetTracker::new(None);
        assert_eq!(t.budget_limit, None);
        assert_eq!(t.remaining(), None);
        assert_eq!(t.check_budget(), BudgetStatus::Ok);
    }

    #[test]
    fn record_accumulates() {
        let mut t = BudgetTracker::new(Some(1.0));
        t.record("deepseek-r1", "adr", 0.05, 1000);
        t.record("deepseek-r1", "adr", 0.03, 500);
        t.record("claude-sonnet", "code", 0.10, 2000);

        assert_eq!(t.total_requests, 3);
        assert!((t.total_cost_usd - 0.18).abs() < 1e-10);
        assert_eq!(t.total_tokens, 3500);
        assert!((t.per_phase_cost["adr"] - 0.08).abs() < 1e-10);
        assert!((t.per_phase_cost["code"] - 0.10).abs() < 1e-10);
        assert!((t.per_model_cost["deepseek-r1"] - 0.08).abs() < 1e-10);
        assert!((t.per_model_cost["claude-sonnet"] - 0.10).abs() < 1e-10);
    }

    #[test]
    fn budget_status_ok_when_under_threshold() {
        let mut t = BudgetTracker::new(Some(1.0));
        t.record("m", "p", 0.50, 100);
        assert_eq!(t.check_budget(), BudgetStatus::Ok);
    }

    #[test]
    fn budget_status_warning_at_threshold() {
        let mut t = BudgetTracker::new(Some(1.0));
        t.record("m", "p", 0.85, 100);
        match t.check_budget() {
            BudgetStatus::Warning(frac) => assert!((frac - 0.85).abs() < 1e-10),
            other => panic!("expected Warning, got {:?}", other),
        }
    }

    #[test]
    fn budget_status_exceeded() {
        let mut t = BudgetTracker::new(Some(1.0));
        t.record("m", "p", 1.05, 100);
        assert_eq!(t.check_budget(), BudgetStatus::Exceeded);
    }

    #[test]
    fn remaining_tracks_correctly() {
        let mut t = BudgetTracker::new(Some(2.0));
        t.record("m", "p", 0.75, 100);
        assert!((t.remaining().unwrap() - 1.25).abs() < 1e-10);
    }

    #[test]
    fn remaining_floors_at_zero() {
        let mut t = BudgetTracker::new(Some(1.0));
        t.record("m", "p", 1.50, 100);
        assert!((t.remaining().unwrap()).abs() < 1e-10);
    }

    #[test]
    fn summary_captures_state() {
        let mut t = BudgetTracker::new(Some(5.0));
        t.record("m1", "adr", 0.10, 500);
        let s = t.summary();
        assert_eq!(s.total_requests, 1);
        assert_eq!(s.budget_limit, Some(5.0));
        assert!((s.remaining.unwrap() - 4.90).abs() < 1e-10);
    }

    #[test]
    fn from_session_restores_cost() {
        let mut session = DevSession::new("test");
        let _ = session.add_cost(0.50, 2000);
        let t = BudgetTracker::from_session(&session, Some(1.0));
        assert!((t.total_cost_usd - 0.50).abs() < 1e-10);
        assert_eq!(t.total_tokens, 2000);
        assert_eq!(t.budget_limit, Some(1.0));
    }
}
