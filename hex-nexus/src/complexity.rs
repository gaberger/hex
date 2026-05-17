//! Task complexity scoring for quantization-aware inference routing (ADR-2603271000).
//!
//! Estimates how complex an inference request is so the router can select the
//! minimum quantization tier that will produce acceptable quality output.

use hex_core::QuantizationLevel;

/// Estimated complexity of an inference task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComplexityLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl ComplexityLevel {
    /// Minimum quantization level required for this complexity.
    /// Uses conservative defaults — biased toward escalation until Neural Lab calibrates.
    pub fn min_quantization(self) -> QuantizationLevel {
        match self {
            ComplexityLevel::Low => QuantizationLevel::Q2,
            ComplexityLevel::Medium => QuantizationLevel::Q4,
            ComplexityLevel::High => QuantizationLevel::Q8,
            ComplexityLevel::Critical => QuantizationLevel::Cloud,
        }
    }
}

/// Score the complexity of an inference request from its prompt text and context files.
///
/// Returns a `ComplexityLevel` that maps to a minimum quantization tier.
/// The heuristic is additive — multiple signals push the score higher.
pub fn score_complexity(prompt: &str, context_files: &[&str]) -> ComplexityLevel {
    let mut score: i32 = 0;

    // ── Prompt token estimate (1 token ≈ 4 chars) ─────────────────────
    let estimated_tokens = prompt.len() / 4;
    if estimated_tokens < 200 {
        // Low base — short prompts are typically simple
    } else if estimated_tokens < 500 {
        score += 1;
    } else if estimated_tokens < 1000 {
        score += 2;
    } else {
        score += 4; // Long prompts (>1000 tokens) almost always need higher fidelity
    }

    // ── Cross-file / cross-adapter signals ────────────────────────────
    let cross_file_keywords = [
        "import from",
        "depends on",
        "across adapters",
        "cross-adapter",
        "multiple files",
        "refactor across",
        "move to",
        "rename across",
    ];
    let has_cross_file = cross_file_keywords
        .iter()
        .any(|k| prompt.to_lowercase().contains(k));
    if has_cross_file {
        score += 1;
    }

    // Context files signal cross-file work
    if context_files.len() > 3 {
        score += 1;
    }

    // ── Security-sensitive signals (minimum Medium) ────────────────────
    let security_keywords = [
        "auth", "secret", "vault", "credential", "token", "api key",
        "permission", "access control", "encrypt", "decrypt",
    ];
    let has_security = security_keywords.iter().any(|k| prompt.to_lowercase().contains(k));

    // ── Architecture-level signals (minimum High) ─────────────────────
    let arch_keywords = [
        "adr", "hexagonal", "composition-root", "architecture decision",
        "port interface", "adapter boundary", "dependency injection",
        "domain layer", "use case", "bounded context",
    ];
    let has_arch = arch_keywords.iter().any(|k| prompt.to_lowercase().contains(k));

    // ── Map score + overrides to ComplexityLevel ──────────────────────
    let base_level = match score {
        0..=1 => ComplexityLevel::Low,
        2..=3 => ComplexityLevel::Medium,
        4..=5 => ComplexityLevel::High,
        _ => ComplexityLevel::Critical,
    };

    // Apply minimums from keyword signals
    let mut level = base_level;
    if has_cross_file && level < ComplexityLevel::Medium {
        level = ComplexityLevel::Medium;
    }
    if has_security && level < ComplexityLevel::Medium {
        level = ComplexityLevel::Medium;
    }
    if has_arch && level < ComplexityLevel::High {
        level = ComplexityLevel::High;
    }

    level
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_simple_prompt_is_low() {
        let complexity = score_complexity("Add a docstring to this function", &[]);
        assert_eq!(complexity, ComplexityLevel::Low);
    }

    #[test]
    fn long_prompt_is_at_least_medium() {
        let long = "x".repeat(4001); // ~1000 tokens
        let complexity = score_complexity(&long, &[]);
        assert!(complexity >= ComplexityLevel::High);
    }

    #[test]
    fn cross_file_keyword_increases_score() {
        let complexity = score_complexity("Refactor this to import from the new port", &[]);
        assert!(complexity >= ComplexityLevel::Medium);
    }

    #[test]
    fn security_keyword_forces_medium() {
        let complexity = score_complexity("Add auth check here", &[]);
        assert!(complexity >= ComplexityLevel::Medium);
    }

    #[test]
    fn arch_keyword_forces_high() {
        let complexity = score_complexity("Update the ADR for this component", &[]);
        assert!(complexity >= ComplexityLevel::High);
    }

    #[test]
    fn min_quantization_mapping() {
        assert_eq!(ComplexityLevel::Low.min_quantization(), QuantizationLevel::Q2);
        assert_eq!(ComplexityLevel::Medium.min_quantization(), QuantizationLevel::Q4);
        assert_eq!(ComplexityLevel::High.min_quantization(), QuantizationLevel::Q8);
        assert_eq!(ComplexityLevel::Critical.min_quantization(), QuantizationLevel::Cloud);
    }
}
