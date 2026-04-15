//! Q-value report types for inference tier routing.
//!
//! `QReportEntry` captures a single state→model Q-value observation,
//! used by `hex inference q-report` to surface tier routing decisions.

use serde::{Deserialize, Serialize};

/// A single row in the inference Q-value report.
///
/// Each entry represents the learned value of choosing `model` when the
/// system is in `state` (e.g. a task-type or tier label). `trend_7d`
/// gives directional momentum so operators can spot regressions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QReportEntry {
    /// State identifier (e.g. "t1_scaffold", "t2_codegen").
    pub state: String,
    /// Model identifier (e.g. "qwen3:4b", "devstral-small-2:24b").
    pub model: String,
    /// Learned Q-value for this (state, model) pair.
    pub q_value: f64,
    /// Number of times this pair has been visited.
    pub visits: u64,
    /// ISO-8601 timestamp of the most recent observation.
    pub last_seen: String,
    /// 7-day trend: positive = improving, negative = degrading, None = insufficient data.
    pub trend_7d: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        let entry = QReportEntry {
            state: "t1_scaffold".into(),
            model: "qwen3:4b".into(),
            q_value: 0.87,
            visits: 42,
            last_seen: "2026-04-15T12:00:00Z".into(),
            trend_7d: Some(0.03),
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let back: QReportEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry, back);
    }

    #[test]
    fn serde_round_trip_none_trend() {
        let entry = QReportEntry {
            state: "t2_codegen".into(),
            model: "qwen2.5-coder:32b".into(),
            q_value: 0.65,
            visits: 3,
            last_seen: "2026-04-15T08:30:00Z".into(),
            trend_7d: None,
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let back: QReportEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry, back);
        assert!(json.contains("\"trend_7d\":null"));
    }
}
