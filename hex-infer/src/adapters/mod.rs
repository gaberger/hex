//! Concrete `IInferencePort` adapters.
//!
//! Moved out of `hex-nexus` by Phase 1 of the solo refactor. They never depended on the daemon —
//! `grep crate:: ` over both files found nothing but a doc comment — so the only thing binding
//! inference to a control plane was the crate they happened to live in.

pub mod claude_code;
pub mod ollama;

pub use claude_code::ClaudeCodeInferenceAdapter;
pub use ollama::OllamaInferenceAdapter;
