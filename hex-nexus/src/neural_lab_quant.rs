//! Quantization-aware Neural Lab calibration experiment (ADR-2603271000, P4).
//!
//! Runs a standardised benchmark prompt through all uncalibrated providers,
//! scores outputs, and writes quality_score back to SpacetimeDB.
//!
//! Triggered via `POST /api/neural-lab/experiments/quant-calibration`.
//! Skips providers that are already calibrated (quality_score >= 0.0).

use std::time::Instant;

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use serde_json::json;

use crate::state::SharedState;

// ── Types ──────────────────────────────────────────────────────────────────

/// Result of calibrating a single provider.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCalibrationResult {
    pub provider_id: String,
    pub quantization_level: String,
    /// -1.0 means calibration was not attempted or failed.
    pub quality_score: f32,
    pub latency_ms: u64,
    pub skipped: bool,
    pub reason: Option<String>,
}

// ── Benchmark ──────────────────────────────────────────────────────────────

/// A simple Rust code-generation task used as the calibration benchmark.
///
/// Chosen because it has a clear correct answer, is short enough to be cheap,
/// and the output is easy to score (look for Rust syntax markers).
const BENCHMARK_PROMPT: &str =
    "Write a Rust function that takes a Vec<i32> and returns the sum of all \
     even numbers. Provide only the function body, no main(), no imports.";

// ── Handler ────────────────────────────────────────────────────────────────

/// `POST /api/neural-lab/experiments/quant-calibration`
///
/// Iterates all registered providers in quantization-tier order (Q2 → Cloud),
/// runs the benchmark prompt through each uncalibrated one, scores the reply,
/// and PATCHes the quality_score back to SpacetimeDB.
///
/// Returns a JSON summary: total, calibrated, skipped, failed, results[].
pub async fn run_quant_calibration_handler(
    State(state): State<SharedState>,
) -> (StatusCode, Json<serde_json::Value>) {
    let stdb = match &state.inference_stdb {
        Some(s) => s,
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "SpacetimeDB not connected" })),
            )
        }
    };

    let mut providers = match stdb.list_providers().await {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to list providers: {}", e) })),
            )
        }
    };

    // Process in quantization tier order: cheapest first.
    providers.sort_by(|a, b| {
        let la = a.quantization_level.parse::<hex_core::QuantizationLevel>()
            .unwrap_or(hex_core::QuantizationLevel::Cloud);
        let lb = b.quantization_level.parse::<hex_core::QuantizationLevel>()
            .unwrap_or(hex_core::QuantizationLevel::Cloud);
        la.cmp(&lb)
    });

    // Use loopback — this handler runs inside hex-nexus, so self-calls always hit localhost.
    let nexus_url = "http://localhost:5555";

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    let mut results: Vec<ProviderCalibrationResult> = Vec::new();

    for provider in &providers {
        // Only OpenRouter providers are supported in batch calibration for now.
        if provider.provider_type != "openrouter" {
            results.push(ProviderCalibrationResult {
                provider_id: provider.provider_id.clone(),
                quantization_level: provider.quantization_level.clone(),
                quality_score: -1.0,
                latency_ms: 0,
                skipped: true,
                reason: Some("non-openrouter provider — use hex inference test".to_string()),
            });
            continue;
        }

        // Skip already-calibrated providers.
        if provider.quality_score >= 0.0 {
            results.push(ProviderCalibrationResult {
                provider_id: provider.provider_id.clone(),
                quantization_level: provider.quantization_level.clone(),
                quality_score: provider.quality_score,
                latency_ms: 0,
                skipped: true,
                reason: Some(format!("already calibrated ({:.2})", provider.quality_score)),
            });
            continue;
        }

        // Resolve API key: stored key_ref first, then injected state config.
        let api_key = if !provider.api_key_ref.is_empty() {
            provider.api_key_ref.clone()
        } else {
            state.openrouter_api_key.clone().unwrap_or_default()
        };

        if api_key.is_empty() {
            results.push(ProviderCalibrationResult {
                provider_id: provider.provider_id.clone(),
                quantization_level: provider.quantization_level.clone(),
                quality_score: -1.0,
                latency_ms: 0,
                skipped: true,
                reason: Some("no API key available".to_string()),
            });
            continue;
        }

        // Parse first model name from JSON array.
        let model: String = serde_json::from_str::<Vec<String>>(&provider.models_json)
            .ok()
            .and_then(|v| v.into_iter().next())
            .unwrap_or_else(|| provider.models_json.trim_matches('"').to_string());

        // Run the benchmark prompt.
        let start = Instant::now();
        let resp = client
            .post(format!("{}/chat/completions", provider.base_url))
            .bearer_auth(&api_key)
            .json(&json!({
                "model": model,
                "messages": [{ "role": "user", "content": BENCHMARK_PROMPT }],
                "max_tokens": 200,
            }))
            .send()
            .await;
        let latency_ms = start.elapsed().as_millis() as u64;

        let quality_score = match resp {
            Err(e) => {
                results.push(ProviderCalibrationResult {
                    provider_id: provider.provider_id.clone(),
                    quantization_level: provider.quantization_level.clone(),
                    quality_score: -1.0,
                    latency_ms,
                    skipped: false,
                    reason: Some(format!("request error: {}", e)),
                });
                continue;
            }
            Ok(r) if !r.status().is_success() => {
                let status = r.status().as_u16();
                results.push(ProviderCalibrationResult {
                    provider_id: provider.provider_id.clone(),
                    quantization_level: provider.quantization_level.clone(),
                    quality_score: -1.0,
                    latency_ms,
                    skipped: false,
                    reason: Some(format!("HTTP {}", status)),
                });
                continue;
            }
            Ok(r) => {
                let body = r.json::<serde_json::Value>().await.unwrap_or_default();
                let reply = body
                    .pointer("/choices/0/message/content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                score_response(&reply, latency_ms)
            }
        };

        // PATCH quality_score back via the calibrate endpoint.
        let patch_url = format!(
            "{}/api/inference/endpoints/{}",
            nexus_url, provider.provider_id
        );
        if let Err(e) = client
            .patch(&patch_url)
            .json(&json!({ "quality_score": quality_score }))
            .send()
            .await
        {
            tracing::warn!(
                provider = %provider.provider_id,
                error = %e,
                "quant-calibration: failed to PATCH quality_score (non-fatal)"
            );
        }

        results.push(ProviderCalibrationResult {
            provider_id: provider.provider_id.clone(),
            quantization_level: provider.quantization_level.clone(),
            quality_score,
            latency_ms,
            skipped: false,
            reason: None,
        });
    }

    let calibrated = results.iter().filter(|r| !r.skipped && r.quality_score >= 0.0).count();
    let skipped = results.iter().filter(|r| r.skipped).count();
    let failed = results.iter().filter(|r| !r.skipped && r.quality_score < 0.0).count();

    tracing::info!(
        calibrated,
        skipped,
        failed,
        "quant-calibration experiment complete"
    );

    (
        StatusCode::OK,
        Json(json!({
            "experimentType": "quant-calibration",
            "total": results.len(),
            "calibrated": calibrated,
            "skipped": skipped,
            "failed": failed,
            "results": results,
        })),
    )
}

// ── Scoring ────────────────────────────────────────────────────────────────

/// Score a provider's response to the benchmark prompt.
///
/// Uses the same formula as `hex inference test` so scores are directly comparable:
/// - 0.70 base
/// - latency bonus: +0.15 (<3s), +0.08 (<8s), +0.02 (<20s), -0.05 (>=20s)
/// - sanity bonus:  +0.15 if Rust code markers present; +0.05 if non-empty
fn score_response(reply: &str, latency_ms: u64) -> f32 {
    let latency_bonus: f32 = if latency_ms < 3_000 {
        0.15
    } else if latency_ms < 8_000 {
        0.08
    } else if latency_ms < 20_000 {
        0.02
    } else {
        -0.05
    };

    let sanity_bonus: f32 = if !reply.is_empty()
        && (reply.contains("fn ")
            || reply.contains("->")
            || reply.contains("for ")
            || reply.contains("let ")
            || reply.contains("iter()"))
    {
        0.15
    } else if !reply.is_empty() {
        0.05
    } else {
        0.0
    };

    (0.70_f32 + latency_bonus + sanity_bonus).clamp(0.0, 1.0)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_rust_reply_scores_high() {
        let reply = "fn sum_even(v: Vec<i32>) -> i32 { v.iter().filter(|&&x| x % 2 == 0).sum() }";
        let score = score_response(reply, 500);
        assert!(score >= 0.99, "fast Rust reply should score near 1.0, got {}", score);
    }

    #[test]
    fn empty_reply_gets_no_sanity_bonus() {
        let score = score_response("", 500);
        assert!((score - 0.85).abs() < 0.01, "empty fast reply: 0.70 + 0.15 = 0.85, got {}", score);
    }

    #[test]
    fn slow_reply_gets_penalty() {
        let reply = "fn sum_even(v: Vec<i32>) -> i32 { 0 }";
        let score = score_response(reply, 25_000);
        assert!(score < 0.85, "slow reply should score below 0.85, got {}", score);
    }

    #[test]
    fn score_clamps_to_one() {
        let reply = "fn foo() -> i32 { let x = 1; for i in 0..10 {} x }";
        let score = score_response(reply, 100);
        assert!(score <= 1.0, "score must not exceed 1.0");
    }
}
