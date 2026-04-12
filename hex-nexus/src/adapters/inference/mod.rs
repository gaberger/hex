//! Concrete `IInferencePort` adapters for hex-nexus.
//!
//! Ships with one reference implementation today: [`ollama`], the standalone
//! provider wired into [`crate::composition::standalone`] per ADR-2604112000.
//! Future providers (vLLM, OpenAI-compatible) will live alongside it as new
//! submodules implementing the same `hex_core::ports::inference::IInferencePort`
//! trait.

pub mod ollama;

pub use ollama::OllamaInferenceAdapter;
