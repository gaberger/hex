//! Quantization-aware inference provider selection (ADR-2603271000).
//!
//! Selects the best available provider that meets a minimum quantization tier,
//! preferring higher quality scores and escalating on failure.

use hex_core::QuantizationLevel;

use crate::adapters::spacetime_inference::InferenceProviderRow;

/// Policy for quantization selection, read from agent YAML `model.quantization`.
#[derive(Debug, Clone)]
pub struct QuantPolicy {
    /// Default tier to use for this agent's tasks.
    pub default: QuantizationLevel,
    /// Hard floor — never use a provider below this tier.
    pub minimum: QuantizationLevel,
    /// Tier to escalate to when complexity is scored High.
    pub on_complexity_high: QuantizationLevel,
    /// Tier to escalate to after repeated provider failures.
    pub on_failure: QuantizationLevel,
}

impl Default for QuantPolicy {
    fn default() -> Self {
        Self {
            default: QuantizationLevel::Q4,
            minimum: QuantizationLevel::Q2,
            on_complexity_high: QuantizationLevel::Q8,
            on_failure: QuantizationLevel::Cloud,
        }
    }
}

/// Select the best provider meeting the minimum quantization requirement.
///
/// Filtering:
///   1. Only providers with `quantization_level >= min_level` are candidates.
///   2. Only healthy (or unknown) providers are preferred; unhealthy providers
///      are included as last resort.
///
/// Sorting (among candidates):
///   1. Healthy providers before unhealthy.
///   2. Higher quality score wins; uncalibrated (-1.0) uses tier defaults.
///
/// Returns the first match, or `None` if no providers are registered.
pub fn select_provider(
    providers: &[InferenceProviderRow],
    min_level: QuantizationLevel,
) -> Option<&InferenceProviderRow> {
    if providers.is_empty() {
        return None;
    }

    let mut candidates: Vec<&InferenceProviderRow> = providers
        .iter()
        .filter(|p| {
            let level = p.quantization_level
                .parse::<QuantizationLevel>()
                .unwrap_or(QuantizationLevel::Q4);
            level >= min_level
        })
        .collect();

    if candidates.is_empty() {
        // No provider meets the minimum — fall back to best available
        candidates = providers.iter().collect();
    }

    // Sort: healthy first, then by effective quality score descending
    candidates.sort_by(|a, b| {
        let health_a = a.healthy;
        let health_b = b.healthy;
        if health_a != health_b {
            return health_b.cmp(&health_a); // healthy (1) before unhealthy (0)
        }
        let score_a = effective_quality(a);
        let score_b = effective_quality(b);
        score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    candidates.into_iter().next()
}

/// Get the effective quality score for a provider.
///
/// If `quality_score` is -1.0 (uncalibrated), falls back to the tier default.
pub fn effective_quality(provider: &InferenceProviderRow) -> f32 {
    if provider.quality_score >= 0.0 {
        provider.quality_score
    } else {
        let level = provider.quantization_level
            .parse::<QuantizationLevel>()
            .unwrap_or(QuantizationLevel::Q4);
        level.default_quality_score()
    }
}

/// Get the next higher quantization tier for escalation on failure.
pub fn escalate_tier(current: QuantizationLevel) -> QuantizationLevel {
    match current {
        QuantizationLevel::Q2 => QuantizationLevel::Q4,
        QuantizationLevel::Q3 => QuantizationLevel::Q4,
        QuantizationLevel::Q4 => QuantizationLevel::Q8,
        QuantizationLevel::Q8 => QuantizationLevel::Fp16,
        QuantizationLevel::Fp16 => QuantizationLevel::Cloud,
        QuantizationLevel::Cloud => QuantizationLevel::Cloud,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider(id: &str, quant: &str, quality: f32, healthy: u8) -> InferenceProviderRow {
        InferenceProviderRow {
            provider_id: id.to_string(),
            provider_type: "ollama".to_string(),
            base_url: "http://localhost:11434".to_string(),
            api_key_ref: String::new(),
            models_json: "[\"test\"]".to_string(),
            rate_limit_rpm: 60,
            rate_limit_tpm: 100_000,
            current_rpm: 0,
            current_tpm: 0,
            healthy,
            last_health_check: String::new(),
            avg_latency_ms: 0,
            quantization_level: quant.to_string(),
            context_window: 4096,
            quality_score: quality,
        }
    }

    #[test]
    fn prefers_higher_quality_among_qualifying() {
        let providers = vec![
            make_provider("low-quality", "q4", 0.5, 1),
            make_provider("high-quality", "q8", 0.9, 1),
        ];
        let selected = select_provider(&providers, QuantizationLevel::Q4).unwrap();
        assert_eq!(selected.provider_id, "high-quality");
    }

    #[test]
    fn filters_below_minimum_tier() {
        let providers = vec![
            make_provider("q2-provider", "q2", 0.9, 1),
            make_provider("q8-provider", "q8", 0.7, 1),
        ];
        // Minimum Q4 — q2 should be excluded, q8 selected
        let selected = select_provider(&providers, QuantizationLevel::Q4).unwrap();
        assert_eq!(selected.provider_id, "q8-provider");
    }

    #[test]
    fn falls_back_when_no_provider_meets_minimum() {
        let providers = vec![make_provider("q2-only", "q2", 0.8, 1)];
        // Minimum is Cloud but only q2 exists — returns q2 as best available
        let selected = select_provider(&providers, QuantizationLevel::Cloud).unwrap();
        assert_eq!(selected.provider_id, "q2-only");
    }

    #[test]
    fn healthy_preferred_over_unhealthy() {
        let providers = vec![
            make_provider("unhealthy-q8", "q8", 0.95, 0),
            make_provider("healthy-q4", "q4", 0.80, 1),
        ];
        let selected = select_provider(&providers, QuantizationLevel::Q4).unwrap();
        assert_eq!(selected.provider_id, "healthy-q4");
    }

    #[test]
    fn escalation_ladder() {
        assert_eq!(escalate_tier(QuantizationLevel::Q2), QuantizationLevel::Q4);
        assert_eq!(escalate_tier(QuantizationLevel::Q4), QuantizationLevel::Q8);
        assert_eq!(escalate_tier(QuantizationLevel::Q8), QuantizationLevel::Fp16);
        assert_eq!(escalate_tier(QuantizationLevel::Cloud), QuantizationLevel::Cloud);
    }
}
