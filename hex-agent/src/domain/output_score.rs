//! Output quality scoring — domain types for LLM output analysis.
//!
//! These types represent the quality dimensions measured after each
//! LLM response, feeding reward signals back into the RL engine.

use serde::{Deserialize, Serialize};

/// A file change produced by an LLM tool_use response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub change_type: ChangeType,
    /// Number of lines added.
    pub lines_added: usize,
    /// Number of lines removed.
    pub lines_removed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Created,
    Modified,
    Deleted,
}

/// Context passed to the output analyzer after an LLM response.
#[derive(Debug, Clone, Serialize)]
pub struct AnalysisContext {
    /// What the agent was asked to do.
    pub task_description: String,
    /// The raw LLM response text.
    pub llm_response: String,
    /// Files created, modified, or deleted by tool_use calls.
    pub files_changed: Vec<FileChange>,
    /// Project root path for architecture analysis.
    pub project_root: String,
    /// Model that produced this response.
    pub model_used: String,
    /// Response latency in milliseconds.
    pub latency_ms: u64,
    /// Total tokens consumed (input + output).
    pub total_tokens: u64,
}

/// Quality score breakdown for an LLM output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputScore {
    /// Overall quality score (0.0–1.0), weighted combination of dimensions.
    pub overall: f64,
    /// Hex boundary compliance (1.0 = no violations introduced).
    pub boundary_compliance: f64,
    /// Whether the generated code compiles.
    pub compiles: Option<bool>,
    /// Whether relevant tests pass.
    pub tests_pass: Option<bool>,
    /// Token efficiency: useful content / total tokens (0.0–1.0).
    pub token_efficiency: f64,
    /// Human-readable feedback items for self-correction.
    pub feedback: Vec<String>,
}

impl OutputScore {
    /// Compute overall score from dimensions.
    ///
    /// Weights:
    /// - Boundary compliance: 30% (architecture correctness is critical)
    /// - Compilation: 25% (non-compiling code is useless)
    /// - Tests: 20% (regression prevention)
    /// - Token efficiency: 10% (cost optimization)
    /// - Base quality: 15% (response was non-empty and addressed the task)
    pub fn compute(
        boundary_compliance: f64,
        compiles: Option<bool>,
        tests_pass: Option<bool>,
        token_efficiency: f64,
        has_content: bool,
    ) -> Self {
        let compile_score = compiles.map_or(0.8, |c| if c { 1.0 } else { 0.0 });
        let test_score = tests_pass.map_or(0.8, |t| if t { 1.0 } else { 0.3 });
        let base = if has_content { 1.0 } else { 0.0 };

        let overall = boundary_compliance * 0.30
            + compile_score * 0.25
            + test_score * 0.20
            + token_efficiency * 0.10
            + base * 0.15;

        let mut feedback = Vec::new();

        if boundary_compliance < 1.0 {
            feedback.push(format!(
                "Architecture: {:.0}% boundary compliance — check hex layer rules",
                boundary_compliance * 100.0
            ));
        }
        if compiles == Some(false) {
            feedback.push("Compilation failed — generated code has errors".to_string());
        }
        if tests_pass == Some(false) {
            feedback.push("Tests failed — regression introduced".to_string());
        }
        if token_efficiency < 0.5 {
            feedback.push(format!(
                "Token efficiency {:.0}% — response may be bloated",
                token_efficiency * 100.0
            ));
        }

        Self {
            overall: overall.clamp(0.0, 1.0),
            boundary_compliance,
            compiles,
            tests_pass,
            token_efficiency,
            feedback,
        }
    }

    /// Convert to an RL reward value (-1.0 to 1.0).
    ///
    /// Maps overall score [0, 1] to reward [-1, 1]:
    /// - 0.0 overall → -1.0 reward (terrible)
    /// - 0.5 overall →  0.0 reward (neutral)
    /// - 1.0 overall →  1.0 reward (excellent)
    pub fn to_reward(&self) -> f64 {
        (self.overall * 2.0) - 1.0
    }

    /// Whether this score is below the self-correction threshold.
    pub fn needs_retry(&self) -> bool {
        self.overall < 0.6
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perfect_score() {
        let score = OutputScore::compute(1.0, Some(true), Some(true), 0.9, true);
        assert!(score.overall > 0.9);
        assert!(score.feedback.is_empty());
        assert!(!score.needs_retry());
    }

    #[test]
    fn compile_failure_drops_score() {
        let score = OutputScore::compute(1.0, Some(false), Some(true), 0.8, true);
        assert!(score.overall < 0.8);
        assert!(score.feedback.iter().any(|f| f.contains("Compilation failed")));
    }

    #[test]
    fn boundary_violation_drops_score() {
        let score = OutputScore::compute(0.5, Some(true), Some(true), 0.8, true);
        assert!(score.overall < 0.9);
        assert!(score.feedback.iter().any(|f| f.contains("boundary compliance")));
    }

    #[test]
    fn empty_response_drops_score() {
        let score = OutputScore::compute(1.0, Some(true), Some(true), 0.0, false);
        assert!(score.overall < 0.9);
    }

    #[test]
    fn reward_mapping() {
        let excellent = OutputScore::compute(1.0, Some(true), Some(true), 1.0, true);
        assert!(excellent.to_reward() > 0.8);

        let terrible = OutputScore::compute(0.0, Some(false), Some(false), 0.0, false);
        assert!(terrible.to_reward() < -0.5);
    }

    #[test]
    fn needs_retry_threshold() {
        let good = OutputScore::compute(1.0, Some(true), Some(true), 0.8, true);
        assert!(!good.needs_retry());

        let bad = OutputScore::compute(0.3, Some(false), Some(false), 0.2, true);
        assert!(bad.needs_retry());
    }

    #[test]
    fn unknown_compile_test_status() {
        // When compile/test status is unknown, use neutral score (0.8)
        let score = OutputScore::compute(1.0, None, None, 0.8, true);
        assert!(score.overall > 0.7);
        assert!(score.feedback.is_empty());
    }

    #[test]
    fn token_efficiency_feedback() {
        let score = OutputScore::compute(1.0, Some(true), Some(true), 0.3, true);
        assert!(score.feedback.iter().any(|f| f.contains("Token efficiency")));
    }
}
