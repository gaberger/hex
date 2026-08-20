//! hex-analyzer — architectural-health detectors.
//!
//! Each detector lives under [`analyzers`] and emits a JSON `{findings: [...]}`
//! envelope the improver (P2/P3 of wp-architectural-health-detectors) can parse.

pub mod analyzers;
