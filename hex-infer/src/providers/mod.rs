//! Concrete `IInferencePort` adapters.
//!
//! Each submodule speaks to exactly one backend and implements
//! [`hex_core::ports::inference::IInferencePort`]. Nothing outside this
//! module tree may name a provider — that is founding goal G1, enforced
//! at the crate boundary (see the crate docs).
//!
//! - [`ollama`] — HTTP-backed, the primary local provider.
//! - [`claude_code`] — subprocess-backed (`claude -p`), the frontier fallback.

pub mod claude_code;
pub mod ollama;

pub use claude_code::ClaudeCodeInferenceAdapter;
pub use ollama::OllamaInferenceAdapter;
