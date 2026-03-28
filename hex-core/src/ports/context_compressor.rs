//! Context compressor port — contract for prompt compression (ADR-2603281000 P3).
//!
//! Implementors reduce accumulated tool output to fit within a token budget
//! while preserving code blocks and error messages verbatim.

/// A port for compressing accumulated tool output before it is appended to
/// an agent's conversation history.
///
/// Compression is lossy by design — prose is summarised, but code blocks and
/// error lines are never altered. The budget is a soft target: the adapter
/// guarantees output ≤ `budget_tokens * CHARS_PER_TOKEN` but never truncates
/// a code block mid-way.
pub trait IContextCompressorPort: Send + Sync {
    /// Compress `output` to approximately `budget_tokens` tokens.
    ///
    /// Returns the input unchanged if it already fits within the budget.
    fn compress_tool_output(&self, output: &str, budget_tokens: u32) -> String;

    /// Estimate token count using a chars-per-token heuristic.
    /// Default: 4 chars ≈ 1 token (good enough for routing decisions).
    fn estimate_tokens(&self, text: &str) -> u32 {
        (text.len() as u32).saturating_add(3) / 4
    }
}
