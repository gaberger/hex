//! Agent loop with tool use (wp-sop-agent-loop, ADR-2026-05-27-1330).
//!
//! Converts the SOP "accept" path's content-generation step from one-shot
//! persona inference to a ReAct loop. The persona iterates: read context →
//! reason → invoke a tool → observe → repeat, until it emits a terminal
//! action (`code_patch_propose`).
//!
//! Motivation: today's dogfooding session
//! (docs/analysis/standalone-ollama-proof-2026-05-27.md) wired the OUTER
//! SOP shell (classifier → typed commit → drafter → twin → executor) but
//! the INNER content step is one-shot LLM with no ability to read the
//! target file or verify compilation. Local AI (qwen2.5-coder:14b) emits
//! plausible-looking code with fabricated imports because nobody let it
//! look at the surrounding repo first. This module gives it tools.
//!
//! Phase layout (see docs/workplans/wp-sop-agent-loop.json):
//! - P1 (this module): tool surface — `IAgentTool` + 4 concrete tools.
//! - P2: ReAct driver — `driver::run()`.
//! - P3: drafter swap — bridge from `drafter::draft_one` into the loop.
//! - P4: compile gate before twin.
//! - P5: rejection feedback into the trajectory.
//! - P6: STDB observability (agent_trajectory + agent_step tables).
//! - P7: acceptance test against today's P7.2 brief.

pub mod driver;
pub mod policy;
pub mod tool;
pub mod tools;
pub mod trajectory;

pub use driver::{run, AgentRunInput};
pub use tool::{IAgentTool, Observation, TerminalAction, ToolError};
pub use tools::{CargoCheckTool, CodePatchProposeTool, RepoGrepTool, RepoReadTool};
pub use trajectory::{AgentStep, TerminatedReason, Trajectory};
