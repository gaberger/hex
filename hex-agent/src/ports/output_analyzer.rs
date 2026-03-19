//! Output Analyzer Port — contract for LLM output quality analysis.
//!
//! Adapters implement this to analyze LLM responses and produce quality
//! scores that feed into the RL reward signal.

use async_trait::async_trait;

// Re-export domain types so adapters import from ports, not domain (hex boundary rule)
pub use crate::domain::output_score::{AnalysisContext, ChangeType, FileChange, OutputScore};

/// Analyze LLM output quality for RL feedback.
///
/// Implementations may:
/// - Call hex-nexus `/api/analyze` for boundary checking
/// - Run `cargo check` / `bun run check` for compilation
/// - Run test suites for regression detection
/// - Compute token efficiency metrics
#[async_trait]
pub trait OutputAnalyzerPort: Send + Sync {
    /// Analyze an LLM response and return a quality score.
    ///
    /// This is called after each tool_use response that modifies files.
    /// The score feeds into `RlPort::report_reward()`.
    async fn analyze(&self, context: &AnalysisContext) -> OutputScore;
}

/// No-op analyzer for when analysis is disabled or unavailable.
pub struct NoopOutputAnalyzer;

#[async_trait]
impl OutputAnalyzerPort for NoopOutputAnalyzer {
    async fn analyze(&self, _context: &AnalysisContext) -> OutputScore {
        // Return a neutral score — no analysis performed
        OutputScore::compute(1.0, None, None, 0.7, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::output_score::AnalysisContext;

    #[tokio::test]
    async fn noop_returns_neutral_score() {
        let analyzer = NoopOutputAnalyzer;
        let ctx = AnalysisContext {
            task_description: "test".to_string(),
            llm_response: "done".to_string(),
            files_changed: vec![],
            project_root: ".".to_string(),
            model_used: "test".to_string(),
            latency_ms: 100,
            total_tokens: 500,
        };
        let score = analyzer.analyze(&ctx).await;
        assert!(score.overall > 0.5);
        assert!(!score.needs_retry());
    }
}
