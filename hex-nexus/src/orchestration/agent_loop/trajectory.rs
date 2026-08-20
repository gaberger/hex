//! Trajectory + AgentStep — what an agent-loop run produces
//! (wp-sop-agent-loop P2.1).
//!
//! The driver returns one of these per `run()` call. The drafter (P3)
//! wraps `final_action` into a `proposed_action_open` call. The
//! observability layer (P6) writes one `agent_trajectory` row + N
//! `agent_step` rows per Trajectory.

use serde::{Deserialize, Serialize};

use crate::orchestration::agent_loop::tool::TerminalAction;

/// Why a trajectory stopped. Either we reached a successful terminal
/// action, or we hit a budget / contract guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminatedReason {
    /// Persona invoked `code_patch_propose` (or any future terminal
    /// tool). `final_action` is populated.
    TerminalAction,
    /// Step count budget hit before a terminal action.
    MaxSteps,
    /// Cumulative token budget hit before a terminal action.
    TokenBudget,
    /// Persona emitted N consecutive parse-unrecoverable responses
    /// (off-contract JSON, missing required fields). We give up and let
    /// the drafter abandon the commitment.
    ParseExhausted,
    /// Persona requested a tool name we don't have. Caught at the
    /// driver, so a typo doesn't loop forever.
    UnknownTool { name: String },
    /// Inference call itself failed (network, timeout, provider 404).
    /// Carries the first 200 chars of the underlying error.
    InferenceFailed { error: String },
}

/// One iteration of the ReAct loop. The persona produces a `thought`
/// (free-text reasoning) + an `action` (tool + args). The driver
/// dispatches the tool and records its `observation`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStep {
    pub step_idx: u32,
    pub thought: String,
    pub tool: String,
    /// JSON-serialised args as the persona emitted them (so the
    /// observability layer can replay decisions). Empty object if the
    /// persona omitted args.
    pub args_json: String,
    /// What the tool returned. For tool errors, this is the error's
    /// Display form prefixed with `error: `.
    pub observation: String,
    pub bytes: usize,
    pub truncated: bool,
    /// Output tokens consumed by this step's inference call. Cumulative
    /// across the trajectory is `Trajectory::total_tokens`.
    pub output_tokens: u64,
    pub input_tokens: u64,
    pub latency_ms: u64,
}

/// One complete agent-loop run, end to end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    /// Set by the driver from a UUID when STDB observability is on,
    /// otherwise the empty string. Persisted as the `agent_trajectory.id`
    /// in P6.
    pub id: String,
    pub role: String,
    pub task_brief: String,
    pub steps: Vec<AgentStep>,
    /// Populated iff `terminated_reason == TerminalAction`.
    pub final_action: Option<TerminalAction>,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
    pub total_latency_ms: u64,
    pub terminated_reason: TerminatedReason,
}

impl Trajectory {
    pub fn new(role: impl Into<String>, task_brief: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            role: role.into(),
            task_brief: task_brief.into(),
            steps: Vec::new(),
            final_action: None,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cost_usd: 0.0,
            total_latency_ms: 0,
            terminated_reason: TerminatedReason::MaxSteps, // overwritten on every real termination
        }
    }

    /// Total tokens (input + output) used across all steps. Driver compares
    /// this against the token budget to decide whether to keep iterating.
    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens + self.total_output_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_trajectory_is_empty() {
        let t = Trajectory::new("hex-coder", "do the thing");
        assert_eq!(t.role, "hex-coder");
        assert_eq!(t.task_brief, "do the thing");
        assert!(t.steps.is_empty());
        assert!(t.final_action.is_none());
        assert_eq!(t.total_tokens(), 0);
    }

    #[test]
    fn trajectory_round_trips_through_serde() {
        let mut t = Trajectory::new("hex-coder", "draft a test");
        t.steps.push(AgentStep {
            step_idx: 0,
            thought: "Read the harness first".into(),
            tool: "repo_read".into(),
            args_json: r#"{"path":"examples/standalone-pipeline-test/run.sh"}"#.into(),
            observation: "#!/usr/bin/env bash\n...".into(),
            bytes: 12345,
            truncated: false,
            output_tokens: 42,
            input_tokens: 800,
            latency_ms: 1500,
        });
        t.total_input_tokens = 800;
        t.total_output_tokens = 42;
        t.total_latency_ms = 1500;
        t.terminated_reason = TerminatedReason::TerminalAction;
        t.final_action = Some(TerminalAction {
            tool: "code_patch_propose".into(),
            path: "hex-cli/tests/foo.rs".into(),
            mode: "create".into(),
            content: "fn main(){}\n".into(),
        });

        let json = serde_json::to_string(&t).unwrap();
        let back: Trajectory = serde_json::from_str(&json).unwrap();
        assert_eq!(back.steps.len(), 1);
        assert_eq!(back.steps[0].tool, "repo_read");
        assert_eq!(back.terminated_reason, TerminatedReason::TerminalAction);
        assert_eq!(back.total_tokens(), 842);
        assert!(back.final_action.is_some());
    }

    #[test]
    fn terminated_reason_round_trips() {
        for r in [
            TerminatedReason::TerminalAction,
            TerminatedReason::MaxSteps,
            TerminatedReason::TokenBudget,
            TerminatedReason::ParseExhausted,
            TerminatedReason::UnknownTool { name: "nope".into() },
            TerminatedReason::InferenceFailed { error: "x".into() },
        ] {
            let j = serde_json::to_string(&r).unwrap();
            let back: TerminatedReason = serde_json::from_str(&j).unwrap();
            assert_eq!(back, r);
        }
    }
}
