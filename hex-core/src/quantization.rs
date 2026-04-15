//! QuantizationLevel — first-class quantization tier for inference routing.
//!
//! Enables quantization-aware provider selection: cheap local Q2/Q4 models
//! for simple tasks, escalating to cloud for complex ones. Based on TurboQuant
//! (Google Research 2026) which shows sub-4-bit quantization achieves near-FP16
//! accuracy at 4-8x memory reduction on routine tasks.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Quantization precision tier for an inference provider.
///
/// Ordered from cheapest/fastest (Q2) to most accurate (Cloud).
/// `Ord` is derived from discriminant order so that `Q2 < Q3 < Q4 < Q8 < Fp16 < Cloud`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QuantizationLevel {
    Q2,
    Q3,
    #[default]
    Q4,
    Q8,
    Fp16,
    Cloud,
}

impl QuantizationLevel {
    /// Default quality score for uncalibrated providers at this tier.
    /// Neural Lab calibration will replace these with measured values.
    pub fn default_quality_score(self) -> f32 {
        match self {
            QuantizationLevel::Q2 => 0.60,
            QuantizationLevel::Q3 => 0.70,
            QuantizationLevel::Q4 => 0.80,
            QuantizationLevel::Q8 => 0.92,
            QuantizationLevel::Fp16 => 0.97,
            QuantizationLevel::Cloud => 1.0,
        }
    }

    /// Parse a GGUF model tag suffix into a quantization level.
    ///
    /// Matches the suffix portion of Ollama model names like `llama3.2:3b-q4_k_m`.
    /// Returns `None` if the suffix is not a recognized GGUF quantization tag.
    pub fn from_gguf_tag(tag: &str) -> Option<Self> {
        let lower = tag.to_lowercase();
        GGUF_RULES
            .iter()
            .find(|r| (r.matches)(&lower))
            .map(|r| r.level)
    }

    /// Detect quantization level from an Ollama model name.
    ///
    /// Parses the model name for GGUF suffixes after the colon tag separator.
    /// Examples: `llama3.2:3b-q4_k_m` → Q4, `qwen3:32b` → None (unknown).
    pub fn detect_from_model_name(model_name: &str) -> Option<Self> {
        // Look for the tag part after ':'
        let tag = if let Some(pos) = model_name.rfind(':') {
            &model_name[pos + 1..]
        } else {
            model_name
        };
        Self::from_gguf_tag(tag)
    }

    /// Returns the string representation used in YAML and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            QuantizationLevel::Q2 => "q2",
            QuantizationLevel::Q3 => "q3",
            QuantizationLevel::Q4 => "q4",
            QuantizationLevel::Q8 => "q8",
            QuantizationLevel::Fp16 => "fp16",
            QuantizationLevel::Cloud => "cloud",
        }
    }
}

#[allow(dead_code)]
pub struct GgufRule {
    pub label: &'static str,
    pub level: QuantizationLevel,
    pub signals: &'static [&'static str],
    pub matches: fn(&str) -> bool,
}

fn match_q2(s: &str) -> bool { s.contains("q2_k") || s.contains("q2") }
fn match_q3(s: &str) -> bool { s.contains("q3_k") || s.contains("q3") }
fn match_q4(s: &str) -> bool {
    s.contains("q4_k") || s.contains("q4_0") || s.contains("q4_1") || s.contains("q4")
}
fn match_q5(s: &str) -> bool { s.contains("q5_k") || s.contains("q5") }
fn match_q8(s: &str) -> bool { s.contains("q8_0") || s.contains("q8") }
fn match_fp16(s: &str) -> bool {
    s.contains("f16") || s.contains("fp16") || s.contains("f32") || s.contains("fp32")
}

pub static GGUF_RULES: &[GgufRule] = &[
    GgufRule { label: "q2", level: QuantizationLevel::Q2, signals: &["q2_k", "q2"], matches: match_q2 },
    GgufRule { label: "q3", level: QuantizationLevel::Q3, signals: &["q3_k", "q3"], matches: match_q3 },
    GgufRule { label: "q4", level: QuantizationLevel::Q4, signals: &["q4_k", "q4_0", "q4_1", "q4"], matches: match_q4 },
    GgufRule { label: "q5_as_q4", level: QuantizationLevel::Q4, signals: &["q5_k", "q5"], matches: match_q5 },
    GgufRule { label: "q8", level: QuantizationLevel::Q8, signals: &["q8_0", "q8"], matches: match_q8 },
    GgufRule { label: "fp16", level: QuantizationLevel::Fp16, signals: &["f16", "fp16", "f32", "fp32"], matches: match_fp16 },
];

impl fmt::Display for QuantizationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for QuantizationLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_lowercase();
        // First try GGUF tag detection
        if let Some(level) = Self::from_gguf_tag(&lower) {
            return Ok(level);
        }
        // Then try canonical names
        match lower.as_str() {
            "q2" | "int2" | "2bit" | "2-bit" => Ok(QuantizationLevel::Q2),
            "q3" | "int3" | "3bit" | "3-bit" => Ok(QuantizationLevel::Q3),
            "q4" | "int4" | "4bit" | "4-bit" => Ok(QuantizationLevel::Q4),
            "q8" | "int8" | "8bit" | "8-bit" => Ok(QuantizationLevel::Q8),
            "fp16" | "f16" | "half" | "16bit" | "16-bit" => Ok(QuantizationLevel::Fp16),
            "cloud" | "api" | "full" | "fp32" | "f32" | "none" => Ok(QuantizationLevel::Cloud),
            _ => Err(format!(
                "Unknown quantization level '{}'. Valid values: q2, q3, q4, q8, fp16, cloud, or GGUF tags (q2_k, q4_k_m, etc.)",
                s
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_ascending() {
        assert!(QuantizationLevel::Q2 < QuantizationLevel::Q4);
        assert!(QuantizationLevel::Q4 < QuantizationLevel::Q8);
        assert!(QuantizationLevel::Q8 < QuantizationLevel::Fp16);
        assert!(QuantizationLevel::Fp16 < QuantizationLevel::Cloud);
    }

    #[test]
    fn display_roundtrip() {
        for level in [
            QuantizationLevel::Q2,
            QuantizationLevel::Q3,
            QuantizationLevel::Q4,
            QuantizationLevel::Q8,
            QuantizationLevel::Fp16,
            QuantizationLevel::Cloud,
        ] {
            let s = level.to_string();
            let parsed: QuantizationLevel = s.parse().unwrap();
            assert_eq!(level, parsed);
        }
    }

    #[test]
    fn gguf_tag_parsing() {
        assert_eq!(QuantizationLevel::detect_from_model_name("llama3.2:3b-q2_k"), Some(QuantizationLevel::Q2));
        assert_eq!(QuantizationLevel::detect_from_model_name("llama3.2:3b-q4_k_m"), Some(QuantizationLevel::Q4));
        assert_eq!(QuantizationLevel::detect_from_model_name("llama3.2:3b-q8_0"), Some(QuantizationLevel::Q8));
        assert_eq!(QuantizationLevel::detect_from_model_name("qwen3:32b"), None);
        assert_eq!(QuantizationLevel::detect_from_model_name("qwen3:32b-fp16"), Some(QuantizationLevel::Fp16));
    }

    #[test]
    fn quality_scores_increase_with_tier() {
        let tiers = [
            QuantizationLevel::Q2,
            QuantizationLevel::Q3,
            QuantizationLevel::Q4,
            QuantizationLevel::Q8,
            QuantizationLevel::Fp16,
            QuantizationLevel::Cloud,
        ];
        for window in tiers.windows(2) {
            assert!(window[0].default_quality_score() < window[1].default_quality_score());
        }
    }

    #[test]
    fn gguf_rule_table_invariants() {
        assert!(GGUF_RULES.len() >= 6, "expected at least 6 GGUF rules");
        for rule in GGUF_RULES {
            assert!(!rule.label.is_empty());
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
        }
        let q2_idx = GGUF_RULES.iter().position(|r| r.label == "q2").unwrap();
        let q4_idx = GGUF_RULES.iter().position(|r| r.label == "q4").unwrap();
        assert!(q2_idx < q4_idx,
            "q2 must precede q4 (q2 contains 'q2' which won't match 'q4', but order documents intent)");
    }
}
