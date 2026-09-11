//! Concrete `IInferencePort` adapters.
//!
//! Each submodule speaks to exactly one backend and implements
//! [`hex_core::ports::inference::IInferencePort`]. Nothing outside this
//! module tree may name a provider — that is founding goal G1, enforced
//! at the crate boundary (see the crate docs).
//!
//! - [`ollama`] — HTTP-backed, the primary local provider. Real NDJSON streaming.
//! - [`openai_compat`] — any OpenAI-compatible endpoint: MiniMax, vLLM,
//!   OpenRouter, Groq, Together, Ollama's `/v1` shim.
//! - [`anthropic`] — the Messages API, with prompt caching and extended thinking.
//! - [`claude_code`] — subprocess-backed (`claude -p`), the frontier fallback.

pub mod anthropic;
pub mod claude_code;
pub mod ollama;
pub mod openai_compat;
mod vec_stream;

pub use anthropic::AnthropicAdapter;
pub use claude_code::ClaudeCodeInferenceAdapter;
pub use ollama::OllamaInferenceAdapter;
pub use openai_compat::OpenAiCompatAdapter;
