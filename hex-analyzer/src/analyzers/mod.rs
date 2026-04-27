//! Detector modules. Each one is independently testable; `main.rs`
//! routes a single CLI invocation to one or more of them and merges
//! findings into a shared `{findings: [...]}` envelope.

pub mod cohesion;
pub mod composition_churn;
pub mod dead_layer;
pub mod duplication;
pub mod god_types;
pub mod orphan;
