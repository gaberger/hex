//! Inference for hex, with no daemon in the path.
//!
//! `hex do` reached its model through an HTTP call to a locally-running nexus, which then called
//! the provider. The agent loop therefore could not run unless a control plane was up — the daemon
//! mediated a call it added nothing to. This crate is that call, as a library.
//!
//! Everything here speaks `hex_core::ports::inference::IInferencePort`. G1 requires that no
//! consumer names a provider, and this is the enforcement point: callers hold the trait, and which
//! adapter is behind it is a composition decision.

pub mod adapters;
pub mod complete;
pub mod spend;

pub use adapters::{ClaudeCodeInferenceAdapter, OllamaInferenceAdapter};
pub use complete::{complete_raw, complete_text};
