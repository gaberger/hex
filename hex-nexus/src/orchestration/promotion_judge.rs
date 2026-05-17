//! PromotionJudge — the Layer-3 judge from ADR-2604261311 re-bound to
//! swap-ticket transitions per substrate ADR-2604261500 C6
//! (wp-substrate-shadow-promotion P4.1).
//!
//! On each `tick()` the judge:
//! 1. Reads all swap_tickets currently in `shadow` state.
//! 2. Filters to those whose shadow window has elapsed
//!    (`shadow_started_at + shadow_window_seconds <= now`).
//! 3. For each due ticket, reads the recorded `shadow_sample` rows.
//! 4. Deserializes the ticket's `success_criteria_json` into a
//!    `Vec<SuccessCriterion>` (the canonical type lives in
//!    `hex_core::ports::adapter_generator`) and evaluates each criterion
//!    against the candidate samples.
//! 5. If every criterion passes, transitions the ticket to `shadow_green`
//!    via `swap_ticket_transition`. Otherwise transitions to `shadow_red`
//!    and records a sentinel `shadow_sample` capturing the rejection reason
//!    so the dashboard (P5) can surface why.
//!
//! Idempotent — re-running on a non-Shadow ticket is a no-op (the STDB
//! state machine rejects any subsequent transition out of a terminal
//! state, so this code does not need to re-check explicitly).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use hex_core::ports::adapter_generator::SuccessCriterion;
use serde::Deserialize;

use crate::ports::state::{ISwapTicketStatePort, ShadowSampleRecord, SwapTicketRecord};

pub struct PromotionJudge {
    state: Arc<dyn ISwapTicketStatePort>,
}

#[derive(Debug, Clone, Default)]
pub struct JudgeTickReport {
    pub considered: usize,
    pub due: usize,
    pub promoted_to_green: Vec<String>,
    pub marked_red: Vec<(String, String)>, // (ticket_id, reason)
    pub errors: Vec<(String, String)>,
}

#[derive(Deserialize, Default)]
struct CandidateSampleMetrics {
    #[serde(default)]
    latency_ms: u64,
    #[serde(default)]
    error: bool,
}

impl PromotionJudge {
    pub fn new(state: Arc<dyn ISwapTicketStatePort>) -> Self {
        Self { state }
    }

    pub async fn tick(&self) -> JudgeTickReport {
        self.tick_at(Utc::now()).await
    }

    /// Test seam: deterministic clock.
    pub async fn tick_at(&self, now: DateTime<Utc>) -> JudgeTickReport {
        let mut report = JudgeTickReport::default();

        let tickets = match self.state.shadow_tickets_due(&now.to_rfc3339()).await {
            Ok(t) => t,
            Err(e) => {
                report.errors.push(("<list>".into(), e.to_string()));
                return report;
            }
        };
        report.considered = tickets.len();

        for ticket in tickets {
            if !window_elapsed(&ticket, now) {
                continue;
            }
            report.due += 1;
            self.judge_ticket(&ticket, now, &mut report).await;
        }

        report
    }

    async fn judge_ticket(
        &self,
        ticket: &SwapTicketRecord,
        now: DateTime<Utc>,
        report: &mut JudgeTickReport,
    ) {
        let samples = match self.state.shadow_samples_for(&ticket.id).await {
            Ok(s) => s,
            Err(e) => {
                report.errors.push((ticket.id.clone(), e.to_string()));
                return;
            }
        };

        let criteria: Vec<SuccessCriterion> =
            serde_json::from_str(&ticket.success_criteria_json).unwrap_or_default();

        let verdict = evaluate(&samples, &criteria);
        let now_str = now.to_rfc3339();

        match verdict {
            Verdict::Pass => {
                if let Err(e) = self
                    .state
                    .swap_ticket_transition(&ticket.id, "shadow_green", &now_str)
                    .await
                {
                    report.errors.push((ticket.id.clone(), e.to_string()));
                } else {
                    report.promoted_to_green.push(ticket.id.clone());
                }
            }
            Verdict::Fail(reason) => {
                if let Err(e) = self
                    .state
                    .swap_ticket_transition(&ticket.id, "shadow_red", &now_str)
                    .await
                {
                    report.errors.push((ticket.id.clone(), e.to_string()));
                    return;
                }
                // Sentinel sample so the dashboard can surface the reason
                // without forcing dashboard code to interpret transitions.
                let _ = self
                    .state
                    .shadow_sample_record(
                        &ticket.id,
                        sentinel_seq(&samples),
                        &ticket.incumbent_adapter_id,
                        &ticket.candidate_adapter_id,
                        "{}",
                        "{}",
                        false,
                        &reason,
                        &now_str,
                    )
                    .await;
                report.marked_red.push((ticket.id.clone(), reason));
            }
        }
    }
}

#[derive(Debug)]
enum Verdict {
    Pass,
    Fail(String),
}

fn window_elapsed(ticket: &SwapTicketRecord, now: DateTime<Utc>) -> bool {
    if ticket.shadow_started_at.is_empty() {
        return false;
    }
    let started = match DateTime::parse_from_rfc3339(&ticket.shadow_started_at) {
        Ok(t) => t.with_timezone(&Utc),
        Err(_) => return false,
    };
    let elapsed = now.signed_duration_since(started).num_seconds();
    elapsed >= ticket.shadow_window_seconds as i64
}

fn sentinel_seq(samples: &[ShadowSampleRecord]) -> u64 {
    samples.iter().map(|s| s.call_seq).max().unwrap_or(0) + 1
}

fn evaluate(samples: &[ShadowSampleRecord], criteria: &[SuccessCriterion]) -> Verdict {
    if criteria.is_empty() {
        // No criteria specified → default-pass. Caller is responsible for
        // attaching meaningful criteria when proposing a swap; the judge
        // does not invent gates.
        return Verdict::Pass;
    }
    if samples.is_empty() {
        return Verdict::Fail("no shadow samples recorded".into());
    }

    for criterion in criteria {
        if let Err(reason) = check_criterion(criterion, samples) {
            return Verdict::Fail(reason);
        }
    }
    Verdict::Pass
}

fn check_criterion(criterion: &SuccessCriterion, samples: &[ShadowSampleRecord]) -> Result<(), String> {
    match criterion {
        SuccessCriterion::ResponseEquivalence { tolerance } => {
            let agreed = samples.iter().filter(|s| s.agreed).count();
            let ratio = agreed as f64 / samples.len() as f64;
            // Day-one interpretation: `tolerance` is the maximum acceptable
            // disagreement rate (0.0 = perfect agreement required, 1.0 =
            // any agreement passes). Equivalent to: `1.0 - ratio <= tolerance`.
            if (1.0 - ratio) > *tolerance {
                return Err(format!(
                    "ResponseEquivalence: {:.2}% disagreement exceeds tolerance {:.2}%",
                    (1.0 - ratio) * 100.0,
                    tolerance * 100.0
                ));
            }
            Ok(())
        }
        SuccessCriterion::LatencyP99BelowMs(ceiling) => {
            let mut candidate_latencies: Vec<u64> = samples
                .iter()
                .filter_map(|s| {
                    serde_json::from_str::<CandidateSampleMetrics>(&s.candidate_metrics_json)
                        .ok()
                        .map(|m| m.latency_ms)
                })
                .collect();
            if candidate_latencies.is_empty() {
                return Err("LatencyP99BelowMs: no candidate metrics parsed".into());
            }
            candidate_latencies.sort_unstable();
            let p99_idx = ((candidate_latencies.len() as f64) * 0.99).ceil() as usize;
            let p99_idx = p99_idx.saturating_sub(1).min(candidate_latencies.len() - 1);
            let p99 = candidate_latencies[p99_idx];
            if p99 > *ceiling {
                return Err(format!(
                    "LatencyP99BelowMs: candidate p99 {}ms exceeds ceiling {}ms",
                    p99, ceiling
                ));
            }
            Ok(())
        }
        SuccessCriterion::ErrorRateBelow(ceiling) => {
            let total = samples.len();
            let candidate_errors = samples
                .iter()
                .filter_map(|s| {
                    serde_json::from_str::<CandidateSampleMetrics>(&s.candidate_metrics_json)
                        .ok()
                        .map(|m| m.error)
                })
                .filter(|err| *err)
                .count();
            let rate = candidate_errors as f32 / total as f32;
            if rate > *ceiling {
                return Err(format!(
                    "ErrorRateBelow: candidate error rate {:.2}% exceeds ceiling {:.2}%",
                    rate * 100.0,
                    ceiling * 100.0
                ));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use crate::ports::state::StateError;

    /// Stub state port that returns canned tickets / samples and records
    /// every transition + sentinel sample insert.
    #[derive(Default)]
    struct StubState {
        tickets: Mutex<Vec<SwapTicketRecord>>,
        samples_by_ticket: Mutex<std::collections::BTreeMap<String, Vec<ShadowSampleRecord>>>,
        transitions: Mutex<Vec<(String, String)>>,
        sentinel_samples: Mutex<Vec<(String, String)>>, // (ticket_id, reason)
    }

    impl StubState {
        fn add_ticket(&self, t: SwapTicketRecord) {
            self.tickets.lock().unwrap().push(t);
        }
        fn add_samples(&self, ticket_id: &str, samples: Vec<ShadowSampleRecord>) {
            self.samples_by_ticket
                .lock()
                .unwrap()
                .insert(ticket_id.to_string(), samples);
        }
    }

    #[async_trait]
    impl ISwapTicketStatePort for StubState {
        async fn swap_ticket_create(&self, _: &str, _: &str, _: &str, _: &str, _: &str, _: &str, _: f32, _: u64, _: &str, _: &str) -> Result<(), StateError> { Ok(()) }

        async fn swap_ticket_transition(&self, id: &str, new_state: &str, _: &str) -> Result<(), StateError> {
            self.transitions.lock().unwrap().push((id.to_string(), new_state.to_string()));
            Ok(())
        }

        async fn swap_ticket_set_shadow_started(&self, _: &str, _: &str) -> Result<(), StateError> { Ok(()) }
        async fn swap_ticket_set_config(&self, _: &str, _: &str, _: f32, _: u64, _: &str) -> Result<(), StateError> { Ok(()) }

        async fn shadow_sample_record(&self, ticket_id: &str, _: u64, _: &str, _: &str, _: &str, _: &str, _: bool, reason: &str, _: &str) -> Result<(), StateError> {
            self.sentinel_samples.lock().unwrap().push((ticket_id.to_string(), reason.to_string()));
            Ok(())
        }

        async fn shadow_tickets_due(&self, _: &str) -> Result<Vec<SwapTicketRecord>, StateError> {
            Ok(self.tickets.lock().unwrap().clone())
        }

        async fn shadow_samples_for(&self, ticket_id: &str) -> Result<Vec<ShadowSampleRecord>, StateError> {
            Ok(self
                .samples_by_ticket
                .lock()
                .unwrap()
                .get(ticket_id)
                .cloned()
                .unwrap_or_default())
        }

        async fn shadow_green_tickets(&self) -> Result<Vec<SwapTicketRecord>, StateError> {
            Ok(self
                .tickets
                .lock()
                .unwrap()
                .iter()
                .filter(|t| t.state == "shadow_green")
                .cloned()
                .collect())
        }
    }

    fn ticket(id: &str, criteria: Vec<SuccessCriterion>, started_minutes_ago: i64, window_seconds: u64) -> SwapTicketRecord {
        let started = Utc::now() - chrono::Duration::minutes(started_minutes_ago);
        SwapTicketRecord {
            id: id.into(),
            project_id: "test".into(),
            port_id: "inference".into(),
            incumbent_adapter_id: "mock-a".into(),
            candidate_adapter_id: "mock-b".into(),
            candidate_manifest_json: "{}".into(),
            state: "shadow".into(),
            shadow_traffic_fraction: 1.0,
            shadow_window_seconds: window_seconds,
            shadow_started_at: started.to_rfc3339(),
            success_criteria_json: serde_json::to_string(&criteria).unwrap(),
            created_at: started.to_rfc3339(),
            updated_at: started.to_rfc3339(),
        }
    }

    fn sample(call_seq: u64, agreed: bool, latency_ms: u64, candidate_error: bool) -> ShadowSampleRecord {
        ShadowSampleRecord {
            id: call_seq,
            ticket_id: "t1".into(),
            call_seq,
            incumbent_adapter_id: "mock-a".into(),
            candidate_adapter_id: "mock-b".into(),
            incumbent_metrics_json: r#"{"latency_ms":50,"error":false}"#.into(),
            candidate_metrics_json: serde_json::json!({
                "latency_ms": latency_ms,
                "error": candidate_error,
            }).to_string(),
            agreed,
            reason: String::new(),
            recorded_at: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn promotion_judge_promotes_to_shadow_green_when_all_criteria_pass() {
        let state = Arc::new(StubState::default());
        state.add_ticket(ticket(
            "t1",
            vec![
                SuccessCriterion::ResponseEquivalence { tolerance: 0.05 },
                SuccessCriterion::LatencyP99BelowMs(200),
                SuccessCriterion::ErrorRateBelow(0.1),
            ],
            10,
            300, // 5-minute window, started 10 min ago → due
        ));
        state.add_samples(
            "t1",
            (1..=20)
                .map(|i| sample(i, true, 100, false))
                .collect(),
        );
        let judge = PromotionJudge::new(state.clone());
        let report = judge.tick().await;
        assert_eq!(report.due, 1);
        assert_eq!(report.promoted_to_green, vec!["t1"]);
        assert!(report.marked_red.is_empty());
        assert_eq!(state.transitions.lock().unwrap()[0], ("t1".into(), "shadow_green".into()));
    }

    #[tokio::test]
    async fn promotion_judge_marks_shadow_red_on_high_disagreement() {
        let state = Arc::new(StubState::default());
        state.add_ticket(ticket(
            "t1",
            vec![SuccessCriterion::ResponseEquivalence { tolerance: 0.05 }],
            10,
            300,
        ));
        // 50% disagreement, way over 5% tolerance.
        state.add_samples(
            "t1",
            (1..=10)
                .map(|i| sample(i, i % 2 == 0, 100, false))
                .collect(),
        );
        let judge = PromotionJudge::new(state.clone());
        let report = judge.tick().await;
        assert_eq!(report.due, 1);
        assert!(report.promoted_to_green.is_empty());
        assert_eq!(report.marked_red.len(), 1);
        assert!(report.marked_red[0].1.contains("ResponseEquivalence"));
        assert_eq!(state.transitions.lock().unwrap()[0], ("t1".into(), "shadow_red".into()));
        // Sentinel sample recorded with the rejection reason.
        assert_eq!(state.sentinel_samples.lock().unwrap()[0].0, "t1");
        assert!(state.sentinel_samples.lock().unwrap()[0].1.contains("ResponseEquivalence"));
    }

    #[tokio::test]
    async fn promotion_judge_marks_shadow_red_on_high_error_rate() {
        let state = Arc::new(StubState::default());
        state.add_ticket(ticket(
            "t1",
            vec![SuccessCriterion::ErrorRateBelow(0.1)],
            10,
            300,
        ));
        // 50% candidate error rate, ceiling is 10%.
        state.add_samples(
            "t1",
            (1..=10)
                .map(|i| sample(i, false, 100, i % 2 == 0))
                .collect(),
        );
        let judge = PromotionJudge::new(state.clone());
        let report = judge.tick().await;
        assert_eq!(report.marked_red.len(), 1);
        assert!(report.marked_red[0].1.contains("ErrorRateBelow"));
    }

    #[tokio::test]
    async fn promotion_judge_marks_shadow_red_on_high_latency_p99() {
        let state = Arc::new(StubState::default());
        state.add_ticket(ticket(
            "t1",
            vec![SuccessCriterion::LatencyP99BelowMs(150)],
            10,
            300,
        ));
        // 9 fast + 1 slow → p99 is the slow one (5000ms).
        let mut samples: Vec<_> = (1..=9).map(|i| sample(i, true, 100, false)).collect();
        samples.push(sample(10, true, 5000, false));
        state.add_samples("t1", samples);
        let judge = PromotionJudge::new(state.clone());
        let report = judge.tick().await;
        assert_eq!(report.marked_red.len(), 1);
        assert!(report.marked_red[0].1.contains("LatencyP99BelowMs"));
    }

    #[tokio::test]
    async fn promotion_judge_skips_tickets_whose_window_has_not_elapsed() {
        let state = Arc::new(StubState::default());
        state.add_ticket(ticket(
            "t1",
            vec![SuccessCriterion::ResponseEquivalence { tolerance: 0.05 }],
            1,    // started 1 min ago
            3600, // 60-min window → not due
        ));
        let judge = PromotionJudge::new(state.clone());
        let report = judge.tick().await;
        assert_eq!(report.considered, 1);
        assert_eq!(report.due, 0);
        assert!(report.promoted_to_green.is_empty());
        assert!(report.marked_red.is_empty());
        assert!(state.transitions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn promotion_judge_marks_red_when_no_samples_recorded() {
        let state = Arc::new(StubState::default());
        state.add_ticket(ticket(
            "t1",
            vec![SuccessCriterion::ResponseEquivalence { tolerance: 0.05 }],
            10,
            300,
        ));
        // No samples added.
        let judge = PromotionJudge::new(state.clone());
        let report = judge.tick().await;
        assert_eq!(report.marked_red.len(), 1);
        assert!(report.marked_red[0].1.contains("no shadow samples"));
    }

    #[tokio::test]
    async fn promotion_judge_default_passes_when_no_criteria_specified() {
        let state = Arc::new(StubState::default());
        state.add_ticket(ticket("t1", vec![], 10, 300));
        state.add_samples("t1", vec![sample(1, true, 100, false)]);
        let judge = PromotionJudge::new(state.clone());
        let report = judge.tick().await;
        // Empty criteria -> default pass; caller's responsibility to attach
        // meaningful gates when proposing a swap.
        assert_eq!(report.promoted_to_green, vec!["t1"]);
    }
}
