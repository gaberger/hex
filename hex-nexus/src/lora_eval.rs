//! Bench-gate evaluation for LoRA idiom adapters (ADR-2606161300 §5, Phase 1 step 7).
//!
//! An adapter is promoted to a tier default ONLY when measured first-draft acceptance
//! lifts vs the bare base, with no codegen-quality and no throughput regression beyond
//! budget (spec `bench-gate-acceptance-lift-blocking`). **The verdict authority is the
//! measurement, never the training loss** — this is the guard against the
//! `feedback_qwen36_not_for_codegen` failure mode (a model that only *looks* better).
//!
//! Two pieces here are pure and rigorously tested — the BLOCKING promotion gate
//! ([`decide_promotion`]) and resource-governor admission ([`admit_local_job`], spec
//! `resource-governor-training-admission-negative`). The acceptance *measurement* is a
//! Phase-1 syntactic proxy: it drives the real inference path over a small codegen task
//! set and scores whether each draft is structurally well-formed. The production
//! verdict is the full agentic bench suite (ADR-2606071734); the proxy is honest about
//! being a floor, and every verdict string says which gate produced it.

use std::sync::Arc;

use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};
use hex_core::resource_governor::{self, AdmissionDecision};

/// Metrics captured for one serving configuration (bare base, or base+adapter).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvalMetrics {
    /// First-draft acceptance rate in `[0,1]` — the primary signal.
    pub acceptance: f64,
    /// General codegen quality score in `[0,1]` — a regression guard.
    pub quality: f64,
    /// Throughput in tokens/sec — a regression guard.
    pub tok_per_sec: f64,
}

/// Outcome of the BLOCKING bench gate.
#[derive(Debug, Clone)]
pub struct PromotionVerdict {
    pub promoted: bool,
    pub reason: String,
}

/// Allowed codegen-quality drop before the adapter is rejected.
pub const DEFAULT_QUALITY_TOLERANCE: f64 = 0.02;
/// Allowed throughput drop (fraction of base tok/s) before rejection.
pub const DEFAULT_THROUGHPUT_BUDGET_PCT: f64 = 0.15;

/// Pure BLOCKING promotion gate (spec `bench-gate-acceptance-lift-blocking`).
///
/// Promote IFF first-draft acceptance strictly lifts AND codegen quality has not
/// regressed beyond `quality_tolerance` AND throughput has not regressed beyond
/// `throughput_budget_pct` of the base. Any single failure ⇒ NOT promoted.
pub fn decide_promotion(
    base: &EvalMetrics,
    adapter: &EvalMetrics,
    quality_tolerance: f64,
    throughput_budget_pct: f64,
) -> PromotionVerdict {
    if adapter.acceptance <= base.acceptance {
        return reject(format!(
            "no first-draft acceptance lift (base {:.2} → adapter {:.2})",
            base.acceptance, adapter.acceptance
        ));
    }
    if adapter.quality < base.quality - quality_tolerance {
        return reject(format!(
            "codegen quality regressed beyond tolerance (base {:.2} → adapter {:.2}, tol {:.2})",
            base.quality, adapter.quality, quality_tolerance
        ));
    }
    let min_tps = base.tok_per_sec * (1.0 - throughput_budget_pct);
    if adapter.tok_per_sec < min_tps {
        return reject(format!(
            "throughput regressed beyond budget (base {:.0} → adapter {:.0} tok/s, floor {:.0})",
            base.tok_per_sec, adapter.tok_per_sec, min_tps
        ));
    }
    PromotionVerdict {
        promoted: true,
        reason: format!(
            "acceptance lift {:.2}→{:.2}, quality {:.2}→{:.2}, {:.0}→{:.0} tok/s",
            base.acceptance, adapter.acceptance, base.quality, adapter.quality,
            base.tok_per_sec, adapter.tok_per_sec
        ),
    }
}

fn reject(reason: String) -> PromotionVerdict {
    PromotionVerdict { promoted: false, reason }
}

/// Resource-governor admission for a (potentially heavy) local eval/train job
/// (spec `resource-governor-training-admission-negative`).
///
/// Reads `/proc/meminfo` and asks [`hex_core::resource_governor::admit`] whether the
/// job fits with a safety margin. On refusal (or a missing/unreadable meminfo, treated
/// as zero headroom) it returns `Err(reason)` so the caller DEFERS — it never lets the
/// job proceed into an OOM.
pub fn admit_local_job(
    model_footprint_mb: u64,
    job_headroom_mb: u64,
    safety_mb: u64,
) -> Result<(), String> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let available = resource_governor::parse_mem_available_mb(&meminfo).unwrap_or(0);
    match resource_governor::admit(model_footprint_mb, job_headroom_mb, available, safety_mb) {
        AdmissionDecision::Admit => Ok(()),
        AdmissionDecision::RouteToFrontier => Err(format!(
            "resource governor refused local LoRA eval: need {}MB + {}MB headroom + {}MB safety, only {}MB available — deferred (no OOM)",
            model_footprint_mb, job_headroom_mb, safety_mb, available
        )),
    }
}

/// Built-in codegen tasks for the Phase-1 acceptance proxy. Deliberately small and
/// hex-idiomatic so the proxy is fast; the real suite is ADR-2606071734.
const PROXY_TASKS: &[&str] = &[
    "Write a Rust function `pub fn add(a: i32, b: i32) -> i32` that returns the sum. Output only code.",
    "Write a Rust function that returns the length of a &str. Output only code.",
    "Write a Rust struct `Point { x: f64, y: f64 }` with a `fn norm(&self) -> f64`. Output only code.",
];

/// Score one draft for first-draft acceptance (Phase-1 syntactic proxy): the snippet
/// must contain a `fn`, have balanced braces/parens, and not punt with `todo!()` /
/// `unimplemented!()`. Deterministic — no model in the loop.
fn draft_accepted(text: &str) -> bool {
    let code = strip_code_fences(text);
    if !code.contains("fn ") {
        return false;
    }
    if code.contains("todo!") || code.contains("unimplemented!") {
        return false;
    }
    balanced(&code, '{', '}') && balanced(&code, '(', ')')
}

/// Quality proxy: non-trivial, non-empty body.
fn draft_quality_ok(text: &str) -> bool {
    let code = strip_code_fences(text);
    code.trim().len() >= 40
}

fn strip_code_fences(text: &str) -> String {
    // Keep content between the first pair of ``` fences if present, else the whole text.
    if let Some(open) = text.find("```") {
        let after = &text[open + 3..];
        // skip an optional language tag on the same line
        let body_start = after.find('\n').map(|i| open + 3 + i + 1).unwrap_or(open + 3);
        if let Some(close_rel) = text[body_start..].find("```") {
            return text[body_start..body_start + close_rel].to_string();
        }
    }
    text.to_string()
}

fn balanced(s: &str, open: char, close: char) -> bool {
    let mut depth: i64 = 0;
    for c in s.chars() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        }
    }
    depth == 0
}

/// Measure [`EvalMetrics`] for one model by running it over [`PROXY_TASKS`].
///
/// Returns an error if the inference port can't serve the model at all (so the caller
/// reports "could not evaluate" rather than fabricating a verdict).
pub async fn measure_model(
    inference: &Arc<dyn IInferencePort>,
    model: &str,
) -> Result<EvalMetrics, String> {
    let mut accepted = 0usize;
    let mut quality_ok = 0usize;
    let mut tps_sum = 0.0f64;
    let mut ran = 0usize;

    for task in PROXY_TASKS {
        let req = InferenceRequest {
            model: model.to_string(),
            system_prompt: "You are a Rust codegen assistant. Output only code.".to_string(),
            messages: vec![hex_core::domain::messages::Message::user(task)],
            tools: vec![],
            max_tokens: 512,
            temperature: 0.2,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Low,
            grammar: None,
        };
        let resp = inference
            .complete(req)
            .await
            .map_err(|e| format!("inference failed for model '{model}': {e:?}"))?;
        let text: String = resp
            .content
            .iter()
            .filter_map(|b| match b {
                hex_core::domain::messages::ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        if draft_accepted(&text) {
            accepted += 1;
        }
        if draft_quality_ok(&text) {
            quality_ok += 1;
        }
        if resp.latency_ms > 0 {
            tps_sum += resp.output_tokens as f64 / (resp.latency_ms as f64 / 1000.0);
        }
        ran += 1;
    }

    if ran == 0 {
        return Err("no tasks ran".to_string());
    }
    Ok(EvalMetrics {
        acceptance: accepted as f64 / ran as f64,
        quality: quality_ok as f64 / ran as f64,
        tok_per_sec: tps_sum / ran as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(acceptance: f64, quality: f64, tps: f64) -> EvalMetrics {
        EvalMetrics { acceptance, quality, tok_per_sec: tps }
    }

    #[test]
    fn promotes_on_lift_with_no_regression() {
        let base = m(0.6, 0.8, 50.0);
        let adapter = m(0.8, 0.8, 48.0);
        let v = decide_promotion(&base, &adapter, DEFAULT_QUALITY_TOLERANCE, DEFAULT_THROUGHPUT_BUDGET_PCT);
        assert!(v.promoted, "{}", v.reason);
    }

    #[test]
    fn rejects_without_acceptance_lift() {
        let base = m(0.7, 0.8, 50.0);
        let same = m(0.7, 0.9, 60.0);
        assert!(!decide_promotion(&base, &same, 0.02, 0.15).promoted);
        let worse = m(0.5, 0.9, 60.0);
        assert!(!decide_promotion(&base, &worse, 0.02, 0.15).promoted);
    }

    #[test]
    fn rejects_on_quality_regression_despite_lift() {
        let base = m(0.6, 0.90, 50.0);
        let adapter = m(0.9, 0.80, 50.0); // acceptance up but quality down 0.10 > tol
        assert!(!decide_promotion(&base, &adapter, 0.02, 0.15).promoted);
    }

    #[test]
    fn rejects_on_throughput_regression_despite_lift() {
        let base = m(0.6, 0.8, 100.0);
        let adapter = m(0.9, 0.8, 70.0); // 30% slower > 15% budget
        assert!(!decide_promotion(&base, &adapter, 0.02, 0.15).promoted);
    }

    #[test]
    fn admit_refuses_when_no_headroom() {
        // A 1TB footprint never fits this box → governor refuses (defer, never OOM).
        assert!(admit_local_job(1_000_000, 2_000, 1_000).is_err());
    }

    #[test]
    fn admit_allows_trivial_job() {
        // A 0MB footprint always fits.
        assert!(admit_local_job(0, 0, 0).is_ok());
    }

    #[test]
    fn draft_acceptance_proxy_discriminates() {
        assert!(draft_accepted("```rust\npub fn add(a: i32, b: i32) -> i32 { a + b }\n```"));
        assert!(!draft_accepted("here is some prose with no code"));
        assert!(!draft_accepted("fn add(a: i32) -> i32 { todo!() }"));
        assert!(!draft_accepted("fn broken( { ")); // unbalanced
    }
}
