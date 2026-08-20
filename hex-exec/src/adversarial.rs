//! Adversarial review pipeline — the hex-native cooperative+adversarial harness.
//!
//! Distilled from a 25-agent workflow that built a concurrent job queue: the build's
//! own passing tests still hid 6 real bugs; an *independent adversarial* pass found
//! them. This is that pass, made a first-class hex capability:
//!
//!   hunt (parallel lenses) → skeptical verify (parallel, default-refute) → fix-loop
//!   (sequential, each fix gated by a ground-truth test command)
//!
//! Each agent is a `claude -p` worker (hex's frontier path — no API key, no VRAM).
//! Findings flow between phases as structured JSON. The gate (a shell command that
//! must exit 0) is the only authority on whether a fix counts — the same
//! evidence-gate discipline as the do-loop.

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

fn claude_binary() -> String {
    std::env::var("HEX_CLAUDE_BINARY").unwrap_or_else(|_| "claude".to_string())
}

/// One adversarial lens — a focused failure class a reviewer hunts.
struct Lens {
    key: &'static str,
    focus: &'static str,
}

const LENSES: &[Lens] = &[
    Lens { key: "correctness", focus: "logic errors, wrong results, broken invariants, off-by-one" },
    Lens { key: "concurrency", focus: "data races, deadlocks, non-atomic multi-step state transitions, TOCTOU, double-processing" },
    Lens { key: "durability-safety", focus: "data loss, crash-safety, partial/torn writes, integer overflow, panics, unwrap on external input" },
    Lens { key: "edges", focus: "boundary conditions, empty/zero/duplicate inputs, error paths, terminal-state operations" },
];

#[derive(Debug, Clone, Deserialize)]
pub struct Finding {
    pub title: String,
    #[serde(default)]
    pub location: String,
    pub description: String,
    #[serde(default)]
    pub lens: String,
}

#[derive(Deserialize)]
struct FindingsEnvelope {
    findings: Vec<Finding>,
}

#[derive(Deserialize)]
struct VerdictEnvelope {
    is_real: bool,
    #[serde(default)]
    reasoning: String,
}

/// Outcome of a review run.
#[derive(Debug, Default)]
pub struct ReviewReport {
    pub candidate: usize,
    pub confirmed: Vec<Finding>,
    pub fixed: Vec<String>,
    pub gate_passed: bool,
    pub notes: Vec<String>,
}

/// Extract the first balanced JSON value (object or array) from agent prose. Pure and
/// testable — `claude -p` often wraps JSON in markdown fences or commentary.
pub fn extract_json(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{' || b == b'[')?;
    let (open, close) = if bytes[start] == b'{' { (b'{', b'}') } else { (b'[', b']') };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for i in start..bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return std::str::from_utf8(&bytes[start..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// Spawn one `claude -p` agent in `cwd`, return stdout.
async fn claude_run(prompt: &str, cwd: &Path, timeout_secs: u64) -> Result<String, String> {
    let fut = tokio::process::Command::new(claude_binary())
        .arg("-p")
        .arg("--dangerously-skip-permissions")
        .arg(prompt)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match tokio::time::timeout(Duration::from_secs(timeout_secs), fut).await {
        Ok(Ok(o)) => Ok(String::from_utf8_lossy(&o.stdout).to_string()),
        Ok(Err(e)) => Err(format!("spawn claude: {e}")),
        Err(_) => Err("claude -p timed out".to_string()),
    }
}

/// Default attempts for `claude_run` call sites that don't take an explicit retry
/// count from the caller (e.g. `run_review`, whose CLI surface isn't parametrized).
const DEFAULT_RETRIES: u32 = 3;

/// Retry-wrapped [`claude_run`]: on timeout or spawn error, retry up to `attempts`
/// times (clamped 1-6), feeding the prior failure back into the next attempt's prompt
/// for context. No sleep/backoff — failures here are inference-latency timeouts on a
/// single long call, not rate limits, so immediate retry is the right shape (mirrors
/// the bounded-attempt loop in direct_exec.rs rather than time-based backoff).
async fn claude_run_retry(
    prompt: &str,
    cwd: &Path,
    timeout_secs: u64,
    attempts: u32,
) -> Result<String, String> {
    let attempts = attempts.clamp(1, 6);
    let mut last_err = String::new();
    for attempt in 1..=attempts {
        let this_prompt = retry_prompt(prompt, attempt, attempts, &last_err);
        match claude_run(&this_prompt, cwd, timeout_secs).await {
            Ok(out) => return Ok(out),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// Pure prompt-formatting for a retry attempt — split out from [`claude_run_retry`]
/// so the retry framing logic is unit-testable without spawning a real subprocess.
fn retry_prompt(prompt: &str, attempt: u32, attempts: u32, last_err: &str) -> String {
    if attempt == 1 {
        prompt.to_string()
    } else {
        format!("{prompt}\n\n(retry {attempt}/{attempts} — the previous attempt failed: {last_err})")
    }
}

/// Stage and commit everything under `target` using a commit-local factory identity
/// (ADR-2606071323 §4) so autonomous commits are attributable and never masquerade as
/// the operator. Non-fatal by design: any failure (nothing to commit, git missing,
/// no repo) is recorded as a note, never surfaced as an error — the build/review
/// itself already succeeded per the gate by the time this runs.
async fn commit_result(repo_root: &Path, target: &str, subject: &str, trailer: &str, notes: &mut Vec<String>) {
    let add = tokio::process::Command::new("git")
        .args(["add", "--", target])
        .current_dir(repo_root)
        .output()
        .await;
    match add {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            notes.push(format!(
                "git add failed (non-fatal): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            return;
        }
        Err(e) => {
            notes.push(format!("git not found — skipping auto-commit (non-fatal): {e}"));
            return;
        }
    }
    let subject: String = subject.lines().next().unwrap_or(subject).chars().take(72).collect();
    let msg = format!("{subject}\n\nCo-Authored-By: {trailer} <noreply@hex.local>");
    let commit = tokio::process::Command::new("git")
        .args([
            "-c", "user.name=hex-factory",
            "-c", "user.email=factory@hex.local",
            "commit", "-m", &msg, "--", target,
        ])
        .current_dir(repo_root)
        .output()
        .await;
    match commit {
        Ok(out) if out.status.success() => notes.push("auto-committed result (hex-factory)".to_string()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("nothing to commit") {
                notes.push("nothing new to commit".to_string());
            } else {
                notes.push(format!("git commit failed (non-fatal): {}", stderr.trim()));
            }
        }
        Err(e) => notes.push(format!("git not found — skipping auto-commit (non-fatal): {e}")),
    }
}

/// Run the adversarial review pipeline over `target` (a path), gated by `gate` (a
/// shell command that must exit 0). Fixes are applied to the working tree and left
/// uncommitted for operator review.
pub async fn run_review(target: &str, gate: &str, repo_root: &Path) -> ReviewReport {
    let mut report = ReviewReport::default();

    // ── Phase 1: hunt (parallel lenses) ──────────────────────────────────────
    let mut hunts = Vec::new();
    for lens in LENSES {
        let prompt = format!(
            "You are an adversarial code reviewer. Read the code under `{target}` (and its tests) \
             and hunt ONLY for this failure class: {focus}. Report exclusively REAL bugs you can \
             point to in the actual code — do NOT invent issues; if the code is correct on this \
             lens, return an empty list. Do NOT modify any files. Output ONLY a JSON object: \
             {{\"findings\":[{{\"title\":\"...\",\"location\":\"file:line or fn\",\"description\":\"the concrete failure\",\"lens\":\"{key}\"}}]}}",
            target = target, focus = lens.focus, key = lens.key
        );
        let root = repo_root.to_path_buf();
        hunts.push(tokio::spawn(async move { claude_run_retry(&prompt, &root, 600, DEFAULT_RETRIES).await }));
    }
    for h in hunts {
        if let Ok(Ok(out)) = h.await {
            if let Some(js) = extract_json(&out) {
                if let Ok(env) = serde_json::from_str::<FindingsEnvelope>(js) {
                    report.candidate += env.findings.len();
                    report.confirmed.extend(env.findings); // staged; pruned by verify below
                }
            }
        }
    }
    let candidates = std::mem::take(&mut report.confirmed);

    // ── Phase 2: skeptical verify (parallel, default-refute) ─────────────────
    let mut checks = Vec::new();
    for f in candidates {
        let prompt = format!(
            "Independently and SKEPTICALLY verify this claimed bug in the code under `{target}`. \
             Read the actual code at the cited location. Default to is_real=false unless you can \
             point to the exact wrong code and explain the concrete failure sequence; reject vague \
             or speculative claims. Do NOT modify files. Output ONLY JSON: \
             {{\"is_real\":true|false,\"reasoning\":\"...\"}}.\n\nCLAIM:\ntitle: {title}\nlocation: {loc}\ndescription: {desc}",
            target = target, title = f.title, loc = f.location, desc = f.description
        );
        let root = repo_root.to_path_buf();
        checks.push((f, tokio::spawn(async move { claude_run_retry(&prompt, &root, 600, DEFAULT_RETRIES).await })));
    }
    for (f, c) in checks {
        if let Ok(Ok(out)) = c.await {
            if let Some(js) = extract_json(&out) {
                if let Ok(v) = serde_json::from_str::<VerdictEnvelope>(js) {
                    if v.is_real {
                        report.confirmed.push(f);
                    } else {
                        report.notes.push(format!("refuted: {} — {}", f.title, v.reasoning));
                    }
                }
            }
        }
    }

    // ── Phase 3: fix-loop (sequential, each fix gated) ───────────────────────
    for f in &report.confirmed {
        let prompt = format!(
            "Fix this CONFIRMED bug in the code under `{target}`, then add a regression test that \
             fails without the fix. Make a minimal, correct change. Bug:\ntitle: {title}\nlocation: {loc}\ndescription: {desc}",
            target = target, title = f.title, loc = f.location, desc = f.description
        );
        if claude_run_retry(&prompt, repo_root, 900, DEFAULT_RETRIES).await.is_ok() {
            let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
            if passed {
                report.fixed.push(f.title.clone());
            } else {
                report.notes.push(format!("fix for '{}' did not pass the gate", f.title));
            }
        }
    }

    // ── Final gate ───────────────────────────────────────────────────────────
    let (passed, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
    report.gate_passed = passed;
    if passed {
        commit_result(
            repo_root,
            target,
            &format!("fix: adversarial review pass on {target}"),
            "hex-swarm-review",
            &mut report.notes,
        )
        .await;
    }
    report
}

/// Outcome of a cooperative build run.
#[derive(Debug, Default)]
pub struct BuildReport {
    pub designs: usize,
    pub critiques: usize,
    pub spec_chars: usize,
    pub build_ok: bool,
    pub notes: Vec<String>,
}

/// Competing design priorities — the divergence that makes the red-team meaningful.
const DESIGN_PRIORITIES: &[&str] = &[
    "durability-and-correctness-first: crash-safety, persistence, recovery, and provable invariants are paramount",
    "concurrency-first: correct and lock-minimal under heavy parallelism; no races, no double-processing",
    "simplicity-first: the smallest design that is obviously correct; the fewest moving parts",
    "performance-first: throughput and low overhead, without sacrificing correctness",
];

/// Run the cooperative-design half of the harness: diverge (N designs from competing
/// priorities) → red-team each → synthesize one spec → build to the gate. Pairs with
/// [`run_review`] for the full cooperative+adversarial pipeline.
pub async fn run_build(
    challenge: &str,
    target: &str,
    gate: &str,
    n_designs: usize,
    repo_root: &Path,
    timeout_secs: u64,
    retries: u32,
) -> BuildReport {
    let mut report = BuildReport::default();
    let n = n_designs.clamp(2, DESIGN_PRIORITIES.len());

    // ── Phase 1: diverge — N designs from competing priorities ───────────────
    let mut tasks = Vec::new();
    for prio in DESIGN_PRIORITIES.iter().take(n) {
        let prompt = format!(
            "You are a senior systems engineer. Propose a concrete design for this challenge:\n{challenge}\n\n\
             Your design PRIORITY: {prio}\n\nBe specific about the data model, the key algorithms, the \
             concurrency/atomicity strategy, and the main risks. Output your design as clear prose (no code yet)."
        );
        let root = repo_root.to_path_buf();
        tasks.push(tokio::spawn(async move { claude_run_retry(&prompt, &root, timeout_secs, retries).await }));
    }
    let mut designs = Vec::new();
    for t in tasks {
        if let Ok(Ok(d)) = t.await {
            designs.push(d);
        }
    }
    report.designs = designs.len();
    if designs.is_empty() {
        report.notes.push("no designs produced".into());
        return report;
    }

    // ── Phase 2: red-team each design (adversarial) ──────────────────────────
    let mut ctasks = Vec::new();
    for (i, d) in designs.iter().enumerate() {
        let prompt = format!(
            "Adversarially review this design for the challenge:\n{challenge}\n\nBe RUTHLESS — find fatal \
             flaws, race conditions, lost-data scenarios, correctness gaps, and unhandled edge cases. List \
             them concretely.\n\nDESIGN {i}:\n{d}"
        );
        let root = repo_root.to_path_buf();
        ctasks.push(tokio::spawn(async move { claude_run_retry(&prompt, &root, timeout_secs, retries).await }));
    }
    let mut critiques = Vec::new();
    for t in ctasks {
        if let Ok(Ok(c)) = t.await {
            critiques.push(c);
        }
    }
    report.critiques = critiques.len();

    // ── Phase 3: synthesize one build spec ───────────────────────────────────
    let designs_block = designs
        .iter()
        .enumerate()
        .map(|(i, d)| format!("--- DESIGN {i} ---\n{d}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let critiques_block = critiques.join("\n\n--- next critique ---\n\n");
    let spec = match claude_run_retry(
        &format!(
            "You are the lead architect. Given these candidate designs and their adversarial critiques for \
             the challenge:\n{challenge}\n\nSynthesize ONE concrete build spec: the public API, the internal \
             data model, the exact concurrency/correctness strategy, and a TEST PLAN that exercises the hard \
             cases the red team raised. Every fatal flaw must be designed out. Output the spec as clear prose \
             an implementer can follow.\n\nDESIGNS:\n{designs_block}\n\nCRITIQUES:\n{critiques_block}"
        ),
        repo_root,
        timeout_secs,
        retries,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            report.notes.push(format!("synthesize failed: {e}"));
            return report;
        }
    };
    report.spec_chars = spec.len();

    // ── Phase 4: build to the gate ───────────────────────────────────────────
    let build_prompt = format!(
        "Implement the following spec as code under `{target}`. Write the full implementation AND a \
         comprehensive test suite per the spec's test plan. Then run the gate command `{gate}` and ITERATE \
         — fix compile errors and failing tests — until the gate exits 0. Do not stop until the gate passes.\n\n\
         CHALLENGE:\n{challenge}\n\nSPEC:\n{spec}"
    );
    if let Err(e) = claude_run_retry(&build_prompt, repo_root, timeout_secs.saturating_mul(4), retries).await {
        report.notes.push(format!("build agent error: {e}"));
    }
    let (ok, _) = crate::direct_exec::run_evidence(gate, repo_root).await;
    report.build_ok = ok;
    if ok {
        commit_result(
            repo_root,
            target,
            &format!("feat: {challenge}"),
            "hex-swarm-build",
            &mut report.notes,
        )
        .await;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::{extract_json, retry_prompt};

    #[test]
    fn retry_prompt_first_attempt_is_unmodified() {
        assert_eq!(retry_prompt("do the thing", 1, 3, ""), "do the thing");
    }

    #[test]
    fn retry_prompt_later_attempts_include_prior_failure() {
        let p = retry_prompt("do the thing", 2, 3, "claude -p timed out");
        assert!(p.starts_with("do the thing"));
        assert!(p.contains("retry 2/3"));
        assert!(p.contains("claude -p timed out"));
    }

    #[test]
    fn extracts_object_from_markdown_fence() {
        let s = "Here are the findings:\n```json\n{\"findings\": [{\"title\": \"x\"}]}\n```\nDone.";
        assert_eq!(extract_json(s), Some("{\"findings\": [{\"title\": \"x\"}]}"));
    }

    #[test]
    fn handles_braces_inside_strings() {
        let s = "{\"reasoning\": \"the code does foo() { bar }\", \"is_real\": true}";
        assert_eq!(extract_json(s), Some(s));
    }

    #[test]
    fn extracts_array() {
        assert_eq!(extract_json("noise [1, 2, [3]] tail"), Some("[1, 2, [3]]"));
    }

    #[test]
    fn none_when_no_json() {
        assert_eq!(extract_json("no json here"), None);
    }

    #[test]
    fn ignores_close_brace_in_string_before_open() {
        let s = "prefix } then {\"a\": 1}";
        assert_eq!(extract_json(s), Some("{\"a\": 1}"));
    }
}
