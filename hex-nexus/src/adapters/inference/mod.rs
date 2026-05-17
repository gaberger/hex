//! Concrete `IInferencePort` adapters for hex-nexus.
//!
//! Ships with two reference implementations today: [`ollama`] (HTTP-backed,
//! the primary standalone provider) and [`claude_code`] (subprocess-backed,
//! the fallback standalone provider) — both wired into
//! [`crate::composition::standalone`] per ADR-2604112000. Future providers
//! (vLLM, OpenAI-compatible) will live alongside them as new submodules
//! implementing the same `hex_core::ports::inference::IInferencePort`
//! trait.

pub mod claude_code;
pub mod ollama;

pub use claude_code::ClaudeCodeInferenceAdapter;
pub use ollama::OllamaInferenceAdapter;
