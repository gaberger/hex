//! `hex-infer` — the inference tier of hex, as a library.
//!
//! Per ADR-2608241500 (collapse hex to a solo software-engineering agent),
//! inference no longer lives behind an HTTP daemon. Every provider adapter,
//! the tier router, best-of-N with its compile gate, and the frontier
//! fallback are assembled here as plain Rust that `hex-exec` calls
//! in-process. There is no axum handler, no `AppState`, and no loopback hop.
//!
//! # Founding goal G1 — model independence
//!
//! `hex-infer` is the **single enforcement point** for G1: no consumer
//! outside this crate and `hex_core::ports::inference` may name a concrete
//! provider. Callers depend on
//! [`hex_core::ports::inference::IInferencePort`]; the choice of Ollama,
//! an OpenAI-compatible endpoint, Anthropic, or the `claude -p` subprocess
//! is resolved here from `.hex/project.json`.
//!
//! # Layout
//!
//! - [`providers`] — concrete `IInferencePort` adapters, one module each:
//!   Ollama, any OpenAI-compatible endpoint, Anthropic, and `claude -p`.
//!
//! Tier routing, best-of-N, and the config loader land in this crate in
//! workplan tasks P2.4 and P2.5.

pub mod complete;
pub mod config;
pub mod endpoint;
pub mod providers;
pub mod routing;
pub mod transport;

pub use complete::{complete, CompleteError, CompleteRequest, Completion};
pub use config::InferenceConfig;
pub use endpoint::Endpoint;
pub use providers::{
    AnthropicAdapter, ClaudeCodeInferenceAdapter, OllamaInferenceAdapter, OpenAiCompatAdapter,
};
