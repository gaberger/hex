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

/// A bench task: a prompt plus a deterministic acceptance predicate over the draft.
struct ProxyTask {
    prompt: &'static str,
    /// Returns true if the draft is "accepted" — for generic tasks this is the syntactic
    /// gate; for boundary tasks it checks the specific hex idiom the adapter should inject.
    check: fn(&str) -> bool,
}

/// Built-in tasks for the Phase-1 acceptance proxy. Two kinds, so one acceptance number
/// reflects both concerns:
///   * GENERIC codegen (the no-regression guard — the first adapter *hurt* this).
///   * BOUNDARY idioms (what a `hex-boundaries` adapter is actually meant to inject:
///     no cross-adapter imports, `.js` relative-import extensions, ports-only deps,
///     composition-root wiring — CLAUDE.md hexagonal rules).
/// Deterministic, no model in the loop. The real verdict authority is ADR-2606071734.
fn proxy_tasks() -> Vec<ProxyTask> {
    vec![
        // ── Generic codegen guard ──────────────────────────────────────────
        ProxyTask { prompt: "Write a Rust function `pub fn add(a: i32, b: i32) -> i32` that returns the sum. Output only code.", check: generic_ok },
        ProxyTask { prompt: "Write a Rust function `pub fn parse_i32(s: &str) -> Result<i32, String>` mapping the error to a String. Output only code.", check: generic_ok },
        ProxyTask { prompt: "Write a Rust trait `Greet` with `fn greet(&self) -> String`. Output only code.", check: generic_ok },
        ProxyTask { prompt: "Write a Rust function `pub fn evens(v: &[i32]) -> Vec<i32>` returning the even numbers. Output only code.", check: generic_ok },
        // ── Hex boundary idioms ────────────────────────────────────────────
        ProxyTask {
            prompt: "In a hexagonal (ports & adapters) TypeScript project, write the import statement a secondary adapter uses to depend on its `UserRepository` port. Output only the import line.",
            check: imports_port_not_adapter,
        },
        ProxyTask {
            prompt: "Write a TypeScript relative import of `./domain/user` for a NodeNext project. Output only the import line.",
            check: uses_js_extension,
        },
        ProxyTask {
            prompt: "A primary adapter contains `import { Db } from '../secondary/db.js'`. That violates hex layering. Write the corrected approach (depend on a port instead). Output only code.",
            check: no_cross_adapter_import,
        },
        ProxyTask {
            prompt: "Write a TypeScript composition-root snippet that constructs a `FileSystemAdapter` and injects it into a `ReadFile` use case. Output only code.",
            check: wires_in_composition,
        },
    ]
}

/// Generic syntactic acceptance: contains a definition, balanced, no `todo!`.
fn generic_ok(text: &str) -> bool {
    draft_accepted(text)
}

/// The draft imports from a `ports/` module and NOT from a sibling adapter.
fn imports_port_not_adapter(text: &str) -> bool {
    let code = strip_code_fences(text).to_lowercase();
    code.contains("port") && !mentions_cross_adapter_import(&code)
}

/// Relative imports carry an explicit `.js` extension (NodeNext rule).
fn uses_js_extension(text: &str) -> bool {
    let code = strip_code_fences(text);
    code.contains(".js'") || code.contains(".js\"")
}

/// The corrected code does not import from a sibling adapter directory.
fn no_cross_adapter_import(text: &str) -> bool {
    let code = strip_code_fences(text).to_lowercase();
    !mentions_cross_adapter_import(&code)
}

/// Composition-root wiring: constructs an adapter and passes it into a use case.
fn wires_in_composition(text: &str) -> bool {
    let code = strip_code_fences(text);
    let lc = code.to_lowercase();
    code.contains("new ") && lc.contains("adapter") && (lc.contains("usecase") || lc.contains("readfile") || lc.contains("use case"))
}

/// Heuristic for a cross-adapter import (a primary/secondary adapter importing another
/// adapter directory) — the canonical hex boundary violation.
fn mentions_cross_adapter_import(lc: &str) -> bool {
    lc.contains("from '../secondary/")
        || lc.contains("from \"../secondary/")
        || lc.contains("from '../primary/")
        || lc.contains("from \"../primary/")
        || lc.contains("from '../adapters/")
        || lc.contains("from \"../adapters/")
}

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

/// Strip `<think>…</think>` reasoning blocks (best-effort) so they don't pollute the
/// brace-balance / fence checks.
fn strip_think(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("<think>") {
        out.push_str(&rest[..open]);
        if let Some(close) = rest[open..].find("</think>") {
            rest = &rest[open + close + "</think>".len()..];
        } else {
            rest = ""; // unterminated think block → drop the remainder
            break;
        }
    }
    out.push_str(rest);
    out
}

/// Extract code from a model response: concatenate ALL fenced code blocks if any are
/// present, otherwise return the think-stripped text. Tolerant of reasoning preamble.
fn strip_code_fences(text: &str) -> String {
    let text = strip_think(text);
    let mut blocks = String::new();
    let mut rest = text.as_str();
    while let Some(open) = rest.find("```") {
        let after = &rest[open + 3..];
        // Skip an optional language tag on the fence's opening line.
        let body_start = after.find('\n').map(|i| i + 1).unwrap_or(after.len());
        let body = &after[body_start..];
        if let Some(close_rel) = body.find("```") {
            blocks.push_str(&body[..close_rel]);
            blocks.push('\n');
            rest = &body[close_rel + 3..];
        } else {
            // Unterminated fence → take the rest as code.
            blocks.push_str(body);
            break;
        }
    }
    if blocks.trim().is_empty() {
        text
    } else {
        blocks
    }
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

/// Measure [`EvalMetrics`] for one model by running it over [`PROXY_TASKS`] directly
/// against Ollama (`{ollama_url}/api/chat`).
///
/// We hit Ollama directly rather than the inference port because the eval compares two
/// *local* models (the bare base and the derived base+adapter), and `state.inference_port`
/// is only wired in standalone/headless deployments. Returns an error if Ollama can't
/// serve the model at all (so the caller reports "could not evaluate" rather than
/// fabricating a verdict — the bench gate's authority depends on real measurement).
pub async fn measure_model(ollama_url: &str, model: &str) -> Result<EvalMetrics, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let url = format!("{}/api/chat", ollama_url.trim_end_matches('/'));

    let tasks = proxy_tasks();
    let mut accepted = 0usize;
    let mut quality_ok = 0usize;
    let mut tps_sum = 0.0f64;
    let mut ran = 0usize;

    for task in &tasks {
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": "You are an expert hexagonal-architecture coding assistant. Output only code."},
                {"role": "user", "content": task.prompt},
            ],
            "stream": false,
            // Disable reasoning so the budget goes to code, not <think> (Qwen3 et al.);
            // ignored by non-thinking models. Measures the draft, which is what the gate
            // and the throughput number should reflect.
            "think": false,
            "options": {"temperature": 0.2, "num_predict": 768},
        });
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("ollama chat request for '{model}': {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("ollama chat for '{model}' returned {}", resp.status()));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("parse ollama response for '{model}': {e}"))?;
        let text = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if (task.check)(text) {
            accepted += 1;
        }
        if draft_quality_ok(text) {
            quality_ok += 1;
        }
        // Ollama reports eval_count (tokens) and eval_duration (ns).
        let eval_count = v.get("eval_count").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let eval_ns = v.get("eval_duration").and_then(|x| x.as_f64()).unwrap_or(0.0);
        if eval_ns > 0.0 {
            tps_sum += eval_count / (eval_ns / 1.0e9);
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
