//! Inference Gateway SpacetimeDB Module (ADR-035)
//!
//! Routes ALL LLM inference through SpacetimeDB. Agents write requests,
//! an external process (hex-nexus) subscribes, calls LLM APIs, and writes
//! responses back via `complete_inference` / `fail_inference`.
//!
//! Tables:
//!   - `inference_request` (public) — agents write requests here
//!   - `inference_response` (public) — responses written after LLM call
//!   - `inference_provider` (public) — registered LLM endpoints
//!   - `agent_budget` (public) — per-agent token budget enforcement
//!   - `inference_stream_chunk` (public) — streaming response chunks

use spacetimedb::{table, reducer, ReducerContext, Table};

// ─── Inference Request ──────────────────────────────────────────────────────

#[table(name = inference_request, public)]
#[derive(Clone, Debug)]
pub struct InferenceRequest {
    #[primary_key]
    #[auto_inc]
    pub request_id: u64,
    pub agent_id: String,
    /// Provider identifier: "anthropic", "minimax", "ollama", "vllm"
    pub provider: String,
    pub model: String,
    /// Serialized messages array (JSON)
    pub messages_json: String,
    /// Serialized tool definitions (JSON)
    pub tools_json: String,
    pub max_tokens: u32,
    /// Stored as string to avoid float precision issues
    pub temperature: String,
    /// 0 = disabled
    pub thinking_budget: u32,
    /// 0 = false, 1 = true (SpacetimeDB bool workaround)
    pub cache_control: u8,
    /// 0=low, 1=normal, 2=high, 3=critical
    pub priority: u8,
    /// "queued", "processing", "completed", "failed"
    pub status: String,
    /// ISO 8601 timestamp
    pub created_at: String,
}

// ─── Inference Response ─────────────────────────────────────────────────────

#[table(name = inference_response, public)]
#[derive(Clone, Debug)]
pub struct InferenceResponse {
    #[primary_key]
    #[auto_inc]
    pub response_id: u64,
    pub request_id: u64,
    pub agent_id: String,
    /// "completed", "failed", "rate_limited", "budget_exceeded"
    pub status: String,
    /// Serialized response content (JSON)
    pub content_json: String,
    pub model_used: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub latency_ms: u64,
    /// Stored as string for precision
    pub cost_usd: String,
    /// ISO 8601 timestamp
    pub created_at: String,
}

// ─── Inference Provider ─────────────────────────────────────────────────────

#[table(name = inference_provider, public)]
#[derive(Clone, Debug)]
pub struct InferenceProvider {
    /// Unique provider identifier
    #[primary_key]
    pub provider_id: String,
    /// "anthropic", "openai_compat", "ollama", "vllm"
    pub provider_type: String,
    pub base_url: String,
    /// Reference to secret_vault key, never plaintext
    pub api_key_ref: String,
    /// Available models + capabilities (JSON)
    pub models_json: String,
    pub rate_limit_rpm: u32,
    pub rate_limit_tpm: u64,
    /// Rolling window counter
    pub current_rpm: u32,
    pub current_tpm: u64,
    /// 0 = unhealthy, 1 = healthy
    pub healthy: u8,
    /// ISO 8601 timestamp
    pub last_health_check: String,
    pub avg_latency_ms: u64,
}

// ─── Agent Budget ───────────────────────────────────────────────────────────

#[table(name = agent_budget, public)]
#[derive(Clone, Debug)]
pub struct AgentBudget {
    #[primary_key]
    pub agent_id: String,
    pub total_budget_tokens: u64,
    pub used_tokens: u64,
    /// Stored as string for precision
    pub total_budget_usd: String,
    /// Stored as string for precision
    pub used_usd: String,
    pub max_single_request_tokens: u64,
    /// ISO 8601 timestamp
    pub updated_at: String,
}

// ─── Inference Stream Chunk ─────────────────────────────────────────────────

#[table(name = inference_stream_chunk, public)]
#[derive(Clone, Debug)]
pub struct InferenceStreamChunk {
    #[primary_key]
    #[auto_inc]
    pub chunk_id: u64,
    pub request_id: u64,
    pub agent_id: String,
    /// "text_delta", "tool_use_start", "input_json_delta", "message_stop"
    pub chunk_type: String,
    pub content: String,
    pub sequence: u32,
    /// ISO 8601 timestamp
    pub created_at: String,
}

// ─── Reducers ───────────────────────────────────────────────────────────────

/// Submit an inference request. Checks budget and rate limits before queuing.
#[reducer]
pub fn request_inference(
    ctx: &ReducerContext,
    agent_id: String,
    provider: String,
    model: String,
    messages_json: String,
    tools_json: String,
    max_tokens: u32,
    temperature: String,
    thinking_budget: u32,
    cache_control: u8,
    priority: u8,
    created_at: String,
) -> Result<(), String> {
    // 1. Check AgentBudget — reject if over budget
    if let Some(budget) = ctx.db.agent_budget().agent_id().find(&agent_id) {
        if budget.used_tokens + max_tokens as u64 > budget.total_budget_tokens {
            ctx.db.inference_response().insert(InferenceResponse {
                response_id: 0,
                request_id: 0,
                agent_id: agent_id.clone(),
                status: "budget_exceeded".to_string(),
                content_json: format!(
                    "{{\"error\":\"Budget exceeded: used {} + requested {} > limit {}\"}}",
                    budget.used_tokens, max_tokens, budget.total_budget_tokens
                ),
                model_used: model,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                latency_ms: 0,
                cost_usd: "0".to_string(),
                created_at: created_at.clone(),
            });
            return Ok(());
        }
    }

    // 2. Check InferenceProvider rate limit
    if let Some(prov) = ctx.db.inference_provider().provider_id().find(&provider) {
        if prov.current_rpm >= prov.rate_limit_rpm {
            ctx.db.inference_response().insert(InferenceResponse {
                response_id: 0,
                request_id: 0,
                agent_id: agent_id.clone(),
                status: "rate_limited".to_string(),
                content_json: format!(
                    "{{\"error\":\"Rate limit exceeded for provider '{}': {}/{}\"}}",
                    provider, prov.current_rpm, prov.rate_limit_rpm
                ),
                model_used: model,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                latency_ms: 0,
                cost_usd: "0".to_string(),
                created_at: created_at.clone(),
            });
            return Ok(());
        }

        // Increment provider's current_rpm
        ctx.db.inference_provider().provider_id().update(InferenceProvider {
            current_rpm: prov.current_rpm + 1,
            ..prov
        });
    }

    // 3. Insert request with status "queued"
    ctx.db.inference_request().insert(InferenceRequest {
        request_id: 0, // auto_inc
        agent_id,
        provider,
        model,
        messages_json,
        tools_json,
        max_tokens,
        temperature,
        thinking_budget,
        cache_control,
        priority,
        status: "queued".to_string(),
        created_at,
    });

    Ok(())
}

/// Mark a request as completed with response data.
#[reducer]
pub fn complete_inference(
    ctx: &ReducerContext,
    request_id: u64,
    content_json: String,
    model_used: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    latency_ms: u64,
    cost_usd: String,
    created_at: String,
) -> Result<(), String> {
    // 1. Update request status
    let request = ctx.db.inference_request().request_id().find(&request_id)
        .ok_or_else(|| format!("Request {} not found", request_id))?;

    ctx.db.inference_request().request_id().update(InferenceRequest {
        status: "completed".to_string(),
        ..request.clone()
    });

    // 2. Insert response
    ctx.db.inference_response().insert(InferenceResponse {
        response_id: 0, // auto_inc
        request_id,
        agent_id: request.agent_id.clone(),
        status: "completed".to_string(),
        content_json,
        model_used,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        latency_ms,
        cost_usd: cost_usd.clone(),
        created_at,
    });

    // 3. Update AgentBudget
    let total_tokens = input_tokens + output_tokens;
    if let Some(budget) = ctx.db.agent_budget().agent_id().find(&request.agent_id) {
        // Parse and add cost
        let prev_usd: f64 = budget.used_usd.parse().unwrap_or(0.0);
        let add_usd: f64 = cost_usd.parse().unwrap_or(0.0);
        let new_usd = prev_usd + add_usd;

        ctx.db.agent_budget().agent_id().update(AgentBudget {
            used_tokens: budget.used_tokens + total_tokens,
            used_usd: format!("{:.6}", new_usd),
            ..budget
        });
    }

    // 4. Update InferenceProvider avg_latency_ms (exponential moving average)
    if let Some(prov) = ctx.db.inference_provider().provider_id().find(&request.provider) {
        let new_avg = (prov.avg_latency_ms * 9 + latency_ms) / 10;
        ctx.db.inference_provider().provider_id().update(InferenceProvider {
            avg_latency_ms: new_avg,
            ..prov
        });
    }

    log::info!(
        "Inference completed: request={}, agent={}, tokens={}",
        request_id, request.agent_id, total_tokens
    );

    Ok(())
}

/// Mark a request as failed.
#[reducer]
pub fn fail_inference(
    ctx: &ReducerContext,
    request_id: u64,
    error_message: String,
    created_at: String,
) -> Result<(), String> {
    let request = ctx.db.inference_request().request_id().find(&request_id)
        .ok_or_else(|| format!("Request {} not found", request_id))?;

    // 1. Update request status
    ctx.db.inference_request().request_id().update(InferenceRequest {
        status: "failed".to_string(),
        ..request.clone()
    });

    // 2. Insert failure response
    ctx.db.inference_response().insert(InferenceResponse {
        response_id: 0, // auto_inc
        request_id,
        agent_id: request.agent_id.clone(),
        status: "failed".to_string(),
        content_json: format!("{{\"error\":{}}}", serde_json_escape(&error_message)),
        model_used: request.model.clone(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        latency_ms: 0,
        cost_usd: "0".to_string(),
        created_at,
    });

    log::info!(
        "Inference failed: request={}, agent={}, error={}",
        request_id, request.agent_id, error_message
    );

    Ok(())
}

/// Register or update an inference provider.
#[reducer]
pub fn register_provider(
    ctx: &ReducerContext,
    provider_id: String,
    provider_type: String,
    base_url: String,
    api_key_ref: String,
    models_json: String,
    rate_limit_rpm: u32,
    rate_limit_tpm: u64,
) -> Result<(), String> {
    validate_provider_type(&provider_type)?;

    if let Some(existing) = ctx.db.inference_provider().provider_id().find(&provider_id) {
        ctx.db.inference_provider().provider_id().update(InferenceProvider {
            provider_type,
            base_url,
            api_key_ref,
            models_json,
            rate_limit_rpm,
            rate_limit_tpm,
            ..existing
        });
    } else {
        ctx.db.inference_provider().insert(InferenceProvider {
            provider_id,
            provider_type,
            base_url,
            api_key_ref,
            models_json,
            rate_limit_rpm,
            rate_limit_tpm,
            current_rpm: 0,
            current_tpm: 0,
            healthy: 0,
            last_health_check: String::new(),
            avg_latency_ms: 0,
        });
    }

    Ok(())
}

/// Set or update an agent's token/cost budget.
#[reducer]
pub fn set_agent_budget(
    ctx: &ReducerContext,
    agent_id: String,
    total_budget_tokens: u64,
    total_budget_usd: String,
    max_single_request_tokens: u64,
    updated_at: String,
) -> Result<(), String> {
    if let Some(existing) = ctx.db.agent_budget().agent_id().find(&agent_id) {
        ctx.db.agent_budget().agent_id().update(AgentBudget {
            total_budget_tokens,
            total_budget_usd,
            max_single_request_tokens,
            updated_at,
            ..existing
        });
    } else {
        ctx.db.agent_budget().insert(AgentBudget {
            agent_id,
            total_budget_tokens,
            used_tokens: 0,
            total_budget_usd,
            used_usd: "0".to_string(),
            max_single_request_tokens,
            updated_at,
        });
    }

    Ok(())
}

/// Reset rate limit counters for all providers. Called periodically.
#[reducer]
pub fn reset_rate_counters(ctx: &ReducerContext) -> Result<(), String> {
    let providers: Vec<InferenceProvider> = ctx.db.inference_provider().iter().collect();

    for prov in providers {
        ctx.db.inference_provider().provider_id().update(InferenceProvider {
            current_rpm: 0,
            current_tpm: 0,
            ..prov
        });
    }

    log::info!("Rate counters reset for all providers");
    Ok(())
}

/// Append a streaming response chunk.
#[reducer]
pub fn append_stream_chunk(
    ctx: &ReducerContext,
    request_id: u64,
    agent_id: String,
    chunk_type: String,
    content: String,
    sequence: u32,
    created_at: String,
) -> Result<(), String> {
    validate_chunk_type(&chunk_type)?;

    ctx.db.inference_stream_chunk().insert(InferenceStreamChunk {
        chunk_id: 0, // auto_inc
        request_id,
        agent_id,
        chunk_type,
        content,
        sequence,
        created_at,
    });

    Ok(())
}

// ─── Pure logic helpers (testable without SpacetimeDB runtime) ───────────────

/// Validate a provider type string.
pub fn validate_provider_type(provider_type: &str) -> Result<(), String> {
    match provider_type {
        "anthropic" | "openai_compat" | "ollama" | "vllm" => Ok(()),
        _ => Err(format!(
            "Unknown provider type '{}'. Expected: anthropic, openai_compat, ollama, vllm",
            provider_type
        )),
    }
}

/// Validate a stream chunk type string.
pub fn validate_chunk_type(chunk_type: &str) -> Result<(), String> {
    match chunk_type {
        "text_delta" | "tool_use_start" | "input_json_delta" | "message_stop" => Ok(()),
        _ => Err(format!(
            "Unknown chunk type '{}'. Expected: text_delta, tool_use_start, input_json_delta, message_stop",
            chunk_type
        )),
    }
}

/// Validate an inference request status string.
pub fn validate_request_status(status: &str) -> Result<(), String> {
    match status {
        "queued" | "processing" | "completed" | "failed" => Ok(()),
        _ => Err(format!(
            "Invalid request status '{}'. Expected: queued, processing, completed, failed",
            status
        )),
    }
}

/// Validate a response status string.
pub fn validate_response_status(status: &str) -> Result<(), String> {
    match status {
        "completed" | "failed" | "rate_limited" | "budget_exceeded" => Ok(()),
        _ => Err(format!(
            "Invalid response status '{}'. Expected: completed, failed, rate_limited, budget_exceeded",
            status
        )),
    }
}

/// Validate priority value (0-3).
pub fn validate_priority(priority: u8) -> Result<(), String> {
    if priority <= 3 {
        Ok(())
    } else {
        Err(format!(
            "Invalid priority {}. Expected: 0=low, 1=normal, 2=high, 3=critical",
            priority
        ))
    }
}

/// Check if an agent is within budget.
pub fn is_within_budget(used_tokens: u64, max_tokens: u32, total_budget_tokens: u64) -> bool {
    used_tokens + max_tokens as u64 <= total_budget_tokens
}

/// Calculate exponential moving average for latency.
pub fn ema_latency(current_avg: u64, new_sample: u64) -> u64 {
    (current_avg * 9 + new_sample) / 10
}

/// Minimal JSON string escaping for error messages embedded in JSON.
pub fn serde_json_escape(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('"');
    for c in s.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c < '\x20' => {
                escaped.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

/// Add two USD cost strings, returning a formatted result.
pub fn add_cost_usd(a: &str, b: &str) -> String {
    let a_val: f64 = a.parse().unwrap_or(0.0);
    let b_val: f64 = b.parse().unwrap_or(0.0);
    format!("{:.6}", a_val + b_val)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Provider type validation ───────────────────────────────────────

    #[test]
    fn valid_provider_types_accepted() {
        for pt in &["anthropic", "openai_compat", "ollama", "vllm"] {
            assert!(validate_provider_type(pt).is_ok(), "Provider type '{}' should be valid", pt);
        }
    }

    #[test]
    fn invalid_provider_type_rejected() {
        assert!(validate_provider_type("unknown").is_err());
        assert!(validate_provider_type("").is_err());
        assert!(validate_provider_type("openai").is_err());
    }

    // ─── Chunk type validation ──────────────────────────────────────────

    #[test]
    fn valid_chunk_types_accepted() {
        for ct in &["text_delta", "tool_use_start", "input_json_delta", "message_stop"] {
            assert!(validate_chunk_type(ct).is_ok(), "Chunk type '{}' should be valid", ct);
        }
    }

    #[test]
    fn invalid_chunk_type_rejected() {
        assert!(validate_chunk_type("unknown").is_err());
        assert!(validate_chunk_type("").is_err());
    }

    // ─── Request status validation ──────────────────────────────────────

    #[test]
    fn valid_request_statuses_accepted() {
        for s in &["queued", "processing", "completed", "failed"] {
            assert!(validate_request_status(s).is_ok(), "Status '{}' should be valid", s);
        }
    }

    #[test]
    fn invalid_request_status_rejected() {
        assert!(validate_request_status("pending").is_err());
        assert!(validate_request_status("").is_err());
    }

    // ─── Response status validation ─────────────────────────────────────

    #[test]
    fn valid_response_statuses_accepted() {
        for s in &["completed", "failed", "rate_limited", "budget_exceeded"] {
            assert!(validate_response_status(s).is_ok(), "Status '{}' should be valid", s);
        }
    }

    #[test]
    fn invalid_response_status_rejected() {
        assert!(validate_response_status("queued").is_err());
        assert!(validate_response_status("").is_err());
    }

    // ─── Priority validation ────────────────────────────────────────────

    #[test]
    fn valid_priorities_accepted() {
        for p in 0..=3u8 {
            assert!(validate_priority(p).is_ok(), "Priority {} should be valid", p);
        }
    }

    #[test]
    fn invalid_priority_rejected() {
        assert!(validate_priority(4).is_err());
        assert!(validate_priority(255).is_err());
    }

    // ─── Budget checking ────────────────────────────────────────────────

    #[test]
    fn within_budget_when_under_limit() {
        assert!(is_within_budget(1000, 500, 2000));
    }

    #[test]
    fn within_budget_at_exact_limit() {
        assert!(is_within_budget(1500, 500, 2000));
    }

    #[test]
    fn over_budget_when_exceeded() {
        assert!(!is_within_budget(1500, 501, 2000));
    }

    #[test]
    fn within_budget_from_zero() {
        assert!(is_within_budget(0, 100, 100));
    }

    #[test]
    fn over_budget_from_zero() {
        assert!(!is_within_budget(0, 101, 100));
    }

    // ─── EMA latency ────────────────────────────────────────────────────

    #[test]
    fn ema_latency_basic() {
        // (100 * 9 + 200) / 10 = 1100 / 10 = 110
        assert_eq!(ema_latency(100, 200), 110);
    }

    #[test]
    fn ema_latency_same_value_stable() {
        assert_eq!(ema_latency(100, 100), 100);
    }

    #[test]
    fn ema_latency_from_zero() {
        // (0 * 9 + 500) / 10 = 50
        assert_eq!(ema_latency(0, 500), 50);
    }

    #[test]
    fn ema_latency_spike_smoothed() {
        // A 10x spike should only move average by 10%
        let avg = ema_latency(100, 1000);
        assert_eq!(avg, 190); // (900 + 1000) / 10
    }

    // ─── JSON escaping ──────────────────────────────────────────────────

    #[test]
    fn escape_plain_string() {
        assert_eq!(serde_json_escape("hello"), "\"hello\"");
    }

    #[test]
    fn escape_quotes() {
        assert_eq!(serde_json_escape("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn escape_backslash() {
        assert_eq!(serde_json_escape("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn escape_newline() {
        assert_eq!(serde_json_escape("line1\nline2"), "\"line1\\nline2\"");
    }

    #[test]
    fn escape_empty_string() {
        assert_eq!(serde_json_escape(""), "\"\"");
    }

    // ─── Cost addition ──────────────────────────────────────────────────

    #[test]
    fn add_cost_basic() {
        assert_eq!(add_cost_usd("0.001000", "0.002000"), "0.003000");
    }

    #[test]
    fn add_cost_from_zero() {
        assert_eq!(add_cost_usd("0", "0.005000"), "0.005000");
    }

    #[test]
    fn add_cost_invalid_treated_as_zero() {
        assert_eq!(add_cost_usd("invalid", "0.001000"), "0.001000");
    }

    #[test]
    fn add_cost_both_invalid() {
        assert_eq!(add_cost_usd("bad", "worse"), "0.000000");
    }
}
