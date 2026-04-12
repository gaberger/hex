pub mod agent_runtime;
pub mod brain;
pub mod build;
pub mod context_compressor;
pub mod coordination;
pub mod enforcement;
pub mod file_system;
// `inference` is a module directory: `inference.rs` declares the trait
// surface and `inference/mock.rs` ships `MockInferencePort` for downstream
// test code (ADR-2604112000 P1.2 / P2 / P5).
pub mod inference;
pub mod sandbox;
pub mod secret;
