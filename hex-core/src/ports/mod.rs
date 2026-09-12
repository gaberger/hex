// `inference` is a module directory: `inference.rs` declares the trait
// surface and `inference/mock.rs` ships `MockInferencePort` for downstream
// test code (ADR-2026-04-11-2000 P1.2 / P2 / P5).
pub mod inference;
// State port contract (IStatePort + focused sub-traits + DTOs). Relocated from
// hex-nexus where it was an anomaly — port traits belong in hex-core with the
// rest (ADR-2606071340 P1). Implemented by the STDB/SQLite adapters.
