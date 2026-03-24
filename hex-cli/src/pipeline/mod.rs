//! Pipeline utilities for `hex dev` — per-phase model selection and RL integration.

pub mod adr_phase;
pub mod agents;
pub mod budget;
pub mod cli_runner;
pub mod code_phase;
pub mod dev_mode;
pub mod fix_agent;
pub mod model_selection;
pub mod objectives;
pub mod quality_agent;
pub mod supervisor;
pub mod swarm_phase;
pub mod validate_phase;
pub mod workplan_phase;

pub use cli_runner::CliRunner;
pub use adr_phase::{AdrPhase, AdrPhaseResult};
pub use agents::{
    DocResult, DocumenterAgent, ReviewerAgent, ReviewIssue, ReviewResult, TestAgentResult,
    TesterAgent, UxIssue, UxReviewResult, UxReviewerAgent,
};
pub use code_phase::{generate_scaffold, CodePhase, CodeStepResult};
pub use dev_mode::{DevConfig, DevMode};
pub use fix_agent::{FixAgent, FixTaskInput, FixTaskOutput};
pub use model_selection::ModelSelector;
pub use quality_agent::{GateTaskInput, GateTaskOutput, QualityGateAgent};
pub use swarm_phase::{SwarmPhase, SwarmPhaseResult};
pub use validate_phase::{
    AnalyzeResult, CompileError, CompileResult, ProposedFix, QualityGateResult, TestResult,
    ValidatePhase, ValidateResult,
};
pub use objectives::{
    agent_for_objective, can_evaluate, dependencies, objectives_for_tier, parallelizable,
    Objective, ObjectiveState,
};
pub use supervisor::{AgentContext, Supervisor, SupervisorResult, TierResult};
pub use workplan_phase::{WorkplanPhase, WorkplanPhaseResult};
