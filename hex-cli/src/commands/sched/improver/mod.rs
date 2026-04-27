//! Self-improvement loop (ADR-2604271100).
//!
//! Pipeline: [`discover`] → variant generation (P2) → judge (P3) → act (P4),
//! tied together by a sched tick (P5). This module hosts the discovery
//! surface; later phases live in `hex-nexus/src/orchestration/`.

pub mod discover;

pub use discover::{discover, discover_with, load_detectors, Detector, Hypothesis, Severity, Source};
