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
pub mod adr_compliance;
