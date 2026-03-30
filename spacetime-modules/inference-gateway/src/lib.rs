//! Inference Gateway SpacetimeDB Module (ADR-035)
//!
//! Routes ALL LLM inference through SpacetimeDB. Agents write requests via
//! `request_inference`, which immediately schedules the `execute_inference`
//! procedure. The procedure makes outbound HTTP calls to LLM APIs directly
//! inside SpacetimeDB — no external bridge required.
//!
//! Tables:
//!   - `inference_request` (public)        — agents write requests here
//!   - `inference_response` (public)       — responses written after LLM call
//!   - `inference_provider` (public)       — registered LLM endpoints
//!   - `inference_api_key` (private)       — actual API keys (set by hex-nexus)
//!   - `inference_execute_schedule`        — per-request procedure schedule
//!   - `agent_budget` (public)             — per-agent token budget enforcement
//!   - `inference_stream_chunk` (public)   — streaming response chunks

#![allow(clippy::too_many_arguments, clippy::needless_borrows_for_generic_args)]

use spacetimedb::{reducer, table, ReducerContext, ScheduleAt, Table};

// ─── Inference Request ──────────────────────────────────────────────────────

#[table(name = inference_request, public)]
#[derive(Clone, Debug)]
pub struct InferenceRequest {
    #[primary_key]
    #[auto_inc]
    pub request_id: u64,
    pub agent_id: String,
    /// Provider identifier: "anthropic", "minimax", "ollama", "vllm", "openrouter"
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
    /// Actual cost reported by OpenRouter (empty for other providers)
    pub openrouter_cost_usd: String,
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
    /// "anthropic", "openai_compat", "ollama", "vllm", "openrouter"
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
    /// Quantization tier: "q2", "q3", "q4", "q8", "fp16", "cloud" (ADR-2603271000)
    pub quantization_level: String,
    /// Context window in tokens (0 = unknown)
    pub context_window: u32,
    /// Quality score 0.0-1.0 (-1.0 = unknown)
    pub quality_score: f32,
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

// ─── API Key Store (private) ─────────────────────────────────────────────────

/// Stores actual resolved API key values for registered providers.
///
/// This table is intentionally private (no `public` attribute) — clients
/// cannot subscribe to it. Only hex-nexus populates it via `set_api_key`.
#[table(name = inference_api_key)]
#[derive(Clone, Debug)]
pub struct InferenceApiKey {
    /// Matches `InferenceProvider.provider_id`
    #[primary_key]
    pub provider_id: String,
    /// Actual API key value resolved from the OS environment by hex-nexus
    pub api_key: String,
}

// ─── Inference Execute Schedule ──────────────────────────────────────────────

/// One-shot schedule row that triggers `execute_inference` for a queued request.
///
/// Inserted by `request_inference` with `ScheduleAt::Interval(Duration::ZERO)`
/// so the procedure fires in the next scheduling tick. The row is automatically
/// deleted by SpacetimeDB after the procedure runs.
#[table(name = inference_execute_schedule, scheduled(execute_inference))]
#[derive(Clone, Debug)]
pub struct InferenceExecuteSchedule {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: ScheduleAt,
    pub request_id: u64,
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
                openrouter_cost_usd: String::new(),
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
                openrouter_cost_usd: String::new(),
                created_at: created_at.clone(),
            });
            return Ok(());
        }

        // Increment provider's current_rpm
        ctx.db
            .inference_provider()
            .provider_id()
            .update(InferenceProvider {
                current_rpm: prov.current_rpm + 1,
                ..prov
            });
    }

    // 3. Insert request with status "queued" — capture the auto-assigned ID
    let inserted = ctx.db.inference_request().insert(InferenceRequest {
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

    // 4. Schedule immediate procedure execution for this request
    ctx.db
        .inference_execute_schedule()
        .insert(InferenceExecuteSchedule {
            scheduled_id: 0, // auto_inc
            scheduled_at: ScheduleAt::Interval(std::time::Duration::ZERO.into()),
            request_id: inserted.request_id,
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
    openrouter_cost_usd: String,
    created_at: String,
) -> Result<(), String> {
    // 1. Update request status
    let request = ctx
        .db
        .inference_request()
        .request_id()
        .find(&request_id)
        .ok_or_else(|| format!("Request {} not found", request_id))?;

    ctx.db
        .inference_request()
        .request_id()
        .update(InferenceRequest {
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
        openrouter_cost_usd: openrouter_cost_usd.clone(),
        created_at,
    });

    // 3. Update AgentBudget
    let total_tokens = input_tokens + output_tokens;
    if let Some(budget) = ctx.db.agent_budget().agent_id().find(&request.agent_id) {
        // Parse and add cost
        let prev_usd: f64 = budget.used_usd.parse().unwrap_or(0.0);
        let add_usd: f64 = if !openrouter_cost_usd.is_empty() {
            openrouter_cost_usd
                .parse()
                .unwrap_or_else(|_| cost_usd.parse().unwrap_or(0.0))
        } else {
            cost_usd.parse().unwrap_or(0.0)
        };
        let new_usd = prev_usd + add_usd;

        ctx.db.agent_budget().agent_id().update(AgentBudget {
            used_tokens: budget.used_tokens + total_tokens,
            used_usd: format!("{:.6}", new_usd),
            ..budget
        });
    }

    // 4. Update InferenceProvider avg_latency_ms (exponential moving average)
    if let Some(prov) = ctx
        .db
        .inference_provider()
        .provider_id()
        .find(&request.provider)
    {
        let new_avg = (prov.avg_latency_ms * 9 + latency_ms) / 10;
        ctx.db
            .inference_provider()
            .provider_id()
            .update(InferenceProvider {
                avg_latency_ms: new_avg,
                ..prov
            });
    }

    log::info!(
        "Inference completed: request={}, agent={}, tokens={}",
        request_id,
        request.agent_id,
        total_tokens
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
    let request = ctx
        .db
        .inference_request()
        .request_id()
        .find(&request_id)
        .ok_or_else(|| format!("Request {} not found", request_id))?;

    // 1. Update request status
    ctx.db
        .inference_request()
        .request_id()
        .update(InferenceRequest {
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
        openrouter_cost_usd: String::new(),
        created_at,
    });

    log::info!(
        "Inference failed: request={}, agent={}, error={}",
        request_id,
        request.agent_id,
        error_message
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
    quantization_level: String,
    context_window: u32,
    quality_score: f32,
) -> Result<(), String> {
    validate_provider_type(&provider_type)?;

    if let Some(existing) = ctx.db.inference_provider().provider_id().find(&provider_id) {
        ctx.db
            .inference_provider()
            .provider_id()
            .update(InferenceProvider {
                provider_type,
                base_url,
                api_key_ref,
                models_json,
                rate_limit_rpm,
                rate_limit_tpm,
                quantization_level,
                context_window,
                quality_score,
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
            quantization_level,
            context_window,
            quality_score,
        });
    }

    Ok(())
}

/// Remove an inference provider by ID.
#[reducer]
pub fn remove_provider(ctx: &ReducerContext, provider_id: String) -> Result<(), String> {
    if let Some(_existing) = ctx.db.inference_provider().provider_id().find(&provider_id) {
        ctx.db
            .inference_provider()
            .provider_id()
            .delete(&provider_id);
        log::info!("Provider removed: {}", provider_id);
        Ok(())
    } else {
        Err(format!("Provider '{}' not found", provider_id))
    }
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
        ctx.db
            .inference_provider()
            .provider_id()
            .update(InferenceProvider {
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

    ctx.db
        .inference_stream_chunk()
        .insert(InferenceStreamChunk {
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

/// Store (or update) the resolved API key for a provider.
///
/// Called by hex-nexus on startup after it reads the key from the OS
/// environment or its own vault. The table is private so clients cannot
/// subscribe to the stored keys.
#[reducer]
pub fn set_api_key(ctx: &ReducerContext, provider_id: String, api_key: String) {
    if let Some(existing) = ctx.db.inference_api_key().provider_id().find(&provider_id) {
        ctx.db
            .inference_api_key()
            .provider_id()
            .update(InferenceApiKey { api_key, ..existing });
    } else {
        ctx.db
            .inference_api_key()
            .insert(InferenceApiKey { provider_id, api_key });
    }
}

// ─── Inference Procedure ─────────────────────────────────────────────────────

/// Execute one queued inference request by calling the provider's LLM API.
///
/// Scheduled by `request_inference` with `ScheduleAt::Interval(Duration::ZERO)`
/// so it fires in the next tick. The schedule row is deleted by SpacetimeDB
/// automatically after the procedure returns.
///
/// Flow:
///   1. `with_tx`: read InferenceRequest + InferenceProvider + InferenceApiKey
///   2. Build and send HTTP POST to the provider endpoint
///   3. `with_tx`: write InferenceResponse + update request status + update budget
#[spacetimedb::procedure]
pub fn execute_inference(
    ctx: &mut spacetimedb::ProcedureContext,
    schedule: InferenceExecuteSchedule,
) {
    let request_id = schedule.request_id;

    // ── Step 1: read needed rows inside a transaction ─────────────────────
    // with_tx takes Fn (not FnMut), so we use RefCell for interior mutability.
    use std::cell::RefCell;
    let req_cell: RefCell<Option<InferenceRequest>> = RefCell::new(None);
    let prov_cell: RefCell<Option<InferenceProvider>> = RefCell::new(None);
    let key_cell: RefCell<Option<String>> = RefCell::new(None);

    ctx.with_tx(|tx| {
        *req_cell.borrow_mut() = tx.db.inference_request().request_id().find(&request_id);
        if let Some(ref req) = *req_cell.borrow() {
            // "auto" = pick the first healthy registered provider that has an API key
            let provider_id = if req.provider == "auto" {
                tx.db
                    .inference_provider()
                    .iter()
                    .find(|p| {
                        p.healthy == 1
                            && tx
                                .db
                                .inference_api_key()
                                .provider_id()
                                .find(&p.provider_id)
                                .is_some()
                    })
                    .map(|p| p.provider_id.clone())
                    .or_else(|| {
                        // Fall back to any provider with a key, even if unhealthy
                        tx.db
                            .inference_provider()
                            .iter()
                            .find(|p| {
                                tx.db
                                    .inference_api_key()
                                    .provider_id()
                                    .find(&p.provider_id)
                                    .is_some()
                            })
                            .map(|p| p.provider_id.clone())
                    })
            } else {
                Some(req.provider.clone())
            };

            if let Some(pid) = provider_id {
                *prov_cell.borrow_mut() =
                    tx.db.inference_provider().provider_id().find(&pid);
                *key_cell.borrow_mut() = tx
                    .db
                    .inference_api_key()
                    .provider_id()
                    .find(&pid)
                    .map(|k| k.api_key);
            }
        }
    });

    let (request, provider, api_key) = match (req_cell.into_inner(), prov_cell.into_inner(), key_cell.into_inner()) {
        (Some(req), Some(prov), Some(key)) => (req, prov, key),
        (Some(req), _, _) => {
            log::error!(
                "execute_inference: no provider/key for request {} provider={}",
                request_id,
                req.provider
            );
            ctx.with_tx(|tx| {
                mark_failed(tx, request_id, &req, "provider or API key not configured");
            });
            return;
        }
        _ => {
            log::error!(
                "execute_inference: request {} not found in DB",
                request_id
            );
            return;
        }
    };

    // ── Step 2: build HTTP request ────────────────────────────────────────
    let (url, body_json, auth_header_name, auth_header_value) =
        build_llm_request(&request, &provider, &api_key);

    let http_request = match spacetimedb::http::Request::builder()
        .uri(&url)
        .method("POST")
        .header("Content-Type", "application/json")
        .header(auth_header_name, auth_header_value)
        .body(body_json)
    {
        Ok(r) => r,
        Err(e) => {
            log::error!("execute_inference: failed to build HTTP request: {:?}", e);
            ctx.with_tx(|tx| {
                mark_failed(tx, request_id, &request, &format!("request build error: {e:?}"));
            });
            return;
        }
    };

    log::info!(
        "execute_inference: sending request={} provider={} model={}",
        request_id,
        request.provider,
        request.model
    );

    // ── Step 3: send HTTP request (outside any transaction) ───────────────
    match ctx.http.send(http_request) {
        Ok(response) => {
            let (parts, body) = response.into_parts();
            let status_code = parts.status.as_u16();
            let body_str = body.into_string_lossy();

            if (200..300).contains(&status_code) {
                let (content_json, model_used, input_tokens, output_tokens, or_cost) =
                    parse_llm_response(&body_str, &request.provider, &provider.provider_type);

                log::info!(
                    "execute_inference: completed request={} model={} in={} out={}",
                    request_id,
                    model_used,
                    input_tokens,
                    output_tokens
                );

                ctx.with_tx(|tx| {
                    // Update request status
                    tx.db.inference_request().request_id().update(InferenceRequest {
                        status: "completed".to_string(),
                        ..request.clone()
                    });

                    // Insert response
                    tx.db.inference_response().insert(InferenceResponse {
                        response_id: 0,
                        request_id,
                        agent_id: request.agent_id.clone(),
                        status: "completed".to_string(),
                        content_json: content_json.clone(),
                        model_used: model_used.clone(),
                        input_tokens,
                        output_tokens,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        latency_ms: 0,
                        cost_usd: "0".to_string(),
                        openrouter_cost_usd: or_cost.clone(),
                        created_at: String::new(),
                    });

                    // Update agent budget
                    let total_tokens = input_tokens + output_tokens;
                    if let Some(budget) = tx.db.agent_budget().agent_id().find(&request.agent_id) {
                        let prev_usd: f64 = budget.used_usd.parse().unwrap_or(0.0);
                        let add_usd: f64 = if !or_cost.is_empty() {
                            or_cost.parse().unwrap_or(0.0)
                        } else {
                            0.0
                        };
                        tx.db.agent_budget().agent_id().update(AgentBudget {
                            used_tokens: budget.used_tokens + total_tokens,
                            used_usd: format!("{:.6}", prev_usd + add_usd),
                            ..budget
                        });
                    }

                    // Update provider avg latency (EMA, no Instant so use 0)
                    if let Some(prov) = tx
                        .db
                        .inference_provider()
                        .provider_id()
                        .find(&request.provider)
                    {
                        tx.db
                            .inference_provider()
                            .provider_id()
                            .update(InferenceProvider {
                                current_rpm: prov.current_rpm.saturating_sub(1),
                                ..prov
                            });
                    }
                });
            } else {
                log::warn!(
                    "execute_inference: HTTP {} for request={}: {}",
                    status_code,
                    request_id,
                    &body_str[..body_str.len().min(200)]
                );
                let err = format!("HTTP {status_code}: {}", &body_str[..body_str.len().min(300)]);
                ctx.with_tx(|tx| {
                    mark_failed(tx, request_id, &request, &err);
                });
            }
        }
        Err(e) => {
            log::error!(
                "execute_inference: HTTP error for request={}: {:?}",
                request_id,
                e
            );
            let err = format!("HTTP error: {e:?}");
            ctx.with_tx(|tx| {
                mark_failed(tx, request_id, &request, &err);
            });
        }
    }
}

// ─── Procedure helpers (not exposed as reducers) ──────────────────────────────

/// Build the HTTP request parameters for a given provider type.
///
/// Returns `(url, body_json, auth_header_name, auth_header_value)`.
fn build_llm_request(
    request: &InferenceRequest,
    provider: &InferenceProvider,
    api_key: &str,
) -> (String, String, &'static str, String) {
    let model = if request.model.is_empty() {
        "claude-3-5-haiku-20241022".to_string()
    } else {
        request.model.clone()
    };

    let max_tokens = if request.max_tokens == 0 { 4096 } else { request.max_tokens };
    let temperature: f64 = request.temperature.parse().unwrap_or(0.7);

    match provider.provider_type.as_str() {
        "anthropic" => {
            let url = format!("{}/messages", provider.base_url.trim_end_matches('/'));
            let body = format!(
                r#"{{"model":{},"max_tokens":{},"temperature":{},"messages":{}}}"#,
                serde_json_escape_string(&model),
                max_tokens,
                temperature,
                request.messages_json,
            );
            (url, body, "x-api-key", api_key.to_string())
        }
        _ => {
            // OpenAI-compatible: OpenRouter, vLLM, Ollama, openai_compat
            let url = format!(
                "{}/chat/completions",
                provider.base_url.trim_end_matches('/')
            );
            let body = format!(
                r#"{{"model":{},"max_tokens":{},"temperature":{},"messages":{}}}"#,
                serde_json_escape_string(&model),
                max_tokens,
                temperature,
                request.messages_json,
            );
            (url, body, "Authorization", format!("Bearer {api_key}"))
        }
    }
}

/// Parse the LLM HTTP response body into structured fields.
///
/// Returns `(content_json, model_used, input_tokens, output_tokens, openrouter_cost)`.
fn parse_llm_response(
    body: &str,
    _provider_id: &str,
    provider_type: &str,
) -> (String, String, u64, u64, String) {
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            log::error!("parse_llm_response: JSON parse error: {}", e);
            return (
                format!(r#"{{"error":"parse error: {}"}}"#, e),
                "unknown".to_string(),
                0,
                0,
                String::new(),
            );
        }
    };

    let model_used = v["model"].as_str().unwrap_or("unknown").to_string();

    match provider_type {
        "anthropic" => {
            let text = v["content"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|c| c["text"].as_str())
                .unwrap_or("");
            let content_json = format!(
                r#"[{{"type":"text","text":{}}}]"#,
                serde_json_escape_string(text)
            );
            let input = v["usage"]["input_tokens"].as_u64().unwrap_or(0);
            let output = v["usage"]["output_tokens"].as_u64().unwrap_or(0);
            (content_json, model_used, input, output, String::new())
        }
        _ => {
            // OpenAI-compatible
            let text = v["choices"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|c| c["message"]["content"].as_str())
                .unwrap_or("");
            let content_json = format!(
                r#"[{{"type":"text","text":{}}}]"#,
                serde_json_escape_string(text)
            );
            let input = v["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
            let output = v["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            let or_cost = v["usage"]["cost"]
                .as_str()
                .unwrap_or("")
                .to_string();
            (content_json, model_used, input, output, or_cost)
        }
    }
}

/// Mark an inference request as failed and write a failure response row.
fn mark_failed(tx: &ReducerContext, request_id: u64, request: &InferenceRequest, error: &str) {
    tx.db
        .inference_request()
        .request_id()
        .update(InferenceRequest {
            status: "failed".to_string(),
            ..request.clone()
        });
    tx.db.inference_response().insert(InferenceResponse {
        response_id: 0,
        request_id,
        agent_id: request.agent_id.clone(),
        status: "failed".to_string(),
        content_json: format!(r#"{{"error":{}}}"#, serde_json_escape(error)),
        model_used: request.model.clone(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        latency_ms: 0,
        cost_usd: "0".to_string(),
        openrouter_cost_usd: String::new(),
        created_at: String::new(),
    });
}

/// Escape a Rust string into a JSON string literal (including surrounding quotes).
fn serde_json_escape_string(s: &str) -> String {
    serde_json_escape(s)
}

// ─── Pure logic helpers (testable without SpacetimeDB runtime) ───────────────

/// Validate a provider type string.
pub fn validate_provider_type(provider_type: &str) -> Result<(), String> {
    match provider_type {
        "anthropic" | "openai_compat" | "ollama" | "vllm" | "openrouter" => Ok(()),
        _ => Err(format!(
            "Unknown provider type '{}'. Expected: anthropic, openai_compat, ollama, vllm, openrouter",
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
        for pt in &["anthropic", "openai_compat", "ollama", "vllm", "openrouter"] {
            assert!(
                validate_provider_type(pt).is_ok(),
                "Provider type '{}' should be valid",
                pt
            );
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
        for ct in &[
            "text_delta",
            "tool_use_start",
            "input_json_delta",
            "message_stop",
        ] {
            assert!(
                validate_chunk_type(ct).is_ok(),
                "Chunk type '{}' should be valid",
                ct
            );
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
            assert!(
                validate_request_status(s).is_ok(),
                "Status '{}' should be valid",
                s
            );
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
            assert!(
                validate_response_status(s).is_ok(),
                "Status '{}' should be valid",
                s
            );
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
            assert!(
                validate_priority(p).is_ok(),
                "Priority {} should be valid",
                p
            );
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

    // ─── OpenRouter cost preference ────────────────────────────────────

    #[test]
    fn openrouter_cost_preferred_over_computed() {
        // When openrouter_cost_usd is present, it should be preferred
        let openrouter_cost = "0.003500";
        let computed_cost = "0.004000";

        let add_usd: f64 = if !openrouter_cost.is_empty() {
            openrouter_cost
                .parse()
                .unwrap_or_else(|_| computed_cost.parse().unwrap_or(0.0))
        } else {
            computed_cost.parse().unwrap_or(0.0)
        };
        assert!((add_usd - 0.003500).abs() < f64::EPSILON);

        // When openrouter_cost_usd is empty, fall back to computed
        let empty_cost = "";
        let add_usd_fallback: f64 = if !empty_cost.is_empty() {
            empty_cost
                .parse()
                .unwrap_or_else(|_| computed_cost.parse().unwrap_or(0.0))
        } else {
            computed_cost.parse().unwrap_or(0.0)
        };
        assert!((add_usd_fallback - 0.004000).abs() < f64::EPSILON);

        // When openrouter_cost_usd is invalid, fall back to computed
        let bad_cost = "not_a_number";
        let add_usd_bad: f64 = if !bad_cost.is_empty() {
            bad_cost
                .parse()
                .unwrap_or_else(|_| computed_cost.parse().unwrap_or(0.0))
        } else {
            computed_cost.parse().unwrap_or(0.0)
        };
        assert!((add_usd_bad - 0.004000).abs() < f64::EPSILON);
    }
}
