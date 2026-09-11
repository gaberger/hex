//! Re-export shim for the inference adapters, which now live in `hex-infer`.
//!
//! Per ADR-2608241500 the provider adapters moved out of the daemon into the
//! `hex-infer` library crate so `hex-exec` can call them in-process. This
//! module stays only so existing `crate::adapters::inference::*` paths inside
//! hex-nexus keep resolving; it is removed with the daemon in workplan P5.2.

pub use hex_infer::providers::{claude_code, ollama};
pub use hex_infer::{ClaudeCodeInferenceAdapter, OllamaInferenceAdapter};
