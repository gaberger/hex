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
use crate::orchestration::agent_loop::trajectory::TerminatedReason;
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

    let tools: Vec<Box<dyn IAgentTool>> = vec![
        Box::new(RepoReadTool::new(repo_root.to_path_buf())),
        Box::new(RepoGrepTool::new(repo_root.to_path_buf())),
        Box::new(CargoCheckTool::new(repo_root.to_path_buf())),
        Box::new(CodePatchProposeTool::new()),
    ];

    let task_brief = render_task_brief(role, success_artifact, action_text, ceo_ask);

    let trajectory = run(AgentRunInput {
        role,
        task_brief: &task_brief,
        tools,
        max_steps: MAX_STEPS,
        max_output_tokens: MAX_OUTPUT_TOKENS,
        inference: shim,
        model: model.to_string(),
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
        "drafter_trajectory: agent_loop run complete"
    );

    match trajectory.terminated_reason {
        TerminatedReason::TerminalAction => {
            let content = trajectory
                .final_action
                .map(|a| a.content)
                .unwrap_or_default();
            Ok(content)
        }
        // Non-terminal halts → abstain. The drafter's circuit-breaker
        // promotes to stub or operator escalation as it does today for
        // any empty/abstained content.
        _ => Ok(String::new()),
    }
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
}
