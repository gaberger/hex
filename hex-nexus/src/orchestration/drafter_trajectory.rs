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
) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http build: {}", e))?;
    let shim = Arc::new(HttpInferenceShim::new(http, inference_url.to_string()));

    let task_brief = render_task_brief(role, success_artifact, action_text, ceo_ask);

    // Run the agent loop, then pre-twin compile-gate the result. If
    // compile fails AND we still have a retry budget, append the
    // diagnostics as a synthetic step + re-run the loop with the
    // updated context so the persona sees the actual error.
    let mut prior_steps: Vec<AgentStep> = Vec::new();
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
    let tmp_dir = std::env::temp_dir();
    let basename = path.rsplit('/').next().unwrap_or("draft.rs");
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    let tmp_path = tmp_dir.join(format!("hex-agent-loop-precompile-{}-{}", unique, basename));
    if let Err(e) = tokio::fs::write(&tmp_path, content).await {
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

    let out_dir = tmp_dir.join(format!("hex-agent-loop-precompile-out-{}", unique));
    let _ = tokio::fs::create_dir_all(&out_dir).await;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new("rustc")
            .env("PATH", &path_env)
            .arg("--edition").arg("2021")
            .arg("--emit").arg("metadata")
            .arg("--crate-type").arg("rlib")
            .arg("--error-format").arg("short")
            .arg("-o").arg(out_dir.join("draft.rmeta"))
            .arg(&tmp_path)
            .output(),
    )
    .await;

    // Best-effort cleanup; ignore errors.
    let _ = tokio::fs::remove_file(&tmp_path).await;
    let _ = tokio::fs::remove_dir_all(&out_dir).await;

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
