//! `complete` — one inference request, and everything hex does when it fails.
//!
//! Lifted out of `hex-nexus/src/routes/inference.rs::inference_complete`
//! (lines 66–1099) per ADR-2608241500 P2.4. That function was an axum handler
//! with 18 `State(...)` reads; this is a plain async function over
//! [`InferenceConfig`], callable in-process with no daemon and no HTTP hop.
//!
//! # What the 18 State reads became
//!
//! | Reads | Was | Now |
//! |---|---|---|
//! | 5 | `state.inference_stdb` — provider registry + logging | `~/.hex/inference-servers.json`; logging dropped |
//! | 4 | `state.rate_limiter` | dropped (ADR) |
//! | 3 | `state.spacetime_secrets` — vault lookup | `std::env::var`, via [`Endpoint::resolve_secret`] |
//! | 5 | `state.openrouter_api_key` / `anthropic_api_key` | env, via [`InferenceConfig`] |
//! | 2 | `state.fingerprints` — ACI injection | [`CompleteRequest::aci_block`], supplied by the caller |
//!
//! Moving ACI injection to the caller is a deliberate layering change, not a
//! drop. Assembling context is `hex-exec` and `hex-graph`'s job — context
//! quality is the stated differentiator of the single-loop thesis — and this
//! crate's job is to place the call. hex-infer no longer needs a fingerprint
//! store to know what a fingerprint is.
//!
//! Also dropped, per the ADR: LoRA idiom-expert attachment (ADR-2606161300 is
//! Proposed, not built), the SpacetimeDB `inference_log` write, and the
//! rate limiter.
//!
//! # The fallback chain, in order
//!
//! 1. **Tools fast-path** — when the request carries tool schemas, walk
//!    tools-capable endpoints local-first and return the first success.
//! 2. **Matched endpoint** — the endpoint serving the requested model.
//! 3. **Retries on that endpoint** — cloud transients retry once after 5s;
//!    a local 503 means "still loading the weights" and gets exponential
//!    backoff with jitter, three attempts; a 429 sleeps 10s first.
//! 4. **Registered `:free` OpenRouter providers**, 2s apart.
//! 5. **A chain of free models** on a bare OpenRouter key, best first.
//! 6. **Any local endpoint**, because a local model beats no answer.
//! 7. **Anthropic direct**, if a real `sk-ant-` key exists.
//!
//! A 401 never enters this chain. Bad credentials are a hard failure: retrying
//! burns the whole budget and ends with an error that names the wrong provider.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::InferenceConfig;
use crate::endpoint::Endpoint;
use crate::routing;
use crate::transport;

/// Default generation cap when the caller does not set one.
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Seconds to wait before retrying a cloud endpoint that returned a transient error.
const CLOUD_RETRY_SLEEP_SECS: u64 = 5;
/// Seconds to wait after a rate-limit response before trying free providers.
const RATE_LIMIT_SLEEP_SECS: u64 = 10;
/// Minimum gap between free-provider attempts. Without it, rapid-fire requests
/// burn the per-minute window before any candidate can succeed.
const FREE_CANDIDATE_GAP_SECS: u64 = 2;
/// Local backoff: first sleep, doubling, and its ceiling.
const LOCAL_BACKOFF_START_MS: u64 = 5_000;
const LOCAL_BACKOFF_CAP_MS: u64 = 60_000;
const LOCAL_BACKOFF_ATTEMPTS: u8 = 3;

/// One completion request.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CompleteRequest {
    /// Model id. `None` means "whatever the best registered endpoint serves".
    pub model: Option<String>,
    /// Messages in OpenAI-compatible form: `[{role, content}]`.
    pub messages: Vec<serde_json::Value>,
    /// System prompt, prepended as a system message.
    #[serde(default)]
    pub system: Option<String>,
    /// Generation cap.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Tool schemas in Anthropic form. When present and non-empty, the tools
    /// fast-path runs first.
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    /// Architecture-context block to prepend to the system prompt
    /// (ADR-2026-03-30-1200). The caller assembles this; see the module docs.
    #[serde(default)]
    pub aci_block: Option<String>,
}

fn default_max_tokens() -> u32 {
    DEFAULT_MAX_TOKENS
}

/// A successful completion.
#[derive(Debug, Clone, Serialize)]
pub struct Completion {
    pub content: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Tool calls, when the model emitted any. Always present (possibly empty)
    /// so callers can parse one shape.
    #[serde(default)]
    pub tool_calls: Vec<serde_json::Value>,
    /// Which provider family answered.
    pub provider: String,
    /// OpenRouter's reported cost in USD, when it reported one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openrouter_cost_usd: Option<String>,
}

impl Completion {
    /// The exact JSON shape `POST /api/inference/complete` returned.
    ///
    /// `hex-exec/src/simple_agent.rs` parses `content`, `tool_calls`, `model`,
    /// `input_tokens` and `output_tokens` off this object. Keeping the shape
    /// byte-identical is what lets P2.5 swap the HTTP call for a library call
    /// without touching a single parser.
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = json!({
            "content": self.content,
            "model": self.model,
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "tool_calls": self.tool_calls,
            "provider": self.provider,
        });
        if let Some(ref cost) = self.openrouter_cost_usd {
            v["openrouter_cost_usd"] = json!(cost);
        }
        v
    }
}

/// Why a completion could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum CompleteError {
    /// Bad credentials. Never retried — see the module docs.
    #[error("authentication failed for {provider}: {detail}")]
    Unauthorized { provider: String, detail: String },
    /// The outer deadline expired.
    #[error("inference exceeded the {secs}s deadline")]
    Timeout { secs: u64 },
    /// Every path in the fallback chain failed.
    #[error("all inference providers failed: {0}")]
    AllProvidersFailed(String),
    /// Nothing was configured to try.
    #[error("{0}")]
    NotConfigured(String),
}

/// Run one completion, with the configured outer deadline.
pub async fn complete(
    request: CompleteRequest,
    cfg: &InferenceConfig,
) -> Result<Completion, CompleteError> {
    let secs = cfg.timeout_secs;
    match tokio::time::timeout(Duration::from_secs(secs), complete_inner(request, cfg)).await {
        Ok(result) => result,
        Err(_) => {
            tracing::error!(secs, "inference timed out");
            Err(CompleteError::Timeout { secs })
        }
    }
}

async fn complete_inner(
    request: CompleteRequest,
    cfg: &InferenceConfig,
) -> Result<Completion, CompleteError> {
    let messages = build_messages(&request);
    let requested = request.model.as_deref().map(str::trim).filter(|m| !m.is_empty());

    // 1. Tools fast-path.
    if let Some(tools) = request.tools.as_ref().filter(|t| !t.is_empty()) {
        if let Some(done) = tools_fast_path(cfg, &messages, tools, requested).await? {
            return Ok(done);
        }
    }

    // 2. The endpoint that serves the requested model, or the best default.
    let endpoint = resolve_endpoint(cfg, requested);

    let result = match endpoint {
        Some(ep) => dispatch_with_fallbacks(cfg, ep, &messages, requested).await,
        None => key_only_fallback(cfg, &messages, requested).await,
    };

    result.map(|(r, provider)| completion_from(r, provider, Vec::new()))
}

/// Prepend the system prompt, merged with the caller's architecture-context
/// block when one is supplied.
fn build_messages(request: &CompleteRequest) -> Vec<serde_json::Value> {
    let system = match (request.aci_block.as_deref(), request.system.as_deref()) {
        (Some(aci), Some(sys)) if !sys.is_empty() => Some(format!("{aci}\n\n---\n\n{sys}")),
        (Some(aci), _) => Some(aci.to_string()),
        (None, Some(sys)) if !sys.is_empty() => Some(sys.to_string()),
        _ => None,
    };
    let mut messages = request.messages.clone();
    if let Some(system) = system {
        messages.insert(0, json!({ "role": "system", "content": system }));
    }
    messages
}

fn completion_from(
    r: transport::InferenceResult,
    provider: String,
    tool_calls: Vec<serde_json::Value>,
) -> Completion {
    let (content, model, input_tokens, output_tokens, cost) = r;
    Completion {
        content,
        model,
        input_tokens,
        output_tokens,
        tool_calls,
        provider,
        openrouter_cost_usd: (!cost.is_empty()).then_some(cost),
    }
}

/// Walk tools-capable endpoints, local-first, and return the first success.
///
/// `Ok(None)` means "no candidate worked" — the caller falls through to the
/// no-tools chain, where the text-mode parser still gets a chance to surface
/// structured output from prose. This replaced a hardcoded OpenRouter path
/// that burned credits while a local Ollama sat idle (observed 2026-05-21:
/// six tasks failed with "insufficient credits" against an idle localhost).
async fn tools_fast_path(
    cfg: &InferenceConfig,
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    requested: Option<&str>,
) -> Result<Option<Completion>, CompleteError> {
    let mut candidates = routing::candidates_for_tools(&cfg.endpoints, requested);

    // Always append a synthetic OpenRouter endpoint as last resort, unless a
    // first-class one is already queued.
    if let Some(key) = cfg.openrouter_api_key.as_deref() {
        if !routing::has_openrouter(&candidates) {
            let model = routing::normalize_for_openrouter(
                requested.unwrap_or("anthropic/claude-haiku-4.5"),
            );
            candidates.push(routing::openrouter_from_key(
                "openrouter-env-fastpath",
                key,
                &model,
            ));
        }
    }

    let mut last_err = String::new();
    for candidate in &candidates {
        let mut ep = candidate.clone();
        ep.honour_requested_model(requested);
        if !ep.resolve_secret() {
            tracing::warn!(
                key = %candidate.secret_key,
                "tools fast-path: no environment value for this key reference; skipping candidate"
            );
            continue;
        }
        tracing::debug!(provider = %ep.provider, model = %ep.model, "tools fast-path: trying candidate");

        match transport::call_inference_endpoint_with_tools(&ep, messages, tools).await {
            Ok((result, tool_calls)) => {
                tracing::info!(
                    provider = %ep.provider,
                    model = %result.1,
                    tool_calls = tool_calls.len(),
                    "inference OK (tools fast-path)"
                );
                return Ok(Some(completion_from(result, ep.provider.clone(), tool_calls)));
            }
            Err(e) if is_auth_failure(&e) => {
                return Err(CompleteError::Unauthorized {
                    provider: ep.provider.clone(),
                    detail: e,
                });
            }
            Err(e) => {
                tracing::warn!(provider = %ep.provider, error = %e, "tools fast-path: candidate failed; trying next");
                last_err = e;
            }
        }
    }

    if !candidates.is_empty() {
        tracing::warn!(
            last_err = %last_err,
            candidates = candidates.len(),
            "tools fast-path: all candidates exhausted — falling through to the no-tools chain"
        );
    }
    Ok(None)
}

/// The endpoint to try first.
///
/// When a model is named but no registered endpoint serves it, this yields
/// `None` on purpose so the key-based path handles it. Routing to an unrelated
/// provider — an offline Ollama, say — wastes the budget and times out.
fn resolve_endpoint(cfg: &InferenceConfig, requested: Option<&str>) -> Option<Endpoint> {
    match requested {
        None => routing::pick_default(&cfg.endpoints).cloned(),
        Some(model) => {
            if let Some(ep) = routing::find_serving(&cfg.endpoints, model) {
                let mut ep = ep.clone();
                ep.model = if ep.provider == "openrouter" {
                    routing::normalize_for_openrouter(model)
                } else {
                    model.to_string()
                };
                return Some(ep);
            }
            // A local-shaped id (no vendor slug) may still be servable by any
            // registered local endpoint, which serves whatever is pulled.
            if !model.contains('/') {
                if let Some(local) = routing::first_local(&cfg.endpoints) {
                    tracing::debug!(model, provider = %local.id, "routing to a local endpoint");
                    let mut ep = local.clone();
                    ep.model = model.to_string();
                    ep.requires_auth = false;
                    ep.secret_key = String::new();
                    return Some(ep);
                }
            }
            tracing::debug!(model, "no registered endpoint serves this model — using the key-based path");
            None
        }
    }
}

/// Call the chosen endpoint, then walk the fallback chain if it fails.
async fn dispatch_with_fallbacks(
    cfg: &InferenceConfig,
    mut ep: Endpoint,
    messages: &[serde_json::Value],
    requested: Option<&str>,
) -> Result<(transport::InferenceResult, String), CompleteError> {
    if !ep.resolve_secret() {
        return Err(CompleteError::NotConfigured(format!(
            "endpoint '{}' needs the environment variable '{}', which is unset",
            ep.id, ep.secret_key
        )));
    }

    let first = transport::call_inference_endpoint(&ep, messages).await;
    let err = match first {
        Ok(r) => return Ok((r, ep.provider.clone())),
        Err(e) if is_auth_failure(&e) => {
            tracing::error!(provider = %ep.provider, error = %e, "authentication failed — not retrying");
            return Err(CompleteError::Unauthorized {
                provider: ep.provider,
                detail: e,
            });
        }
        Err(e) => e,
    };

    if !is_retryable(&err) {
        tracing::warn!(provider = %ep.provider, error = %err, "endpoint failed — going straight to key fallbacks");
        return key_only_fallback(cfg, messages, requested).await;
    }

    // A cloud transient (parse error or 5xx) usually clears on its own.
    if is_cloud_transient(&err, &ep) {
        tracing::warn!(provider = %ep.provider, model = %ep.model, error = %err,
            "transient error — sleeping {CLOUD_RETRY_SLEEP_SECS}s then retrying the same endpoint");
        tokio::time::sleep(Duration::from_secs(CLOUD_RETRY_SLEEP_SECS)).await;
        if let Ok(r) = transport::call_inference_endpoint(&ep, messages).await {
            return Ok((r, ep.provider.clone()));
        }
    }

    // A local 503 means the weights are still loading, not that anything broke.
    if ep.is_local() && (err.contains("connection:") || err.contains("503")) {
        if let Some(r) = local_backoff_retry(&ep, messages).await {
            return Ok((r, ep.provider.clone()));
        }
    }

    if is_rate_limited(&err) {
        tracing::warn!(provider = %ep.provider, "rate limited — sleeping {RATE_LIMIT_SLEEP_SECS}s before free providers");
        tokio::time::sleep(Duration::from_secs(RATE_LIMIT_SLEEP_SECS)).await;
    }

    ep.secret_key.clear(); // no longer needed; keep the key out of later logs
    free_provider_chain(cfg, messages, requested, &err).await
}

/// Retry a local endpoint with exponential backoff plus jitter.
async fn local_backoff_retry(
    ep: &Endpoint,
    messages: &[serde_json::Value],
) -> Option<transport::InferenceResult> {
    let mut backoff_ms = LOCAL_BACKOFF_START_MS;
    for attempt in 1..=LOCAL_BACKOFF_ATTEMPTS {
        let sleep_ms = backoff_ms + jitter_ms();
        tracing::warn!(
            provider = %ep.provider, model = %ep.model, attempt, sleep_ms,
            "local model not ready — backing off before retry"
        );
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        match transport::call_inference_endpoint(ep, messages).await {
            Ok(r) => return Some(r),
            Err(e) => tracing::warn!(attempt, error = %e, "local retry failed"),
        }
        backoff_ms = (backoff_ms * 2).min(LOCAL_BACKOFF_CAP_MS);
    }
    tracing::warn!(provider = %ep.provider, "all local retries exhausted");
    None
}

/// Sub-second clock nanos as a cheap jitter source — no RNG dependency for
/// something this crude.
fn jitter_ms() -> u64 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        % 2_000) as u64
}

/// Registered `:free` providers → a chain of free models on a bare key → any
/// local endpoint → Anthropic direct.
async fn free_provider_chain(
    cfg: &InferenceConfig,
    messages: &[serde_json::Value],
    requested: Option<&str>,
    original_err: &str,
) -> Result<(transport::InferenceResult, String), CompleteError> {
    // Registered :free OpenRouter providers.
    for fp in cfg
        .endpoints
        .iter()
        .filter(|e| e.provider == "openrouter" && e.model.contains(":free"))
    {
        tokio::time::sleep(Duration::from_secs(FREE_CANDIDATE_GAP_SECS)).await;
        let mut ep = fp.clone();
        if !ep.resolve_secret() {
            tracing::warn!(key = %fp.secret_key, ":free provider key unresolved — skipping");
            continue;
        }
        match transport::call_inference_endpoint(&ep, messages).await {
            Ok(r) => return Ok((r, ep.provider)),
            Err(e) => {
                tracing::warn!(model = %ep.model, error = %e, ":free provider failed — trying next");
                // Back off for rate limits; policy and 404 errors never recover.
                if is_rate_limited(&e) && !is_permanent(&e) {
                    tokio::time::sleep(Duration::from_secs(4)).await;
                }
            }
        }
    }

    // A chain of free models on a bare OpenRouter key.
    if let Some(key) = cfg.openrouter_api_key.as_deref() {
        for model in routing::FREE_MODEL_CHAIN {
            tokio::time::sleep(Duration::from_secs(FREE_CANDIDATE_GAP_SECS)).await;
            tracing::info!(model, "trying a free OpenRouter model");
            let ep = routing::openrouter_from_key("openrouter-key-free-fallback", key, model);
            if let Ok(r) = transport::call_inference_endpoint(&ep, messages).await {
                return Ok((r, ep.provider));
            }
        }
    }

    // Any local endpoint. When the free tier is exhausted, local inference is
    // the difference between the loop progressing and the loop sitting idle.
    if let Some(local) = routing::first_local(&cfg.endpoints) {
        let mut ep = local.clone();
        ep.model = local_fallback_model(cfg, requested);
        ep.requires_auth = false;
        ep.secret_key = String::new();
        tracing::info!(provider = %ep.id, model = %ep.model, "all cloud fallbacks failed — trying local");
        match transport::call_inference_endpoint(&ep, messages).await {
            Ok(r) => return Ok((r, ep.provider)),
            Err(e) => tracing::warn!(error = %e, "local fallback also failed"),
        }
    }

    // Anthropic direct.
    if let Some(key) = cfg.anthropic_api_key.as_deref() {
        tracing::info!("all providers exhausted — falling back to Anthropic direct");
        if let Ok(r) = transport::call_anthropic(key, messages).await {
            return Ok((r, "anthropic".to_string()));
        }
    }

    Err(CompleteError::AllProvidersFailed(original_err.to_string()))
}

/// Pick a model the local server can actually serve.
///
/// Priority: the requested model when it is local-shaped; then the configured
/// T2 model, but only when *it* is local — T2 may be a cloud id like
/// `Qwen/Qwen3-32B` that Ollama 404s on; then a known local default. Reading
/// T2 also keeps hex from asking for a model larger than the GPU can hold.
fn local_fallback_model(cfg: &InferenceConfig, requested: Option<&str>) -> String {
    requested
        .filter(|m| !m.contains('/'))
        .map(str::to_string)
        .or_else(|| cfg.local_t2_model().map(str::to_string))
        .unwrap_or_else(|| "qwen2.5-coder:32b".to_string())
}

/// No registered endpoint matched — try the bare keys.
async fn key_only_fallback(
    cfg: &InferenceConfig,
    messages: &[serde_json::Value],
    requested: Option<&str>,
) -> Result<(transport::InferenceResult, String), CompleteError> {
    if let Some(key) = cfg.anthropic_api_key.as_deref() {
        match transport::call_anthropic(key, messages).await {
            Ok(r) => return Ok((r, "anthropic".to_string())),
            Err(e) if is_auth_failure(&e) => {
                return Err(CompleteError::Unauthorized {
                    provider: "anthropic".into(),
                    detail: e,
                })
            }
            Err(e) => tracing::warn!(error = %e, "Anthropic direct failed — trying OpenRouter"),
        }
    }

    if let Some(key) = cfg.openrouter_api_key.as_deref() {
        let model =
            routing::normalize_for_openrouter(&routing::resolve_free_model(requested));
        tracing::info!(model = %model, "using the OpenRouter key fallback");
        let ep = routing::openrouter_from_key("openrouter-key-fallback", key, &model);
        return match transport::call_inference_endpoint(&ep, messages).await {
            Ok(r) => Ok((r, ep.provider)),
            Err(e) if is_auth_failure(&e) => Err(CompleteError::Unauthorized {
                provider: "openrouter".into(),
                detail: e,
            }),
            Err(e) => Err(CompleteError::AllProvidersFailed(e)),
        };
    }

    Err(CompleteError::NotConfigured(
        "no inference endpoints registered in ~/.hex/inference-servers.json, and neither \
         ANTHROPIC_API_KEY nor OPENROUTER_API_KEY is set"
            .into(),
    ))
}

// ── Error classification ──────────────────────────────────────────────────
//
// The transport layer returns `String` errors carrying the upstream status
// text. These predicates are the only place that text is interpreted.

/// HTTP 401. Never retried.
pub fn is_auth_failure(e: &str) -> bool {
    e.contains("401") || e.contains("Unauthorized")
}

/// Worth walking the fallback chain for.
pub fn is_retryable(e: &str) -> bool {
    const MARKERS: &[&str] = &[
        "insufficient credits",
        "402",
        "rate limited",
        "429",
        "parse:",
        "500",
        "503",
        "404",
        "No endpoints",
        "data policy",
        "connection:",
        "null content",
    ];
    MARKERS.iter().any(|m| e.contains(m))
}

/// A cloud hiccup that usually clears within seconds. A local 503 is excluded:
/// that is a model loading, which needs backoff, not an immediate retry.
fn is_cloud_transient(e: &str, ep: &Endpoint) -> bool {
    !ep.is_local()
        && (e.contains("parse:") || e.contains("500") || e.contains("503"))
        && !is_permanent(e)
}

fn is_rate_limited(e: &str) -> bool {
    e.contains("rate limited") || e.contains("429")
}

/// Errors that will not improve with time.
fn is_permanent(e: &str) -> bool {
    e.contains("data policy")
        || e.contains("guardrail")
        || e.contains("No endpoints")
        || (e.contains("404") && !e.contains("rate"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(provider: &str) -> Endpoint {
        Endpoint {
            id: "e".into(),
            url: "http://x".into(),
            provider: provider.into(),
            model: "m".into(),
            status: "unknown".into(),
            requires_auth: false,
            secret_key: String::new(),
            health_checked_at: String::new(),
            quality_score: 0.0,
            quantization_level: String::new(),
        }
    }

    #[test]
    fn a_system_prompt_is_prepended_as_a_system_message() {
        let req = CompleteRequest {
            messages: vec![json!({"role": "user", "content": "hi"})],
            system: Some("be terse".into()),
            ..Default::default()
        };
        let m = build_messages(&req);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[0]["content"], "be terse");
    }

    #[test]
    fn an_aci_block_is_merged_above_the_system_prompt() {
        let req = CompleteRequest {
            messages: vec![json!({"role": "user", "content": "hi"})],
            system: Some("be terse".into()),
            aci_block: Some("ARCH: hexagonal".into()),
            ..Default::default()
        };
        let m = build_messages(&req);
        let sys = m[0]["content"].as_str().unwrap();
        assert!(sys.starts_with("ARCH: hexagonal"));
        assert!(sys.ends_with("be terse"));
    }

    #[test]
    fn an_aci_block_alone_becomes_the_system_prompt() {
        let req = CompleteRequest {
            messages: vec![json!({"role": "user", "content": "hi"})],
            aci_block: Some("ARCH: hexagonal".into()),
            ..Default::default()
        };
        assert_eq!(build_messages(&req)[0]["content"], "ARCH: hexagonal");
    }

    #[test]
    fn no_system_prompt_means_no_extra_message() {
        let req = CompleteRequest {
            messages: vec![json!({"role": "user", "content": "hi"})],
            ..Default::default()
        };
        assert_eq!(build_messages(&req).len(), 1);
    }

    #[test]
    fn an_empty_system_prompt_is_not_prepended() {
        let req = CompleteRequest {
            messages: vec![json!({"role": "user", "content": "hi"})],
            system: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(build_messages(&req).len(), 1);
    }

    /// The wire shape hex-exec/src/simple_agent.rs parses. If this test
    /// changes, P2.5's in-process swap stops being transparent.
    #[test]
    fn the_json_shape_matches_the_old_http_response() {
        let c = Completion {
            content: String::new(),
            model: "anthropic/claude-haiku-4.5".into(),
            input_tokens: 100,
            output_tokens: 20,
            tool_calls: vec![json!({"id": "call_abc", "type": "function"})],
            provider: "openrouter".into(),
            openrouter_cost_usd: Some("0.00001234".into()),
        };
        let v = c.to_json();
        assert_eq!(v["content"], "");
        assert_eq!(v["model"], "anthropic/claude-haiku-4.5");
        assert_eq!(v["input_tokens"], 100);
        assert_eq!(v["output_tokens"], 20);
        assert_eq!(v["tool_calls"][0]["id"], "call_abc");
        assert_eq!(v["openrouter_cost_usd"], "0.00001234");
    }

    #[test]
    fn a_zero_cost_completion_omits_the_cost_field() {
        let c = completion_from(
            ("x".into(), "m".into(), 1, 2, String::new()),
            "ollama".into(),
            vec![],
        );
        assert!(c.openrouter_cost_usd.is_none());
        assert!(c.to_json().get("openrouter_cost_usd").is_none());
        // tool_calls is always present, even when empty.
        assert!(c.to_json()["tool_calls"].is_array());
    }

    #[test]
    fn auth_failures_are_recognised_and_are_not_retryable() {
        for e in ["HTTP 401: bad key", "Unauthorized"] {
            assert!(is_auth_failure(e));
            assert!(!is_retryable(e), "401 must never enter the fallback chain");
        }
    }

    #[test]
    fn the_documented_retryable_markers_all_match() {
        for e in [
            "openrouter: insufficient credits",
            "HTTP 402:",
            "OpenRouter: rate limited",
            "HTTP 429",
            "parse: expected value",
            "HTTP 500",
            "HTTP 503",
            "HTTP 404",
            "No endpoints found",
            "data policy",
            "connection: refused",
            "null content: model returned null",
        ] {
            assert!(is_retryable(e), "{e} should be retryable");
        }
        assert!(!is_retryable("some unmodelled failure"));
    }

    #[test]
    fn a_local_503_is_not_treated_as_a_cloud_transient() {
        assert!(!is_cloud_transient("HTTP 503", &ep("ollama")));
        assert!(is_cloud_transient("HTTP 503", &ep("openrouter")));
    }

    #[test]
    fn permanent_errors_are_excluded_from_the_cloud_retry() {
        assert!(!is_cloud_transient("HTTP 404 data policy", &ep("openrouter")));
        assert!(is_permanent("HTTP 404: no such model"));
        assert!(!is_permanent("rate limited, 404 later"));
    }

    #[test]
    fn the_local_fallback_model_prefers_a_local_shaped_request() {
        let cfg = InferenceConfig::default();
        assert_eq!(local_fallback_model(&cfg, Some("qwen3:4b")), "qwen3:4b");
    }

    #[test]
    fn a_cloud_shaped_request_does_not_become_the_local_fallback_model() {
        let mut cfg = InferenceConfig::default();
        cfg.tier_models.insert("t2".into(), "qwen2.5-coder:14b".into());
        assert_eq!(local_fallback_model(&cfg, Some("Qwen/Qwen3-32B")), "qwen2.5-coder:14b");
    }

    #[test]
    fn a_cloud_t2_is_skipped_for_the_built_in_local_default() {
        let mut cfg = InferenceConfig::default();
        cfg.tier_models.insert("t2".into(), "Qwen/Qwen3-32B".into());
        assert_eq!(local_fallback_model(&cfg, None), "qwen2.5-coder:32b");
    }

    fn cfg_with(endpoints: Vec<Endpoint>) -> InferenceConfig {
        InferenceConfig {
            endpoints,
            timeout_secs: 5,
            ..Default::default()
        }
    }

    fn named(id: &str, provider: &str, model: &str) -> Endpoint {
        let mut e = ep(provider);
        e.id = id.into();
        e.model = model.into();
        e
    }

    #[test]
    fn with_no_model_requested_the_best_default_endpoint_is_chosen() {
        let cfg = cfg_with(vec![
            named("or", "openrouter", "x"),
            named("oll", "ollama", "qwen3:4b"),
        ]);
        assert_eq!(resolve_endpoint(&cfg, None).unwrap().id, "oll");
    }

    #[test]
    fn a_requested_model_selects_the_endpoint_that_serves_it() {
        let cfg = cfg_with(vec![
            named("oll", "ollama", "gemma4-12b"),
            named("tt", "openai_compat", "Qwen/Qwen3-32B"),
        ]);
        let chosen = resolve_endpoint(&cfg, Some("Qwen/Qwen3-32B")).unwrap();
        assert_eq!(chosen.id, "tt");
        assert_eq!(chosen.model, "Qwen/Qwen3-32B");
    }

    /// A local server serves whatever has been pulled, so an unregistered
    /// local-shaped id still routes there rather than falling off the list.
    #[test]
    fn an_unregistered_local_shaped_model_still_routes_to_a_local_endpoint() {
        let cfg = cfg_with(vec![named("oll", "ollama", "gemma4-12b")]);
        let chosen = resolve_endpoint(&cfg, Some("qwen3:4b")).unwrap();
        assert_eq!(chosen.id, "oll");
        assert_eq!(chosen.model, "qwen3:4b");
    }

    /// Routing a cloud id to an unrelated provider wastes the whole budget and
    /// then times out, so no endpoint is better than the wrong one.
    #[test]
    fn an_unservable_cloud_model_yields_no_endpoint() {
        let cfg = cfg_with(vec![named("oll", "ollama", "gemma4-12b")]);
        assert!(resolve_endpoint(&cfg, Some("some-vendor/some-model")).is_none());
    }

    #[test]
    fn an_openrouter_endpoint_gets_the_vendor_namespaced_model_id() {
        let mut or = named("or", "openrouter", "claude-haiku-4-5-20251001");
        or.secret_key = "sk-or-v1-x".into();
        let cfg = cfg_with(vec![or]);
        let chosen = resolve_endpoint(&cfg, Some("claude-haiku-4-5-20251001")).unwrap();
        assert_eq!(chosen.model, "anthropic/claude-haiku-4.5");
    }

    #[tokio::test]
    async fn an_empty_configuration_reports_what_is_missing_rather_than_hanging() {
        let cfg = InferenceConfig {
            timeout_secs: 5,
            ..Default::default()
        };
        let err = complete(CompleteRequest::default(), &cfg).await.unwrap_err();
        assert!(matches!(err, CompleteError::NotConfigured(_)));
        assert!(err.to_string().contains("inference-servers.json"));
    }
}
