//! Architecture analysis module — hexagonal boundary validation, dead export
//! detection, circular dependency detection, and health scoring.
//!
//! Phase 1 (ADR-034): domain types, port traits, layer classifier, path normalizer.
//! Phase 2 (ADR-034): native tree-sitter adapter for import/export extraction.
//! Phase 3 (ADR-034): analysis use cases — boundary checker, cycle detector, dead exports, analyzer.

pub mod domain;
pub mod ports;
pub mod layer_classifier;
pub mod path_normalizer;
pub mod treesitter_adapter;
pub mod boundary_checker;
pub mod cycle_detector;
pub mod dead_export_finder;
pub mod analyzer;
pub mod frontend_checker;
// Architectural-health detectors, folded in from the hex-analyzer crate
// (ADR-2608241500 P6.5). One analysis crate, one entry point — the separate
// binary existed for the improver daemon, which is deleted.
pub mod analyzers;
pub mod fingerprint_extractor;
