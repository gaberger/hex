//! Improver judge phase (ADR-2026-04-27-1100 P3).
//!
//! Takes the unordered hypothesis stream from [`discover`] and scores each
//! one so [`act`] can pick the top-N. Scoring is deliberately a closed
//! formula over (severity, source, evidence) — no inference, no tuning —
//! so the ranking is reproducible and reviewable in CI.
//!
//! The score is a u32 in [0, 100] with these components:
//!
//! * Severity base    : Error 60, Warning 30, Info 10
//! * Source modifier  : per-detector multiplier reflecting how blocking the
//!                      finding is to the rest of the loop. Detector_health
//!                      hypotheses get the highest weight because a broken
//!                      detector can hide every downstream finding.
//! * Evidence boost   : small bumps for evidence-shape signals — `kind:
//!                      "abandoned"` outranks `kind: "deprecated"`, P2
//!                      inbox outranks P1, etc.
//!
//! The sum is clamped to [0, 100]. Equal scores break ties by source name
//! then scope so the ordering is deterministic.
//!
//! [`discover`]: super::discover
//! [`act`]: super::act

use serde::Serialize;

use super::discover::{Hypothesis, Severity, Source};

/// A judged hypothesis — discover output + score + reason. Reason exists so
/// operators (and the ADR-review surface) can inspect *why* something
/// ranked where it did, not just the number.
#[derive(Debug, Clone, Serialize)]
pub struct ScoredHypothesis {
    #[serde(flatten)]
    pub hypothesis: Hypothesis,
    pub score: u32,
    pub reason: String,
}

/// Rank a hypothesis stream highest-score-first. Ties break deterministically
/// (source name asc, then scope asc) so two judge runs over the same
/// discover output produce identical ordering.
pub fn rank(hypotheses: &[Hypothesis]) -> Vec<ScoredHypothesis> {
    let mut scored: Vec<ScoredHypothesis> = hypotheses
        .iter()
        .map(|h| {
            let (score, reason) = score(h);
            ScoredHypothesis {
                hypothesis: h.clone(),
                score,
                reason,
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| format!("{:?}", a.hypothesis.source).cmp(&format!("{:?}", b.hypothesis.source)))
            .then_with(|| a.hypothesis.scope.cmp(&b.hypothesis.scope))
    });
    scored
}

/// Closed-formula score for one hypothesis. Returns (score, reason).
pub fn score(h: &Hypothesis) -> (u32, String) {
    // Detector_health hypotheses come from the homeostatic discover guard
    // (a detector that couldn't produce parseable findings tags itself).
    // These are pre-eminent — every downstream finding the broken detector
    // would have produced is invisible until the surface is fixed.
    let is_health = h
        .evidence
        .get("detector_health")
        .and_then(|v| v.as_str())
        .is_some();
    if is_health {
        // Health hypotheses get the absolute ceiling — a broken detector
        // blocks every downstream finding it would have produced, so even
        // the most urgent real finding (P2 inbox at full boost = 100) must
        // not outrank surface repair.
        return (
            100,
            "detector_health: broken detector blocks downstream findings".to_string(),
        );
    }

    let severity_base: u32 = match h.severity {
        Severity::Error => 60,
        Severity::Warning => 30,
        Severity::Info => 10,
    };
    let source_mod: u32 = match h.source {
        // BuildReadiness findings flag broken builds — code is supposed
        // to compile and doesn't. Highest weight among real findings
        // (above WorkplanIntegrity 35) because a broken build blocks
        // every downstream signal: tests can't run, agents can't dispatch,
        // operator can't ship. Critically also dominates LayerCoverage so
        // that "the code in those layers compiles" beats "more layers
        // exist" in ranking.
        Source::BuildReadiness => 38,
        // TestCoverage findings flag uncovered code — important quality
        // work but not blocking the way build failures are. Score above
        // LayerCoverage (an empty layer is a stub; an untested layer
        // is potentially-broken code masquerading as working).
        Source::TestCoverage => 16,
        // LayerCoverage findings flag missing canonical architecture
        // layers — significant structural work to add. Score lower than
        // active drift sources (active drift has someone using broken
        // surfaces; missing layers represent unbuilt scaffolding).
        Source::LayerCoverage => 12,
        // WorkplanIntegrity findings flag destructive action quality —
        // the system corrupted a file while clearing a hypothesis.
        // Higher than QStarvation because corruption is actively
        // harmful, not just unproductive: every additional auto-act
        // tick can compound the damage.
        Source::WorkplanIntegrity => 35,
        // Q-starvation findings flag broken action mappings — like
        // detector_health, they block the loop's ability to make
        // progress. Sit just below detector_health (which gets a flat
        // 100) so they always rank near the top.
        Source::QStarvation => 30,
        // Inbox criticals are operator-attention work — outrank everything
        // architectural since a stuck P2 means the system is asking for help.
        Source::InboxStale => 25,
        // Escalation == model mis-route, expensive in compute every tick.
        Source::EscalationReport => 20,
        // ADR registry integrity feeds every other planning surface.
        Source::AdrDoctor => 18,
        // Evidence drift means workplans claim work that didn't happen —
        // erodes the trust the rest of the loop is built on.
        Source::ReconcileStrict => 15,
        // Lifecycle drift (stale Proposed, unlinked Superseded) is medium —
        // signals planning debt but doesn't actively break anything.
        Source::AdrLifecycle => 10,
        // Punch-list and git drift are softer signals — noisy without being
        // urgent, so they sit at the bottom of the pile.
        Source::PunchList => 8,
        Source::GitDrift => 5,
    };

    let evidence_boost: u32 = evidence_boost(h);

    // Q-table contribution: per-source learned offset from prior outcomes.
    // Bounded ±10 in learn::q_offset so the static formula remains
    // authoritative until enough samples accumulate. Subtracting a positive
    // offset would never make sense (rewards favor the action) so we treat
    // negative offsets as a small demotion.
    let q = super::learn::q_offset(&super::learn::load_q_table(), h.source);
    let q_component: i32 = q.clamp(-10, 10);

    let static_total = severity_base
        .saturating_add(source_mod)
        .saturating_add(evidence_boost) as i32;
    let with_q = (static_total + q_component).max(0) as u32;
    // Cap real findings at 99 so detector_health (100) always wins ties.
    let clamped = with_q.min(99);
    let reason = format!(
        "severity={}+source={}+evidence={}{}",
        severity_base,
        source_mod,
        evidence_boost,
        if q_component != 0 {
            format!("+q={:+}", q_component)
        } else {
            String::new()
        }
    );
    (clamped, reason)
}

/// Per-source evidence-shape adjustments. Caps at 15 so even the punchiest
/// evidence boost can't outrank a high-severity hypothesis from a more
/// load-bearing detector.
fn evidence_boost(h: &Hypothesis) -> u32 {
    let kind = h
        .evidence
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match h.source {
        Source::AdrLifecycle => match kind {
            "abandoned" => 8,
            "unparseable_status" => 10,
            "proposed" => 4,
            "superseded" | "deprecated" => 2,
            _ => 0,
        },
        Source::ReconcileStrict => match kind {
            "done_without_evidence" => 8,
            "evidence_missing_workplan_id" => 4,
            _ => 0,
        },
        Source::InboxStale => {
            let priority = h
                .evidence
                .get("priority")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            match priority {
                p if p >= 2 => 15,
                1 => 6,
                _ => 0,
            }
        }
        Source::EscalationReport => {
            let rate = h
                .evidence
                .get("rate")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if rate > 0.8 {
                12
            } else if rate > 0.6 {
                6
            } else {
                0
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn fixture(source: Source, severity: Severity, evidence: serde_json::Value) -> Hypothesis {
        Hypothesis {
            id: format!("hyp-{}-{:?}", source as u8, severity),
            source,
            scope: format!("scope-{:?}", source),
            severity,
            evidence,
            generated_at: Utc::now(),
        }
    }

    #[test]
    fn detector_health_outranks_real_findings() {
        // A broken detector beats every real finding because it blocks
        // every downstream signal the broken detector would have produced.
        let health = fixture(
            Source::GitDrift,
            Severity::Error,
            json!({"detector_health": "spawn_or_exit_error", "cmd": "broken"}),
        );
        let critical_inbox = fixture(
            Source::InboxStale,
            Severity::Error,
            json!({"priority": 2, "kind": "p2_overdue"}),
        );
        let ranked = rank(&[critical_inbox.clone(), health.clone()]);
        assert_eq!(ranked[0].hypothesis.source, Source::GitDrift, "health first");
        assert_eq!(ranked[1].hypothesis.source, Source::InboxStale);
    }

    #[test]
    fn p2_inbox_outranks_p0_inbox_at_same_severity() {
        let p2 = fixture(
            Source::InboxStale,
            Severity::Warning,
            json!({"priority": 2}),
        );
        let p0 = fixture(
            Source::InboxStale,
            Severity::Warning,
            json!({"priority": 0}),
        );
        let ranked = rank(&[p0.clone(), p2.clone()]);
        assert!(
            ranked[0].score > ranked[1].score,
            "p2 must beat p0: {} vs {}",
            ranked[0].score,
            ranked[1].score
        );
    }

    #[test]
    fn deterministic_ties_break_on_scope() {
        let a = Hypothesis {
            id: "a".into(),
            source: Source::AdrDoctor,
            scope: "ADR-A".into(),
            severity: Severity::Warning,
            evidence: json!({}),
            generated_at: Utc::now(),
        };
        let b = Hypothesis {
            id: "b".into(),
            source: Source::AdrDoctor,
            scope: "ADR-B".into(),
            severity: Severity::Warning,
            evidence: json!({}),
            generated_at: Utc::now(),
        };
        let r1 = rank(&[a.clone(), b.clone()]);
        let r2 = rank(&[b.clone(), a.clone()]);
        assert_eq!(r1[0].hypothesis.scope, r2[0].hypothesis.scope);
    }

    #[test]
    fn real_findings_clamp_to_99_health_owns_100() {
        // The most-boosted real finding tops out at 99 so detector_health
        // (which scores a flat 100) always wins. Lets the health signal
        // act as a hard precedence rule rather than a soft heuristic.
        let h = fixture(
            Source::InboxStale,
            Severity::Error,
            json!({"priority": 5}),
        );
        let (s, _) = score(&h);
        assert!(s <= 99, "real-finding score must cap below 100: {}", s);

        let health = fixture(
            Source::GitDrift,
            Severity::Info,
            json!({"detector_health": "non_json_stdout", "cmd": "x"}),
        );
        let (hs, _) = score(&health);
        assert_eq!(hs, 100);
    }
}
