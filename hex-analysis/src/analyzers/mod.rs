//! Architectural-health detectors, each independently testable.
//!
//! Folded in from the `hex-analyzer` crate (ADR-2608241500 P6.5), whose
//! binary existed to feed the improver daemon. Rule identity and severity are
//! unchanged — S03 is the regression check.

pub mod cohesion;
pub mod composition_churn;
pub mod dead_layer;
pub mod duplication;
pub mod god_types;
pub mod orphan;
