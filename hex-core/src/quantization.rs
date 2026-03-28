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
        if lower.contains("q2_k") || lower.contains("q2") {
            Some(QuantizationLevel::Q2)
        } else if lower.contains("q3_k") || lower.contains("q3") {
            Some(QuantizationLevel::Q3)
        } else if lower.contains("q4_k") || lower.contains("q4_0") || lower.contains("q4_1") || lower.contains("q4") {
            Some(QuantizationLevel::Q4)
        } else if lower.contains("q5_k") || lower.contains("q5") {
            // Q5 is between Q4 and Q8 — map to Q4 (conservative)
            Some(QuantizationLevel::Q4)
        } else if lower.contains("q8_0") || lower.contains("q8") {
            Some(QuantizationLevel::Q8)
        } else if lower.contains("f16") || lower.contains("fp16")
            || lower.contains("f32") || lower.contains("fp32")
        {
            // fp32 is treated as fp16 tier (conservative downgrade)
            Some(QuantizationLevel::Fp16)
        } else {
            None
        }
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
}
