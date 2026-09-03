//! Re-export of the `IInferencePort` adapters, which now live in `hex-infer`.
//!
//! Phase 1 of the solo refactor moved `ollama.rs` and `claude_code.rs` out of this crate. Neither
//! had a single `crate::` reference — the only thing binding inference to the daemon was the crate
//! they happened to sit in.
//!
//! This shim keeps nexus compiling while the daemon tier is still present. It goes with the crate
//! at Phase 3.

pub use hex_infer::adapters::{claude_code, ollama};
pub use hex_infer::{ClaudeCodeInferenceAdapter, OllamaInferenceAdapter};
