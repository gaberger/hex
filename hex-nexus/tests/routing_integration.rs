//! Integration tests for task-type-aware inference routing (ADR-2604142000).
//!
//! Regression-proofs the motivating case: `run nvidia-smi on bazzite` is short
//! but must route to T2.5 because the classifier floor overrides the complexity
//! scorer's "short prompt => T1" default. Also verifies that when the classifier
//! returns `None`, the complexity scorer governs the effective tier alone.
//!
//! Pipeline under test: `select_provider_task_aware` wires
//! `task_type_classifier::classify` + `complexity::score_complexity` with
//! take-max semantics and delegates provider selection to `select_provider`.

use hex_nexus::adapters::spacetime_inference::InferenceProviderRow;
use hex_nexus::quant_router::select_provider_task_aware;
use hex_nexus::remote::transport::TaskTier;
use hex_nexus::task_type_classifier::{classify, TaskType};

// ── Helpers ───────────────────────────────────────────────────────────────

fn make_provider(id: &str, quant: &str, quality: f32) -> InferenceProviderRow {
    InferenceProviderRow {
        provider_id: id.to_string(),
        provider_type: "ollama".to_string(),
        base_url: "http://localhost:11434".to_string(),
        api_key_ref: String::new(),
        models_json: format!("[\"{}\"]", id),
        rate_limit_rpm: 60,
        rate_limit_tpm: 100_000,
        current_rpm: 0,
        current_tpm: 0,
        healthy: 1,
        last_health_check: String::new(),
        avg_latency_ms: 0,
        quantization_level: quant.to_string(),
        context_window: 4096,
        quality_score: quality,
    }
}

/// Full provider ladder so the router can pick the appropriate tier for any
/// effective_tier in {T1, T2, T2.5, T3}. Provider-selection semantics are
/// covered by `tests/quant_routing.rs`; here we just need *some* provider
/// available at every tier so `select_provider_task_aware` returns `Some(_)`.
fn provider_ladder() -> Vec<InferenceProviderRow> {
    vec![
        make_provider("q2-local", "q2", 0.55),
        make_provider("q4-local", "q4", 0.70),
        make_provider("q8-local", "q8", 0.85),
        make_provider("cloud-frontier", "cloud", 0.95),
    ]
}

// ── (a) Short shell command => T2.5 ───────────────────────────────────────

#[test]
fn short_shell_command_routes_to_t2_5() {
    // Motivating case from ADR-2604142000: short prompt, 30 chars, would
    // score Low complexity (=> T1) without the classifier floor.
    let providers = provider_ladder();
    let (effective, selected) = select_provider_task_aware(
        &providers,
        "run nvidia-smi on bazzite",
        TaskTier::T1,
        &[],
    );
    assert_eq!(
        effective,
        TaskTier::T2_5,
        "short shell-command prompt must be raised to T2.5 by the classifier floor"
    );
    assert!(selected.is_some(), "ladder must select a provider at T2.5");
}

#[test]
fn short_shell_command_ollama_variant_also_raises() {
    // Sanity: a different short shell-command form still routes to T2.5.
    let providers = provider_ladder();
    let (effective, _) = select_provider_task_aware(
        &providers,
        "run ollama ps on bazzite",
        TaskTier::T1,
        &[],
    );
    assert_eq!(effective, TaskTier::T2_5);
}

// ── (b) Short "add a comment" => T1 ───────────────────────────────────────

#[test]
fn short_add_a_comment_stays_at_t1() {
    // Neither classifier nor complexity scorer should escalate. The prompt is
    // below the 200-token floor (no complexity points) and contains no task-type
    // keywords — expect T1.
    let providers = provider_ladder();
    let prompt = "add a comment";

    // Classifier must abstain for this prompt — contract check.
    assert!(
        classify(prompt).is_none(),
        "classifier must return None for trivial comment-add prompts"
    );

    let (effective, selected) = select_provider_task_aware(
        &providers,
        prompt,
        TaskTier::T1,
        &[],
    );
    assert_eq!(effective, TaskTier::T1, "trivial prompts must stay at T1");
    assert!(selected.is_some(), "ladder must select a provider at T1");
}

// ── (c) "debug why X fails" => T2.5 ───────────────────────────────────────

#[test]
fn debug_reasoning_prompt_routes_to_t2_5() {
    // "debug" triggers the Reasoning classifier, raised_tier = T2.5. Prompt is
    // short enough that complexity scoring alone would return T1.
    let providers = provider_ladder();
    let prompt = "debug why the inference call fails on first request";

    let (task_type, raised) = classify(prompt).expect("debug keyword must classify");
    assert_eq!(task_type, TaskType::Reasoning);
    assert_eq!(raised, TaskTier::T2_5);

    let (effective, selected) = select_provider_task_aware(
        &providers,
        prompt,
        TaskTier::T1,
        &[],
    );
    assert_eq!(effective, TaskTier::T2_5, "reasoning prompts must route to T2.5");
    assert!(selected.is_some(), "ladder must select a provider at T2.5");
}

// ── (d) Long generic prose => complexity-scorer-governed ──────────────────

#[test]
fn long_generic_prose_is_governed_by_complexity_scorer() {
    // Long prose with NO classifier-triggering keywords (no "run"+tool pair, no
    // "debug"/"convert"/"migrate"/"api endpoint"/etc.). The complexity scorer
    // alone must drive the tier — and since the prompt exceeds 1000 estimated
    // tokens (>4000 chars), it must escalate above T1.
    //
    // Building the prose from a neutral sentence repeated — avoids accidental
    // keyword collisions with the shell/reasoning/file-transform/precise-syntax
    // patterns.
    let sentence = "The onboarding document should describe how new members navigate \
                    the knowledge base, including where introductory material lives, \
                    which sections deserve careful reading, and how onboarding feedback \
                    flows back to the authoring team for continuous improvement. ";
    let long = sentence.repeat(30); // ~6500 chars => ~1600 tokens
    assert!(long.len() > 4000, "test precondition: prompt must exceed 1000 tokens");

    // Contract: classifier must abstain on this neutral prose.
    assert!(
        classify(&long).is_none(),
        "generic prose must not hit any classifier pattern (check for leaked keywords)"
    );

    let providers = provider_ladder();
    let (effective, _) = select_provider_task_aware(
        &providers,
        &long,
        TaskTier::T1,
        &[],
    );

    // Complexity alone must escalate: >1000 tokens adds +4 => High => Q8 => T2.5.
    // This demonstrates the classifier-silent path correctly yields to the
    // complexity scorer (no suppression when classifier returns None).
    assert_eq!(
        effective,
        TaskTier::T2_5,
        "long generic prose must be governed by complexity scorer (expected T2.5 at ~1600 tokens)"
    );
}

#[test]
fn long_generic_prose_caller_tier_floor_preserved() {
    // If a caller passes T3 for a long generic-prose prompt, take-max keeps T3:
    // the complexity scorer must never downgrade the caller's requested tier.
    let sentence = "This paragraph describes general product considerations \
                    without invoking any tooling or subsystem by name. ";
    let long = sentence.repeat(25);

    let providers = provider_ladder();
    let (effective, _) =
        select_provider_task_aware(&providers, &long, TaskTier::T3, &[]);
    assert_eq!(effective, TaskTier::T3);
}
