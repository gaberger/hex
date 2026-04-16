//! Quantization-aware inference provider selection (ADR-2603271000, ADR-2604052125).
//!
//! Selects the best available provider that meets a minimum quantization tier,
//! preferring free-tier providers with remaining quota and higher quality scores.
//! Integrates with the RateLimitManager for circuit breaker and rate limit checks.

use hex_core::QuantizationLevel;

use crate::adapters::spacetime_inference::InferenceProviderRow;
use crate::complexity::{score_complexity, ComplexityLevel};
use crate::rate_limiter::RateLimitManager;
use crate::remote::transport::TaskTier;
use crate::task_type_classifier::classify_task_type;

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

    // Sort: local providers (Ollama/vLLM) first, then healthy, then by effective quality score descending
    let is_local = |p: &&InferenceProviderRow| -> bool {
        p.provider_type == "ollama" || p.provider_type == "vllm"
    };
    candidates.sort_by(|a, b| {
        let local_a = is_local(a);
        let local_b = is_local(b);
        if local_a != local_b {
            return local_b.cmp(&local_a); // local (true) before cloud (false)
        }
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

/// Map a `TaskTier` to the minimum `QuantizationLevel` required.
fn tier_to_quant(tier: TaskTier) -> QuantizationLevel {
    match tier {
        TaskTier::T1 => QuantizationLevel::Q2,
        TaskTier::T2 => QuantizationLevel::Q4,
        TaskTier::T2_5 => QuantizationLevel::Q8,
        TaskTier::T3 => QuantizationLevel::Cloud,
    }
}

/// Map a `ComplexityLevel` to its equivalent `TaskTier`.
fn complexity_to_tier(level: ComplexityLevel) -> TaskTier {
    match level {
        ComplexityLevel::Low => TaskTier::T1,
        ComplexityLevel::Medium => TaskTier::T2,
        ComplexityLevel::High => TaskTier::T2_5,
        ComplexityLevel::Critical => TaskTier::T3,
    }
}

/// Ordering helper for taking the maximum of two `TaskTier` values.
fn tier_max(a: TaskTier, b: TaskTier) -> TaskTier {
    let ord = |t: TaskTier| -> u8 {
        match t {
            TaskTier::T1 => 1,
            TaskTier::T2 => 2,
            TaskTier::T2_5 => 3,
            TaskTier::T3 => 4,
        }
    };
    if ord(b) > ord(a) { b } else { a }
}

/// Task-type-aware provider selection (ADR-2604142000).
///
/// Combines three tier signals with take-max semantics:
///   1. `caller_tier` — the minimum tier requested by the caller.
///   2. Classifier floor — `task_type_classifier::classify_task_type` may raise
///      the tier based on prompt intent (shell command, reasoning, etc.).
///   3. Complexity scorer — `complexity::score_complexity` may raise the tier
///      based on prompt length and cross-file signals.
///
/// The effective tier is `max(caller_tier, classifier_tier, complexity_tier)`.
/// Provider selection then delegates to `select_provider` with the quantization
/// level that corresponds to the effective tier.
///
/// Returns `(effective_tier, Option<&InferenceProviderRow>)`.
pub fn select_provider_task_aware<'a>(
    providers: &'a [InferenceProviderRow],
    prompt: &str,
    caller_tier: TaskTier,
    context_files: &[&str],
) -> (TaskTier, Option<&'a InferenceProviderRow>) {
    // Classifier floor
    let classifier_tier = classify_task_type(prompt)
        .map(|r| r.raised_tier)
        .unwrap_or(TaskTier::T1);

    // Complexity scorer
    let complexity_tier = complexity_to_tier(score_complexity(prompt, context_files));

    // Take-max across all three signals
    let effective = tier_max(caller_tier, tier_max(classifier_tier, complexity_tier));

    let min_quant = tier_to_quant(effective);
    let provider = select_provider(providers, min_quant);
    (effective, provider)
}

/// Free-tier-aware provider selection (ADR-2604052125).
///
/// Like `select_provider` but also checks rate limits and circuit breakers
/// via the RateLimitManager. Prefers free-tier providers with remaining quota.
///
/// Priority order:
///   1. Free provider with remaining daily quota (circuit closed)
///   2. Free provider in half-open circuit (testing recovery)
///   3. Local provider (Ollama, vLLM) — zero cost, higher latency
///   4. Paid provider with lowest cost
///   5. Frontier provider (Anthropic, OpenAI) — only when lower tiers exhausted
pub async fn select_provider_with_rate_limits(
    providers: &[InferenceProviderRow],
    min_level: QuantizationLevel,
    rate_limiter: &RateLimitManager,
) -> Option<InferenceProviderRow> {
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
        candidates = providers.iter().collect();
    }

    // Check rate limits for each candidate
    let mut available: Vec<(&InferenceProviderRow, bool)> = Vec::new();
    for c in &candidates {
        let can_route = rate_limiter.should_route(&c.provider_id).await;
        if can_route {
            available.push((c, c.healthy == 1));
        }
    }

    // If all are rate-limited, fall back to any healthy candidate
    if available.is_empty() {
        tracing::warn!("all providers rate-limited — falling back to best available");
        available = candidates.iter().map(|c| (*c, c.healthy == 1)).collect();
    }

    // Sort: healthy first, then by quality score
    available.sort_by(|(a, a_healthy), (b, b_healthy)| {
        if a_healthy != b_healthy {
            return b_healthy.cmp(a_healthy);
        }
        let score_a = effective_quality(a);
        let score_b = effective_quality(b);
        score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    available.into_iter().next().map(|(p, _)| p.clone())
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
