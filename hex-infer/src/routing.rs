//! Routing — which endpoint and which model serve this request.
//!
//! Lifted out of the daemon's `inference_complete` handler per
//! ADR-2608241500 P2.4. Everything here is a pure function over the config
//! and the request, which is why it is testable at all; in the handler these
//! rules were inline closures tangled with `State(...)` reads.
//!
//! Three jobs:
//!
//! 1. [`tier_model`] — turn a task's `strategy_hint` into a tier, then a model.
//! 2. [`candidates_for_tools`] — order tools-capable endpoints for the fast path.
//! 3. [`normalize_for_openrouter`] — make a bare model id routable on OpenRouter.

use crate::config::{models_of, InferenceConfig};
use crate::endpoint::Endpoint;

/// The model OpenRouter should serve when the caller asked for "something free".
pub const DEFAULT_FREE_MODEL: &str = "openai/gpt-4o-mini";

/// Free models to try, best first, when every registered provider has failed.
///
/// Ordered by capability, not by response speed: code generation should get
/// the strongest free model available rather than whichever one answers first.
pub const FREE_MODEL_CHAIN: &[&str] = &[
    "openai/gpt-4o-mini",
    "meta-llama/llama-3.3-70b-instruct:free",
    "mistralai/mistral-small-3.1-24b-instruct:free",
    "deepseek/deepseek-r1:free",
    "meta-llama/llama-3.2-3b-instruct:free",
    "arcee-ai/trinity-mini:free",
];

/// Map a workplan task's `strategy_hint` to a tier name.
///
/// Per ADR-2026-04-12-0202 and ADR-2026-04-13-1630:
/// `scaffold` / `transform` / `script` → T1, `codegen` → T2,
/// `inference` → T2.5. Anything unrecognised gets T2, the safe middle.
pub fn tier_for_strategy(strategy_hint: &str) -> &'static str {
    match strategy_hint.trim().to_ascii_lowercase().as_str() {
        "scaffold" | "transform" | "script" => "t1",
        "inference" => "t2.5",
        _ => "t2",
    }
}

/// The model a strategy hint resolves to, or `None` when the tier is
/// unconfigured — the caller then falls back to its own default rather than
/// guessing a model that may not be installed.
pub fn tier_model<'a>(cfg: &'a InferenceConfig, strategy_hint: &str) -> Option<&'a str> {
    cfg.tier_model(tier_for_strategy(strategy_hint))
}

/// Does this endpoint advertise `model`?
///
/// Exact element match, never substring: a substring search routes a request
/// for `llama-3` to any provider whose list happens to contain
/// `meta-llama/llama-3.3-70b-instruct`.
pub fn serves_model(entry: &serde_json::Value, model: &str) -> bool {
    models_of(entry).iter().any(|m| m == model)
}

/// Order tools-capable endpoints for the tools fast-path.
///
/// Base order is local-and-free first (see [`Endpoint::priority_for_tools`]).
/// When the caller named a model, the endpoint that actually serves it wins
/// outright — model match dominates locality. Without that rule a tier request
/// for a cloud model (T2 `Qwen/Qwen3-32B` on Tenstorrent) is hijacked by
/// whatever local Ollama model happens to be registered, because Ollama ranks
/// above openai-compat. Priority then only breaks ties inside the matched and
/// unmatched groups.
pub fn candidates_for_tools(endpoints: &[Endpoint], requested_model: Option<&str>) -> Vec<Endpoint> {
    let mut candidates: Vec<Endpoint> = endpoints
        .iter()
        .filter(|e| e.supports_tools())
        .cloned()
        .collect();

    match requested_model {
        Some(m) => candidates.sort_by_key(|e| {
            let matched = if e.model == m { 0 } else { 1 };
            (matched, e.priority_for_tools())
        }),
        None => candidates.sort_by_key(Endpoint::priority_for_tools),
    }
    candidates
}

/// A synthetic OpenRouter endpoint built from a bare API key.
///
/// Appended as the last-resort candidate so an operator with a key but no
/// registered providers still gets a working call.
pub fn openrouter_from_key(id: &str, key: &str, model: &str) -> Endpoint {
    Endpoint {
        id: id.to_string(),
        url: "https://openrouter.ai/api/v1".to_string(),
        provider: "openrouter".to_string(),
        model: model.to_string(),
        status: "unknown".to_string(),
        requires_auth: true,
        secret_key: key.to_string(),
        health_checked_at: String::new(),
        quality_score: 0.0,
        quantization_level: "cloud".to_string(),
    }
}

/// Is an OpenRouter endpoint already in the candidate list?
pub fn has_openrouter(candidates: &[Endpoint]) -> bool {
    candidates
        .iter()
        .any(|c| c.provider.eq_ignore_ascii_case("openrouter") || c.url.contains("openrouter.ai"))
}

/// Resolve the pseudo-model `openrouter/free` (and an empty request) to a
/// real, consistently-available free model.
pub fn resolve_free_model(requested: Option<&str>) -> String {
    match requested {
        Some(m) if m == "openrouter/free" || m.is_empty() => DEFAULT_FREE_MODEL.to_string(),
        Some(m) => m.to_string(),
        None => DEFAULT_FREE_MODEL.to_string(),
    }
}

/// Normalise a bare model id to OpenRouter's vendor-namespaced form.
///
/// OpenRouter wants `anthropic/claude-sonnet-4-6`, not `claude-sonnet-4-6`.
/// Covers the common families; an unknown bare id passes through unchanged and
/// will 404 on OpenRouter, which triggers the free-model fallback chain.
pub fn normalize_for_openrouter(model: &str) -> String {
    if model.contains('/') {
        return model.to_string();
    }
    if model.starts_with("claude-") {
        return normalize_claude(model);
    }
    if model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        return format!("openai/{model}");
    }
    if model.starts_with("gemini-") {
        return format!("google/{model}");
    }
    if model.starts_with("mistral-") || model.starts_with("mixtral-") {
        return format!("mistralai/{model}");
    }
    if model.starts_with("deepseek-") {
        return format!("deepseek/{model}");
    }
    if model.contains(':') {
        // Ollama-style (e.g. qwen2.5-coder:14b) — will not resolve on
        // OpenRouter. When no local provider is registered to serve it,
        // substitute a tool-capable default so the chain does not 502 on
        // "not a valid model ID".
        tracing::warn!(
            requested = %model,
            fallback = "anthropic/claude-haiku-4.5",
            "Ollama-style model substituted with an OpenRouter-capable default"
        );
        return "anthropic/claude-haiku-4.5".to_string();
    }
    model.to_string()
}

/// Anthropic-direct ids use dashes and an 8-digit date suffix
/// (`claude-haiku-4-5-20251001`). OpenRouter uses a dotted version and no date
/// (`claude-haiku-4.5`). Convert so callers may use either form.
fn normalize_claude(model: &str) -> String {
    let mut parts: Vec<&str> = model.split('-').collect();
    if parts
        .last()
        .is_some_and(|p| p.len() == 8 && p.chars().all(|c| c.is_ascii_digit()))
    {
        parts.pop();
    }
    let n = parts.len();
    let is_num = |s: &str| s.chars().all(|c| c.is_ascii_digit());

    // Pattern A: claude-FAMILY-MAJOR-MINOR → anthropic/claude-FAMILY-MAJOR.MINOR
    if n >= 4 && is_num(parts[n - 1]) && is_num(parts[n - 2]) {
        let prefix = parts[..n - 2].join("-");
        return format!("anthropic/{}-{}.{}", prefix, parts[n - 2], parts[n - 1]);
    }
    // Pattern B: claude-MAJOR-MINOR-FAMILY → anthropic/claude-MAJOR.MINOR-FAMILY
    if n >= 4 && is_num(parts[1]) && is_num(parts[2]) {
        let suffix = parts[3..].join("-");
        return format!("anthropic/claude-{}.{}-{}", parts[1], parts[2], suffix);
    }
    format!("anthropic/{model}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(id: &str, provider: &str, model: &str, status: &str) -> Endpoint {
        Endpoint {
            id: id.into(),
            url: format!("http://{id}"),
            provider: provider.into(),
            model: model.into(),
            status: status.into(),
            requires_auth: false,
            secret_key: String::new(),
            health_checked_at: String::new(),
            quality_score: 0.0,
            quantization_level: String::new(),
        }
    }

    #[test]
    fn strategy_hints_map_to_the_documented_tiers() {
        assert_eq!(tier_for_strategy("scaffold"), "t1");
        assert_eq!(tier_for_strategy("transform"), "t1");
        assert_eq!(tier_for_strategy("script"), "t1");
        assert_eq!(tier_for_strategy("codegen"), "t2");
        assert_eq!(tier_for_strategy("inference"), "t2.5");
        assert_eq!(tier_for_strategy("INFERENCE"), "t2.5");
        assert_eq!(tier_for_strategy("anything else"), "t2");
    }

    #[test]
    fn local_endpoints_lead_when_no_model_is_requested() {
        let eps = vec![
            ep("or", "openrouter", "x", "healthy"),
            ep("oll", "ollama", "qwen3:4b", "healthy"),
        ];
        let c = candidates_for_tools(&eps, None);
        assert_eq!(c[0].id, "oll");
    }

    /// The 2026-06-07 hijack: a cloud T2 request answered by a local model
    /// because Ollama outranks openai-compat.
    #[test]
    fn a_named_model_beats_locality() {
        let eps = vec![
            ep("oll", "ollama", "gemma4-12b", "healthy"),
            ep("tt", "openai_compat", "Qwen/Qwen3-32B", "healthy"),
        ];
        let c = candidates_for_tools(&eps, Some("Qwen/Qwen3-32B"));
        assert_eq!(c[0].id, "tt", "the endpoint serving the named model must lead");
        assert_eq!(c[1].id, "oll", "the rest keep local-first order");
    }

    #[test]
    fn endpoints_without_tool_support_are_excluded() {
        let eps = vec![ep("m", "mystery", "x", "healthy"), ep("oll", "ollama", "y", "healthy")];
        let c = candidates_for_tools(&eps, None);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].id, "oll");
    }

    #[test]
    fn model_match_is_exact_not_substring() {
        let entry = serde_json::json!({"models": "[\"meta-llama/llama-3.3-70b-instruct\"]"});
        assert!(!serves_model(&entry, "llama-3"));
        assert!(serves_model(&entry, "meta-llama/llama-3.3-70b-instruct"));
    }

    #[test]
    fn openrouter_presence_is_detected_by_family_or_url() {
        assert!(has_openrouter(&[ep("a", "openrouter", "m", "unknown")]));
        let mut by_url = ep("b", "custom", "m", "unknown");
        by_url.url = "https://openrouter.ai/api/v1".into();
        assert!(has_openrouter(&[by_url]));
        assert!(!has_openrouter(&[ep("c", "ollama", "m", "unknown")]));
    }

    #[test]
    fn free_model_pseudonyms_resolve_to_a_real_model() {
        assert_eq!(resolve_free_model(Some("openrouter/free")), DEFAULT_FREE_MODEL);
        assert_eq!(resolve_free_model(Some("")), DEFAULT_FREE_MODEL);
        assert_eq!(resolve_free_model(None), DEFAULT_FREE_MODEL);
        assert_eq!(resolve_free_model(Some("anthropic/claude-haiku-4.5")), "anthropic/claude-haiku-4.5");
    }

    #[test]
    fn a_slugged_model_passes_through_untouched() {
        assert_eq!(normalize_for_openrouter("google/gemini-2.0-flash-001"), "google/gemini-2.0-flash-001");
    }

    #[test]
    fn claude_ids_convert_between_anthropic_and_openrouter_spelling() {
        assert_eq!(normalize_for_openrouter("claude-haiku-4-5-20251001"), "anthropic/claude-haiku-4.5");
        assert_eq!(normalize_for_openrouter("claude-sonnet-4-6"), "anthropic/claude-sonnet-4.6");
        assert_eq!(normalize_for_openrouter("claude-3-7-sonnet"), "anthropic/claude-3.7-sonnet");
    }

    #[test]
    fn common_vendor_families_get_their_slug() {
        assert_eq!(normalize_for_openrouter("gpt-4o-mini"), "openai/gpt-4o-mini");
        assert_eq!(normalize_for_openrouter("o3-mini"), "openai/o3-mini");
        assert_eq!(normalize_for_openrouter("gemini-2.0-flash"), "google/gemini-2.0-flash");
        assert_eq!(normalize_for_openrouter("mistral-large"), "mistralai/mistral-large");
        assert_eq!(normalize_for_openrouter("deepseek-r1"), "deepseek/deepseek-r1");
    }

    #[test]
    fn an_ollama_style_tag_becomes_a_tool_capable_default() {
        assert_eq!(normalize_for_openrouter("qwen2.5-coder:14b"), "anthropic/claude-haiku-4.5");
    }

    #[test]
    fn an_unknown_bare_id_passes_through_to_fail_loudly_upstream() {
        assert_eq!(normalize_for_openrouter("some-local-thing"), "some-local-thing");
    }

    #[test]
    fn a_key_built_endpoint_is_shaped_for_openrouter() {
        let e = openrouter_from_key("fallback", "sk-or-v1-x", "openai/gpt-4o-mini");
        assert!(e.requires_auth);
        assert!(e.supports_tools());
        assert_eq!(e.url, "https://openrouter.ai/api/v1");
    }
}

/// Pick an endpoint when the caller named no model.
///
/// Replaces the daemon's quantization router (`quant_router::select_provider`)
/// with the half of it that survives the collapse. The old router classified
/// prompt complexity to choose a minimum weight-quantisation tier, which
/// mattered when a fleet was choosing among many cloud providers. A solo agent
/// names its model through tier routing (see [`tier_model`]), so the implicit
/// guess is dead weight — but the ordering it applied is not, and it is kept:
/// local first, then healthy, then highest benchmarked quality.
pub fn pick_default(endpoints: &[Endpoint]) -> Option<&Endpoint> {
    endpoints.iter().min_by(|a, b| {
        let key = |e: &Endpoint| (!e.is_local(), e.priority_for_tools());
        key(a).cmp(&key(b)).then_with(|| {
            // Higher quality wins; NaN sorts last rather than panicking.
            b.quality_score
                .partial_cmp(&a.quality_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    })
}

/// Find the endpoint that serves `model`, searching the registry entries'
/// full advertised model lists rather than only the primary model.
pub fn find_serving<'a>(endpoints: &'a [Endpoint], model: &str) -> Option<&'a Endpoint> {
    endpoints
        .iter()
        .find(|e| e.model == model)
        // An OpenRouter-format id (`vendor/model`) routes through any
        // registered OpenRouter endpoint that has a key.
        .or_else(|| {
            model.contains('/').then(|| {
                endpoints
                    .iter()
                    .find(|e| e.provider == "openrouter" && !e.secret_key.is_empty())
            })?
        })
}

/// The first registered local endpoint, used as a last-resort target when
/// every cloud path has failed.
pub fn first_local(endpoints: &[Endpoint]) -> Option<&Endpoint> {
    endpoints.iter().find(|e| e.is_local() && !e.url.is_empty())
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    fn ep(id: &str, provider: &str, model: &str, quality: f32) -> Endpoint {
        Endpoint {
            id: id.into(),
            url: format!("http://{id}"),
            provider: provider.into(),
            model: model.into(),
            status: "unknown".into(),
            requires_auth: false,
            secret_key: String::new(),
            health_checked_at: String::new(),
            quality_score: quality,
            quantization_level: String::new(),
        }
    }

    #[test]
    fn the_default_pick_prefers_a_local_endpoint() {
        let eps = vec![ep("or", "openrouter", "x", 0.99), ep("oll", "ollama", "y", 0.5)];
        assert_eq!(pick_default(&eps).unwrap().id, "oll");
    }

    #[test]
    fn quality_breaks_ties_within_a_tier() {
        let eps = vec![ep("a", "ollama", "x", 0.5), ep("b", "ollama", "y", 0.9)];
        assert_eq!(pick_default(&eps).unwrap().id, "b");
    }

    #[test]
    fn an_empty_registry_yields_no_default() {
        assert!(pick_default(&[]).is_none());
    }

    #[test]
    fn a_slugged_model_routes_through_a_keyed_openrouter_endpoint() {
        let mut or = ep("or", "openrouter", "something-else", 0.0);
        or.secret_key = "sk-or-v1-x".into();
        let eps = vec![ep("oll", "ollama", "qwen3:4b", 0.0), or];
        assert_eq!(find_serving(&eps, "google/gemini-2.0-flash").unwrap().id, "or");
    }

    #[test]
    fn a_keyless_openrouter_endpoint_does_not_claim_slugged_models() {
        let eps = vec![ep("or", "openrouter", "x", 0.0)];
        assert!(find_serving(&eps, "google/gemini-2.0-flash").is_none());
    }

    #[test]
    fn an_exact_model_match_wins_over_the_slug_rule() {
        let eps = vec![ep("tt", "openai_compat", "Qwen/Qwen3-32B", 0.0)];
        assert_eq!(find_serving(&eps, "Qwen/Qwen3-32B").unwrap().id, "tt");
    }

    #[test]
    fn first_local_skips_cloud_endpoints() {
        let eps = vec![ep("or", "openrouter", "x", 0.0), ep("oll", "ollama", "y", 0.0)];
        assert_eq!(first_local(&eps).unwrap().id, "oll");
        assert!(first_local(&[ep("or", "openrouter", "x", 0.0)]).is_none());
    }
}
