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
        hunts.push(tokio::spawn(async move { claude_run(&prompt, &root, 600).await }));
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
        checks.push((f, tokio::spawn(async move { claude_run(&prompt, &root, 600).await })));
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
        if claude_run(&prompt, repo_root, 900).await.is_ok() {
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
    report
}

#[cfg(test)]
mod tests {
    use super::extract_json;

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
