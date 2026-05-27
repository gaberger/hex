//! Bridge from drafter commitments into the agent loop
//! (wp-sop-agent-loop P3.1).
//!
//! Given an open commitment that names a real repo path, build the
//! task brief, instantiate the four v1 tools, run the agent loop, and
//! return the trajectory's terminal `content` to the drafter. The
//! drafter still owns all of the downstream gates (empty-detect,
//! INSUFFICIENT_CONTEXT, stub-detect, patch-fidelity, content cap)
//! and the `proposed_action_open` STDB call.
//!
//! Activated when `HEX_AGENT_LOOP_ENABLED=1`. With the flag unset the
//! drafter keeps the existing one-shot LLM path; this lets the loop
//! roll out incrementally without bricking the existing SOP path.

use std::path::Path;
use std::sync::Arc;

use crate::orchestration::agent_loop::inference_shim::HttpInferenceShim;
use crate::orchestration::agent_loop::tool::IAgentTool;
use crate::orchestration::agent_loop::trajectory::{AgentStep, TerminatedReason};
use crate::orchestration::agent_loop::tools::{
    CargoCheckTool, CodePatchProposeTool, RepoGrepTool, RepoReadTool,
};
use crate::orchestration::agent_loop::{run, AgentRunInput};

/// Max ReAct iterations per draft. 8 is generous for the four-tool v1
/// surface: a typical draft needs ≤3 reads + 1 cargo_check + 1
/// code_patch_propose. The token budget below catches runaway loops
/// well before step 8 in practice.
const MAX_STEPS: u32 = 8;
/// Cumulative output-token budget. qwen2.5-coder:14b at 14B Q4 averages
/// ~80 tok/s on a 5070 Ti; 32k caps a single trajectory at ~7 minutes
/// wallclock which is the same order as a CEO-direct ask should take.
const MAX_OUTPUT_TOKENS: u64 = 32_000;
/// Compile-gate retry budget (wp-sop-agent-loop P4). On the first
/// trajectory, if the proposed content fails pre-twin `rustc` we
/// append the diagnostics as a synthetic step and re-run the loop
/// once. After that, abstain and let drafter's outer retry cycle
/// (REJECT_BUDGET) handle further attempts.
const PRECOMPILE_RETRIES: usize = 1;

/// Process-local atomic counter used to name precompile workdirs uniquely
/// across parallel callers. Combined with the PID this guarantees no
/// collisions even under concurrent drafter retries.
static PRECOMPILE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Returns the drafted content for the commitment, or an empty string
/// when the trajectory terminated without a terminal action (e.g.
/// MaxSteps, TokenBudget, ParseExhausted, UnknownTool, InferenceFailed).
/// The caller (drafter::draft_one) treats empty as an abstain — same
/// contract as the existing inline path.
pub async fn draft_via_loop(
    role: &str,
    success_artifact: &str,
    action_text: &str,
    ceo_ask: &str,
    inference_url: &str,
    model: &str,
    repo_root: &Path,
    commitment_id: u64,
) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http build: {}", e))?;
    // reqwest::Client is internally Arc'd, so .clone() shares the pool
    // between the shim's inference traffic and the side-band STDB query
    // for prior rejections. No double-pool cost.
    let shim = Arc::new(HttpInferenceShim::new(http.clone(), inference_url.to_string()));

    // wp-sop-agent-loop P5 — seed the trajectory with any prior twin
    // rejections so the persona's next draft can react to what the
    // reviewer said. Without this, drafter keeps re-running the loop
    // from scratch on every poll tick and the persona has no idea why
    // its previous attempts failed.
    let seed_steps: Vec<AgentStep> = fetch_prior_twin_rejections(&http, commitment_id)
        .await
        .unwrap_or_else(|e| {
            tracing::debug!(
                commitment_id, error = %e,
                "drafter_trajectory: could not fetch prior twin rejections (continuing without)"
            );
            Vec::new()
        });
    if !seed_steps.is_empty() {
        tracing::info!(
            commitment_id,
            n_prior_rejections = seed_steps.len(),
            "drafter_trajectory: seeding trajectory with prior twin rejection rationales"
        );
    }
    let initial_seed_len = seed_steps.len();

    let task_brief = render_task_brief(role, success_artifact, action_text, ceo_ask);

    // Run the agent loop, then pre-twin compile-gate the result. If
    // compile fails AND we still have a retry budget, append the
    // diagnostics as a synthetic step + re-run the loop with the
    // updated context so the persona sees the actual error.
    //
    // prior_steps begins with any twin-rejection-derived synthetic steps
    // from P5 (so the very first agent_loop iteration already sees prior
    // verdicts) and grows further on precompile retries (P4).
    let mut prior_steps: Vec<AgentStep> = seed_steps;
    let _ = initial_seed_len; // future telemetry surface
    let mut retries_remaining = PRECOMPILE_RETRIES;
    loop {
        // Tools are owned by the run; rebuilt each retry since AgentRunInput
        // consumes them. Cheap — each constructor just stashes a PathBuf.
        let tools_for_run: Vec<Box<dyn IAgentTool>> = vec![
            Box::new(RepoReadTool::new(repo_root.to_path_buf())),
            Box::new(RepoGrepTool::new(repo_root.to_path_buf())),
            Box::new(CargoCheckTool::new(repo_root.to_path_buf())),
            Box::new(CodePatchProposeTool::new()),
        ];

        let trajectory = run(AgentRunInput {
            role,
            task_brief: &task_brief,
            tools: tools_for_run,
            max_steps: MAX_STEPS,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            inference: shim.clone(),
            model: model.to_string(),
            prior_steps: prior_steps.clone(),
        })
        .await;

        tracing::info!(
            role = %role,
            path = %success_artifact,
            steps = trajectory.steps.len(),
            total_input_tokens = trajectory.total_input_tokens,
            total_output_tokens = trajectory.total_output_tokens,
            total_latency_ms = trajectory.total_latency_ms,
            reason = ?trajectory.terminated_reason,
            retry = (PRECOMPILE_RETRIES - retries_remaining),
            "drafter_trajectory: agent_loop run complete"
        );

        match trajectory.terminated_reason {
            TerminatedReason::TerminalAction => {
                let action = trajectory.final_action.as_ref().cloned();
                let Some(action) = action else {
                    // Defensive — TerminalAction without an action_record
                    // should not happen.
                    return Ok(String::new());
                };
                // Pre-twin compile gate. Catches the markdown-fence + fabricated-
                // imports class of error BEFORE twin spends review budget on it.
                match precompile_check(&action.path, &action.content).await {
                    Ok(()) => return Ok(action.content),
                    Err(diag) if retries_remaining > 0 => {
                        tracing::info!(
                            role = %role,
                            path = %success_artifact,
                            "drafter_trajectory: pre-twin compile failed; re-running with diagnostics"
                        );
                        // Build prior_steps for the retry: previous trajectory's
                        // steps + a synthetic cargo_check observation with the
                        // diagnostics. The persona's next turn sees the error
                        // and (we hope) iterates rather than re-submitting the
                        // same broken draft.
                        let mut steps = trajectory.steps;
                        let next_idx = steps.len() as u32;
                        steps.push(synthetic_compile_step(next_idx, &action.path, &diag));
                        prior_steps = steps;
                        retries_remaining -= 1;
                        continue;
                    }
                    Err(diag) => {
                        tracing::warn!(
                            role = %role,
                            path = %success_artifact,
                            diagnostics = %diag.chars().take(400).collect::<String>(),
                            "drafter_trajectory: pre-twin compile failed (retry budget exhausted); abstaining"
                        );
                        return Ok(String::new());
                    }
                }
            }
            // Non-terminal halts → abstain. The drafter's circuit-breaker
            // promotes to stub or operator escalation as it does today for
            // any empty/abstained content.
            _ => return Ok(String::new()),
        }
    }
}

/// Synthetic step injected after a pre-twin compile failure (P4 + P5).
/// The persona sees this as if it had called `cargo_check` itself, so
/// next turn it can react to the diagnostic and revise.
fn synthetic_compile_step(step_idx: u32, path: &str, diag: &str) -> AgentStep {
    let summary = diag.lines().take(20).collect::<Vec<_>>().join("\n");
    AgentStep {
        step_idx,
        thought: "(synthetic: pre-twin compile gate fired)".to_string(),
        tool: "cargo_check".to_string(),
        args_json: serde_json::json!({"target_path": path}).to_string(),
        observation: format!(
            "error: pre-twin rustc check failed on {path}. Revise the file and call code_patch_propose again. Diagnostics:\n{summary}",
            path = path
        ),
        bytes: summary.len(),
        truncated: diag.len() > summary.len(),
        output_tokens: 0,
        input_tokens: 0,
        latency_ms: 0,
    }
}

/// Pre-twin `rustc` check on a draft's content.
///
/// Writes the content to a unique tempfile, runs
/// `rustc --emit=metadata --crate-type rlib --error-format=short` and
/// reports diagnostics on failure. Catches syntactic / type-level errors
/// + the markdown-fence-leaking-into-code class — does NOT catch
/// crate-level dependency errors (e.g. `use hex_cli::Foo` when the
/// crate isn't linked). The latter requires a real `cargo check`, which
/// lands in P4.2 once an in-place scratch-tree approach proves stable.
///
/// Non-Rust paths short-circuit to Ok — we don't gate them today.
async fn precompile_check(path: &str, content: &str) -> Result<(), String> {
    if !path.ends_with(".rs") {
        return Ok(());
    }
    let basename = path.rsplit('/').next().unwrap_or("draft.rs");

    // Module-tree leaf files declare other modules with `pub mod X;`.
    // Standalone `rustc --test` can't find those sibling files because
    // we check in /tmp away from the workspace. The gate rejects on
    // E0583 ("file not found for module") for every submodule the leaf
    // references — irrelevant to the actual patch. Skip the gate for
    // these files; the workspace cargo check the executor runs post-write
    // is the real verification. Observed 2026-05-27 on the autonomous
    // wire-in of twin_deterministic into orchestration/mod.rs (Finding 7).
    if matches!(basename, "mod.rs" | "lib.rs" | "main.rs") {
        tracing::info!(
            path = %path,
            "precompile_check: skipping module-tree leaf (cargo-context required)"
        );
        return Ok(());
    }

    // Collision-free name: PID + process-local atomic counter. The prior
    // microsecond-only suffix collided under parallel tokio tests because
    // two threads grabbed the same μs slice. tempfile would be ideal but
    // is a dev-dep here; keeping precompile_check usable from non-test
    // code is more valuable than a few lines saved.
    let pid = std::process::id();
    let counter = PRECOMPILE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let work_dir = std::env::temp_dir().join(format!("hex-agent-loop-pc-{}-{}", pid, counter));
    if let Err(e) = tokio::fs::create_dir_all(&work_dir).await {
        return Err(format!("could not create precompile workdir: {}", e));
    }
    let tmp_path = work_dir.join(basename);
    if let Err(e) = tokio::fs::write(&tmp_path, content).await {
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
        return Err(format!("could not write precompile tempfile: {}", e));
    }

    // ~/.cargo/bin on PATH so rustc is reachable from non-login shells
    // (same fix the run.sh harness needed).
    let path_env = {
        let cargo_bin = std::env::var("HOME")
            .ok()
            .map(|h| format!("{}/.cargo/bin", h))
            .unwrap_or_default();
        let existing = std::env::var("PATH").unwrap_or_default();
        if cargo_bin.is_empty() || existing.split(':').any(|seg| seg == cargo_bin) {
            existing
        } else {
            format!("{}:{}", cargo_bin, existing)
        }
    };

    let out_path = work_dir.join("draft.rmeta");

    // Use `--test` so #[test] functions are reachable + borrow-checked.
    // Without --test, rustc treats #[test] fns as dead code in --crate-type
    // rlib mode and silently skips the body — observed 2026-05-27 when a
    // standalone_gate.rs draft with `Ok(addrs) => addrs.any(...)` (a real
    // borrow error) passed the gate but failed `cargo test` immediately.
    // --test implies --crate-type bin so we drop the rlib override.
    //
    // CARGO_MANIFEST_DIR is set by cargo at compile-time; raw rustc has no
    // such env, so file content like `env!("CARGO_MANIFEST_DIR")` will fail
    // here. Stub it to a placeholder so the gate doesn't false-fail
    // legitimate test files that need the macro for path resolution.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new("rustc")
            .env("PATH", &path_env)
            .env("CARGO_MANIFEST_DIR", "/precompile-gate-placeholder")
            .arg("--edition").arg("2021")
            .arg("--test")
            .arg("--emit").arg("metadata")
            .arg("--error-format").arg("short")
            .arg("-o").arg(&out_path)
            .arg(&tmp_path)
            .output(),
    )
    .await;

    // Best-effort cleanup of the per-call workdir; ignore errors.
    let _ = tokio::fs::remove_dir_all(&work_dir).await;

    match result {
        Err(_) => Err("rustc precompile timed out after 30s".to_string()),
        Ok(Err(e)) => Err(format!("rustc spawn failed: {} (PATH={})", e, path_env)),
        Ok(Ok(o)) if o.status.success() => Ok(()),
        Ok(Ok(o)) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            let combined = if stderr.is_empty() { stdout.to_string() } else { stderr.to_string() };
            // Treat "can't find crate" / "use of unresolved crate" as
            // recoverable for v1 — the single-file rustc doesn't have
            // the crate graph; those errors are false positives. Full
            // cargo check lands in a follow-up phase.
            if is_crate_resolution_only(&combined) {
                Ok(())
            } else {
                Err(combined.chars().take(2000).collect())
            }
        }
    }
}

/// Heuristic: when every error line is a "can't find crate X" / "unresolved
/// import" we're seeing the crate-graph-not-linked artifact of running
/// `rustc` on a single file. Treat as pass for v1 so we don't false-fail
/// drafts that reference `hex_core` / sibling crates. P4.2 (cargo check
/// in scratch tree) will replace this when it lands.
fn is_crate_resolution_only(diag: &str) -> bool {
    let lines: Vec<&str> = diag
        .lines()
        .filter(|l| {
            let lower = l.trim().to_ascii_lowercase();
            lower.starts_with("error") || lower.starts_with("warning")
        })
        .collect();
    if lines.is_empty() {
        return false;
    }
    lines.iter().all(|l| {
        let lower = l.to_ascii_lowercase();
        lower.contains("can't find crate")
            || lower.contains("unresolved import")
            || lower.contains("failed to resolve")
    })
}

fn render_task_brief(role: &str, path: &str, action: &str, ceo_ask: &str) -> String {
    let mut brief = format!(
        "Produce the contents of `{path}` per your commitment.\n\n\
         COMMITMENT (your earlier reply): {action}\n"
    );
    if !ceo_ask.is_empty() {
        brief.push_str(&format!(
            "\nORIGINATING OPERATOR ASK (the request that triggered this commitment):\n{}\n",
            ceo_ask.chars().take(2000).collect::<String>()
        ));
    }
    brief.push_str(&format!(
        "\nYou are `{role}`. Use repo_read / repo_grep to ground your draft in the \
         real codebase BEFORE you submit. Use cargo_check (when the target is a \
         Rust source/test file) to verify your draft compiles. Finish with one \
         code_patch_propose call whose path={path} and whose content is the FULL \
         file body (mode=create) or a precise replacement (mode=replace_string / \
         replace_lines for edits)."
    ));
    brief
}

/// Query STDB for any rejected proposed_actions tied to this commitment.
/// Convert each rejection's `twin_rationale` into a synthetic
/// `twin_review` AgentStep so the next agent_loop run starts with the
/// reviewer's reasons in its context (wp-sop-agent-loop P5).
///
/// Returns an empty Vec when there are no prior rejections, when the
/// query fails (best-effort — STDB outage MUST NOT block dispatch), or
/// when `commitment_id == 0` (commitment was never persisted).
async fn fetch_prior_twin_rejections(
    http: &reqwest::Client,
    commitment_id: u64,
) -> Result<Vec<AgentStep>, String> {
    if commitment_id == 0 {
        return Ok(Vec::new());
    }
    let stdb_host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string());
    let hex_db = std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string());
    let url = format!("{}/v1/database/{}/sql", stdb_host, hex_db);
    // SQL is parameterized via string interpolation against a u64 we
    // generated — no injection surface, but still constrain to a tight
    // shape (id ASC, LIMIT 8) to bound prompt-token cost.
    let query = format!(
        "SELECT id, twin_verdict, twin_rationale FROM proposed_action \
         WHERE related_commitment_id = {} AND status = 'rejected' \
         LIMIT 8",
        commitment_id
    );
    let resp = http
        .post(&url)
        .body(query)
        .send()
        .await
        .map_err(|e| format!("sql http: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "sql HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    let body = resp.text().await.map_err(|e| format!("sql body: {}", e))?;
    // STDB SQL returns [{"schema":..., "rows":[[col1, col2, ...]]}].
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("sql parse: {}", e))?;
    let rows = parsed
        .get(0)
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let Some(cols) = row.as_array() else { continue };
        let id = cols.get(0).and_then(|v| v.as_u64()).unwrap_or(0);
        let verdict = cols
            .get(1)
            .and_then(|v| v.as_str())
            .unwrap_or("reject")
            .to_string();
        let rationale = cols
            .get(2)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        out.push(synthetic_twin_step(i as u32, id, &verdict, &rationale));
    }
    Ok(out)
}

/// Build a synthetic AgentStep representing a prior twin verdict. The
/// persona's next turn sees this as if it had emitted a tool call and
/// gotten back the reviewer's verdict — same shape as a real step so
/// the prompt-rendering code doesn't need a special case.
fn synthetic_twin_step(step_idx: u32, action_id: u64, verdict: &str, rationale: &str) -> AgentStep {
    AgentStep {
        step_idx,
        thought: "(synthetic: prior twin verdict — revise your next draft to address this)"
            .to_string(),
        tool: "twin_review".to_string(),
        args_json: serde_json::json!({ "proposed_action_id": action_id }).to_string(),
        observation: format!(
            "PRIOR TWIN VERDICT: {} — {}",
            verdict.to_uppercase(),
            rationale
        ),
        bytes: rationale.len(),
        truncated: false,
        output_tokens: 0,
        input_tokens: 0,
        latency_ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_brief_includes_path_and_role() {
        let brief = render_task_brief(
            "hex-coder",
            "hex-cli/tests/foo.rs",
            "I will create hex-cli/tests/foo.rs",
            "Please create the test.",
        );
        assert!(brief.contains("hex-cli/tests/foo.rs"));
        assert!(brief.contains("hex-coder"));
        assert!(brief.contains("Please create the test."));
        assert!(brief.contains("code_patch_propose"));
    }

    #[test]
    fn task_brief_truncates_long_ceo_ask() {
        let long_ask = "x".repeat(5000);
        let brief = render_task_brief("hex-coder", "docs/x.md", "act", &long_ask);
        // 2000 char cap + the framing text; total well under 5k.
        assert!(brief.len() < 4500);
    }

    #[test]
    fn task_brief_handles_empty_ceo_ask() {
        let brief = render_task_brief("hex-coder", "docs/x.md", "act", "");
        assert!(!brief.contains("ORIGINATING OPERATOR ASK"));
    }

    // ---- precompile_check ----

    #[tokio::test]
    async fn precompile_skips_non_rust_paths() {
        let res = precompile_check("docs/spec.md", "not rust").await;
        assert!(res.is_ok(), "expected Ok for non-.rs path");
    }

    #[tokio::test]
    async fn precompile_accepts_valid_rust() {
        // Requires rustc on PATH. Pre-test guard: skip if missing so the
        // unit-test run doesn't false-fail on a container without rustup.
        if which_or_skip("rustc").is_none() {
            eprintln!("SKIP: rustc not on PATH");
            return;
        }
        let res = precompile_check(
            "hex-cli/tests/dummy.rs",
            "#[test]\nfn ok() { assert_eq!(1, 1); }\n",
        )
        .await;
        assert!(res.is_ok(), "expected Ok for valid Rust, got {:?}", res);
    }

    #[tokio::test]
    async fn precompile_rejects_markdown_fenced_garbage() {
        if which_or_skip("rustc").is_none() {
            eprintln!("SKIP: rustc not on PATH");
            return;
        }
        let res = precompile_check(
            "hex-cli/tests/dummy.rs",
            "```rust\nthis is not valid syntax {{ ! @#\n```\n",
        )
        .await;
        assert!(res.is_err(), "expected Err for garbage Rust");
    }

    #[test]
    fn crate_resolution_only_identifies_unresolved_imports() {
        let diag = "error[E0463]: can't find crate for `hex_core`\nerror[E0432]: unresolved import `serde`";
        assert!(is_crate_resolution_only(diag));
    }

    #[test]
    fn crate_resolution_only_returns_false_for_syntax_errors() {
        let diag = "error: expected `;`, found `}`\nerror[E0432]: unresolved import `serde`";
        assert!(!is_crate_resolution_only(diag));
    }

    #[test]
    fn crate_resolution_only_returns_false_when_no_errors_found() {
        assert!(!is_crate_resolution_only("nothing relevant"));
    }

    #[test]
    fn synthetic_twin_step_renders_verdict_and_rationale() {
        let step = synthetic_twin_step(0, 42, "reject", "off-topic: path not in scope");
        assert_eq!(step.tool, "twin_review");
        assert_eq!(step.step_idx, 0);
        assert!(step.observation.contains("REJECT"));
        assert!(step.observation.contains("off-topic: path not in scope"));
        // args_json should reference the action id so the audit trail
        // can connect the synthetic step back to the original verdict.
        assert!(step.args_json.contains("42"));
        // No tokens or latency for synthetic steps — they're free.
        assert_eq!(step.input_tokens, 0);
        assert_eq!(step.output_tokens, 0);
        assert_eq!(step.latency_ms, 0);
    }

    #[tokio::test]
    async fn fetch_prior_twin_rejections_skips_zero_commitment_id() {
        let http = reqwest::Client::new();
        let result = fetch_prior_twin_rejections(&http, 0).await.unwrap();
        assert!(result.is_empty(), "commitment_id=0 should short-circuit to empty");
    }

    #[test]
    fn synthetic_compile_step_carries_diagnostics() {
        let step = synthetic_compile_step(3, "hex-cli/tests/foo.rs", "error: expected `;`");
        assert_eq!(step.step_idx, 3);
        assert_eq!(step.tool, "cargo_check");
        assert!(step.observation.contains("error: expected"));
        assert!(step.observation.contains("hex-cli/tests/foo.rs"));
        assert_eq!(step.input_tokens, 0);
        assert_eq!(step.output_tokens, 0);
    }

    fn which_or_skip(bin: &str) -> Option<()> {
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(':') {
                if std::path::Path::new(dir).join(bin).is_file() {
                    return Some(());
                }
            }
        }
        // Also check ~/.cargo/bin in case PATH doesn't include it but it
        // exists (matches what precompile_check itself does).
        if let Ok(home) = std::env::var("HOME") {
            if std::path::Path::new(&home).join(".cargo/bin").join(bin).is_file() {
                return Some(());
            }
        }
        None
    }
}
