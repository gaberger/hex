//! Research analysts for the idle-research swarm (ADR-2604151200).
//!
//! Each submodule implements one deterministic, code-first analyst that
//! shells out to existing tooling, parses its output, and emits
//! [`hex_core::Finding`] records.

pub mod architecture_analyst;
pub mod code_quality_analyst;
pub mod coordinator;
pub mod draft_writer;
pub mod drift_analyst;
pub mod naming_analyst;
pub mod performance_analyst;
pub mod ux_analyst;
