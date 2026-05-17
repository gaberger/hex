pub mod adapter_generator;
pub mod agent_comm;
pub mod agent_runtime;
pub mod brain;
pub mod build;
pub mod consolidation_memory;
pub mod context_compressor;
pub mod coordination;
pub mod enforcement;
pub mod experiment;
pub mod file_system;
pub mod file_writer;
// `inference` is a module directory: `inference.rs` declares the trait
// surface and `inference/mock.rs` ships `MockInferencePort` for downstream
// test code (ADR-2604112000 P1.2 / P2 / P5).
pub mod inference;
pub mod sandbox;
pub mod secret;
pub mod validator;
pub mod web;
