//! Pipeline utilities for `hex dev` — per-phase model selection and RL integration.

pub mod adr_phase;
pub mod budget;
pub mod code_phase;
pub mod dev_mode;
pub mod fix_agent;
pub mod model_selection;
pub mod quality_agent;
pub mod swarm_phase;
pub mod validate_phase;
pub mod workplan_phase;

pub use adr_phase::{AdrPhase, AdrPhaseResult};
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
pub use workplan_phase::{WorkplanPhase, WorkplanPhaseResult};
