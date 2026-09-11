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


// ── task tier classification ─────────────────────────────────────────────────
//
// Salvaged from `hex-nexus/src/orchestration/workplan_executor.rs::
// classify_task_tier` and the nine classifier cases in
// `hex-nexus/tests/tier_routing.rs`, per ADR-2608241500 P4.2. The daemon did
// this classification on behalf of `hex plan execute`; once the daemon is gone
// the executor runs in-process and still needs it, and tier routing is the
// thing this crate is for.
//
// What did NOT come across: the router tests around `IRemoteRegistryPort` and
// `IAgentTransportPort` — placing a request on a remote inference host is the
// fleet model this ADR retires, and there is nothing left for them to assert.

/// Inference tier for one unit of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tier {
    /// Trivial edits: renames, typo fixes, comment changes. Fastest model.
    #[serde(rename = "T1", alias = "t1")]
    T1,
    /// Single function or test generation. Best local codegen model.
    #[serde(rename = "T2", alias = "t2")]
    T2,
    /// Multi-function, cross-file agentic work. Strong reasoning model.
    #[serde(rename = "T2.5", alias = "t2.5", alias = "T2_5", alias = "t2_5")]
    T2_5,
    /// Multi-file features. Frontier model only.
    #[serde(rename = "T3", alias = "t3")]
    T3,
}

impl Tier {
    /// The tier name as it appears in `.hex/project.json` and in workplans.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::T1 => "t1",
            Self::T2 => "t2",
            Self::T2_5 => "t2.5",
            Self::T3 => "t3",
        }
    }
}

/// What the classifier needs to know about a task.
///
/// Deliberately not the workplan's own `WorkplanTask`: this crate places
/// inference calls and must not take a position on the workplan schema. The
/// caller projects whatever it has onto these fields.
#[derive(Debug, Default, Clone)]
pub struct TaskShape<'a> {
    /// Explicit tier from the workplan. Wins over every heuristic.
    pub tier: Option<Tier>,
    /// `scaffold` / `transform` / `script` / `codegen` / `inference`.
    pub strategy_hint: Option<&'a str>,
    /// Role the task is assigned to, e.g. `hex-coder`, `planner`.
    pub agent: Option<&'a str>,
    /// Hexagonal layer: `domain`, `ports`, `primary`, `secondary`.
    pub layer: Option<&'a str>,
    /// How many other tasks this one waits on.
    pub dep_count: usize,
    /// Files the task creates or modifies.
    pub files: &'a [String],
    /// Task name and description, used for the design heuristic.
    pub name: &'a str,
    pub description: &'a str,
}

/// Route one task to a tier.
///
/// Priority: explicit `tier` > `strategy_hint` > design heuristic > agent role
/// > layer and dependency count.
///
/// Deliberately conservative in one direction: under-classifying (T3 work sent
/// to T2) costs a retry, while over-classifying (T1 work sent to T3) spends
/// frontier budget on a rename.
pub fn classify_tier(task: &TaskShape<'_>) -> Tier {
    if let Some(tier) = task.tier {
        return tier;
    }

    match task.strategy_hint.map(str::trim) {
        Some(h) if h.eq_ignore_ascii_case("scaffold") => return Tier::T1,
        Some(h) if h.eq_ignore_ascii_case("transform") => return Tier::T1,
        Some(h) if h.eq_ignore_ascii_case("script") => return Tier::T1,
        Some(h) if h.eq_ignore_ascii_case("codegen") => return Tier::T2,
        Some(h) if h.eq_ignore_ascii_case("inference") => return Tier::T2_5,
        _ => {}
    }

    // Front-end DESIGN work needs a reasoning model; standard codegen produces
    // rough, unstyled output (lesson:tier-routing-for-ui, 2026-05-31). Checked
    // before the role default so a coder building a Tailwind grid does not fall
    // through to T2.
    if is_ui_design(task) {
        return Tier::T2_5;
    }

    match task.agent.map(str::trim) {
        Some("planner" | "hex-planner") => return Tier::T2,
        Some("reviewer" | "hex-reviewer") => return Tier::T2,
        Some("integrator" | "hex-integrator") => return Tier::T2_5,
        _ => {}
    }

    match task.layer.map(str::trim) {
        Some("domain") | Some("ports") => Tier::T2,
        Some("primary") | Some("secondary") => {
            if task.dep_count >= 2 {
                Tier::T2_5
            } else {
                Tier::T2
            }
        }
        // Safe default: cheap to retry, expensive to over-spend.
        _ => Tier::T2,
    }
}

/// True when a task is front-end design work — visual, layout or styling.
///
/// Detected by the files it touches or by design vocabulary in its name and
/// description.
fn is_ui_design(task: &TaskShape<'_>) -> bool {
    const UI_EXT: [&str; 7] = [".tsx", ".jsx", ".vue", ".svelte", ".css", ".scss", ".html"];
    if task
        .files
        .iter()
        .any(|f| UI_EXT.iter().any(|e| f.to_ascii_lowercase().ends_with(e)))
    {
        return true;
    }
    const UI_KW: [&str; 9] = [
        "tailwind", "css", "stylesheet", " ui ", "frontend", "layout", "grid of", "component",
        "responsive",
    ];
    let hay = format!("{} {}", task.name, task.description).to_ascii_lowercase();
    UI_KW.iter().any(|k| hay.contains(k))
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

    // ── tier classification (salvaged from hex-nexus/tests/tier_routing.rs) ──

    fn task<'a>() -> TaskShape<'a> {
        TaskShape::default()
    }

    #[test]
    fn an_explicit_tier_overrides_every_heuristic() {
        let t = TaskShape { tier: Some(Tier::T3), layer: Some("domain"), ..task() };
        assert_eq!(classify_tier(&t), Tier::T3);
    }

    #[test]
    fn a_strategy_hint_beats_the_role_and_layer_heuristics() {
        let t = TaskShape {
            strategy_hint: Some("scaffold"),
            agent: Some("integrator"),
            layer: Some("primary"),
            dep_count: 5,
            ..task()
        };
        assert_eq!(classify_tier(&t), Tier::T1);
    }

    #[test]
    fn every_documented_strategy_hint_routes() {
        for (hint, want) in [
            ("scaffold", Tier::T1),
            ("transform", Tier::T1),
            ("script", Tier::T1),
            ("codegen", Tier::T2),
            ("inference", Tier::T2_5),
            ("INFERENCE", Tier::T2_5),
            ("  codegen  ", Tier::T2),
        ] {
            let t = TaskShape { strategy_hint: Some(hint), ..task() };
            assert_eq!(classify_tier(&t), want, "hint {hint:?}");
        }
    }

    #[test]
    fn agent_roles_map_to_their_tiers() {
        for (agent, want) in [
            ("planner", Tier::T2),
            ("hex-planner", Tier::T2),
            ("reviewer", Tier::T2),
            ("hex-reviewer", Tier::T2),
            ("integrator", Tier::T2_5),
            ("hex-integrator", Tier::T2_5),
        ] {
            let t = TaskShape { agent: Some(agent), ..task() };
            assert_eq!(classify_tier(&t), want, "agent {agent:?}");
        }
    }

    #[test]
    fn contract_layers_map_to_t2() {
        for layer in ["domain", "ports"] {
            let t = TaskShape { layer: Some(layer), ..task() };
            assert_eq!(classify_tier(&t), Tier::T2, "layer {layer:?}");
        }
    }

    #[test]
    fn an_adapter_escalates_only_once_it_has_dependencies() {
        let few = TaskShape { layer: Some("secondary"), dep_count: 1, ..task() };
        assert_eq!(classify_tier(&few), Tier::T2);
        let many = TaskShape { layer: Some("primary"), dep_count: 2, ..task() };
        assert_eq!(classify_tier(&many), Tier::T2_5);
    }

    #[test]
    fn an_unknown_layer_takes_the_safe_default() {
        assert_eq!(classify_tier(&task()), Tier::T2);
        let t = TaskShape { layer: Some("something-else"), ..task() };
        assert_eq!(classify_tier(&t), Tier::T2);
    }

    #[test]
    fn design_work_escalates_by_the_files_it_touches() {
        for ext in [".tsx", ".jsx", ".vue", ".svelte", ".css", ".scss", ".html"] {
            let files = vec![format!("app/Widget{ext}")];
            let t = TaskShape { files: &files, layer: Some("primary"), ..task() };
            assert_eq!(classify_tier(&t), Tier::T2_5, "extension {ext}");
        }
    }

    #[test]
    fn design_work_escalates_by_vocabulary() {
        let t = TaskShape {
            name: "Build the responsive grid of cards",
            agent: Some("hex-coder"),
            ..task()
        };
        assert_eq!(classify_tier(&t), Tier::T2_5);
    }

    #[test]
    fn ordinary_backend_work_is_not_mistaken_for_design() {
        let files = vec!["hex-core/src/domain/workplan.rs".to_string()];
        let t = TaskShape {
            files: &files,
            name: "Add a phase gate field",
            description: "Extend the workplan schema.",
            layer: Some("domain"),
            ..task()
        };
        assert_eq!(classify_tier(&t), Tier::T2);
    }

    #[test]
    fn tier_names_round_trip_through_json_including_the_dotted_one() {
        for (json, tier) in [
            ("\"T1\"", Tier::T1),
            ("\"t1\"", Tier::T1),
            ("\"T2.5\"", Tier::T2_5),
            ("\"t2.5\"", Tier::T2_5),
            ("\"T3\"", Tier::T3),
        ] {
            let got: Tier = serde_json::from_str(json).expect(json);
            assert_eq!(got, tier, "parsing {json}");
        }
        assert_eq!(serde_json::to_string(&Tier::T2_5).unwrap(), "\"T2.5\"");
        assert_eq!(Tier::T2_5.as_str(), "t2.5");
    }

}
