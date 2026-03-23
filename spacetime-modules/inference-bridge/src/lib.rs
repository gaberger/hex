//! inference-bridge — Queue-driven LLM inference via SpacetimeDB.
//!
//! ADR-039 Phase 9: Hybrid LLM Bridge.
//!
//! Instead of agents calling hex-nexus HTTP directly for inference,
//! they write an InferenceRequest row to SpacetimeDB. A hex-nexus
//! "worker" subscribes to pending requests, routes them to the best
//! provider, and writes the response back. The requesting agent sees
//! the response via its SpacetimeDB subscription — no polling.

#![allow(clippy::too_many_arguments, clippy::needless_borrows_for_generic_args)]

//!
//! Instead of agents calling hex-nexus HTTP directly for inference,
//! they write an InferenceRequest row to SpacetimeDB. A hex-nexus
//! "worker" subscribes to pending requests, routes them to the best
//! provider, and writes the response back. The requesting agent sees
//! the response via its SpacetimeDB subscription — no polling.
//!
//! Benefits:
//! - Inference requests survive hex-nexus restarts
//! - Multiple hex-nexus instances can compete to fulfill requests (load balancing)
//! - Full audit trail of every inference call
//! - Browser can watch inference in real-time
//! - Rate limiting and budgets enforced server-side

use spacetimedb::{reducer, table, ReducerContext, Table, Timestamp};

// ── Tables ──────────────────────────────────────────────

/// Inference request queue. Agents insert, hex-nexus workers fulfill.
#[table(name = inference_queue, public)]
pub struct InferenceQueue {
    #[primary_key]
    pub request_id: String,
    pub agent_id: String,
    pub swarm_id: String,
    pub model: String,         // requested model (e.g. "qwen2.5-coder:7b")
    pub provider_hint: String, // preferred provider (empty = auto-route)
    pub messages_json: String, // JSON-encoded message array
    pub max_tokens: u32,
    pub temperature: f64,
    pub status: String,    // "pending", "processing", "completed", "failed"
    pub worker_id: String, // which hex-nexus instance claimed this
    pub created_at: Timestamp,
    pub claimed_at: Timestamp,
    pub completed_at: Timestamp,
}

/// Inference response — written by hex-nexus worker after LLM call.
#[table(name = inference_result, public)]
pub struct InferenceResult {
    #[primary_key]
    pub request_id: String,
    pub response_text: String,
    pub model_used: String,
    pub provider_used: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u64,
    pub cost_estimate: f64, // USD estimate
    pub completed_at: Timestamp,
}

/// Per-agent token budget. Enforced before accepting requests.
#[table(name = agent_token_budget, public)]
pub struct AgentTokenBudget {
    #[primary_key]
    pub agent_id: String,
    pub max_tokens: u64,
    pub used_tokens: u64,
    pub max_cost_usd: f64,
    pub used_cost_usd: f64,
    pub period_start: Timestamp,
}

/// Provider routing table — which providers are available and healthy.
#[table(name = provider_route, public)]
pub struct ProviderRoute {
    #[primary_key]
    pub provider_id: String,
    pub provider_type: String, // "ollama", "vllm", "openai", "anthropic"
    pub base_url: String,
    pub models_json: String, // JSON array of model names
    pub healthy: bool,
    pub priority: u32, // lower = preferred
    pub rpm_limit: u32,
    pub current_rpm: u32,
    pub last_health_check: Timestamp,
}

// ── Reducers ────────────────────────────────────────────

/// Agent submits an inference request to the queue.
#[reducer]
pub fn submit_inference(
    ctx: &ReducerContext,
    request_id: String,
    agent_id: String,
    swarm_id: String,
    model: String,
    messages_json: String,
    max_tokens: u32,
    temperature: f64,
) {
    let now = ctx.timestamp;

    // Check budget
    if let Some(budget) = ctx.db.agent_token_budget().agent_id().find(&agent_id) {
        if budget.used_tokens >= budget.max_tokens {
            log::warn!("Agent {} exceeded token budget", agent_id);
            // Still insert but mark as failed
            ctx.db.inference_queue().insert(InferenceQueue {
                request_id,
                agent_id,
                swarm_id,
                model,
                provider_hint: String::new(),
                messages_json,
                max_tokens,
                temperature,
                status: "failed".to_string(),
                worker_id: String::new(),
                created_at: now,
                claimed_at: now,
                completed_at: now,
            });
            return;
        }
    }

    ctx.db.inference_queue().insert(InferenceQueue {
        request_id,
        agent_id,
        swarm_id,
        model,
        provider_hint: String::new(),
        messages_json,
        max_tokens,
        temperature,
        status: "pending".to_string(),
        worker_id: String::new(),
        created_at: now,
        claimed_at: Timestamp::UNIX_EPOCH,
        completed_at: Timestamp::UNIX_EPOCH,
    });
}

/// hex-nexus worker claims a pending request for processing.
/// Uses optimistic locking: only succeeds if status is still "pending".
#[reducer]
pub fn claim_inference(ctx: &ReducerContext, request_id: String, worker_id: String) {
    let Some(mut req) = ctx.db.inference_queue().request_id().find(&request_id) else {
        return;
    };

    if req.status != "pending" {
        log::warn!(
            "Request {} already claimed (status: {})",
            request_id,
            req.status
        );
        return; // Already claimed by another worker
    }

    req.status = "processing".to_string();
    req.worker_id = worker_id;
    req.claimed_at = ctx.timestamp;
    ctx.db.inference_queue().request_id().update(req);
}

/// hex-nexus worker writes the inference result after LLM call completes.
#[reducer]
pub fn complete_inference(
    ctx: &ReducerContext,
    request_id: String,
    response_text: String,
    model_used: String,
    provider_used: String,
    input_tokens: u32,
    output_tokens: u32,
    latency_ms: u64,
    cost_estimate: f64,
) {
    let now = ctx.timestamp;

    // Update queue status
    if let Some(mut req) = ctx.db.inference_queue().request_id().find(&request_id) {
        req.status = "completed".to_string();
        req.completed_at = now;
        let agent_id = req.agent_id.clone();
        ctx.db.inference_queue().request_id().update(req);

        // Update agent budget
        if let Some(mut budget) = ctx.db.agent_token_budget().agent_id().find(&agent_id) {
            budget.used_tokens += (input_tokens + output_tokens) as u64;
            budget.used_cost_usd += cost_estimate;
            ctx.db.agent_token_budget().agent_id().update(budget);
        }
    }

    // Write result
    ctx.db.inference_result().insert(InferenceResult {
        request_id,
        response_text,
        model_used,
        provider_used,
        input_tokens,
        output_tokens,
        latency_ms,
        cost_estimate,
        completed_at: now,
    });
}

/// Mark a request as failed.
#[reducer]
pub fn fail_inference(ctx: &ReducerContext, request_id: String, error: String) {
    if let Some(mut req) = ctx.db.inference_queue().request_id().find(&request_id) {
        req.status = "failed".to_string();
        req.completed_at = ctx.timestamp;
        ctx.db.inference_queue().request_id().update(req);
    }

    // Write error as result
    ctx.db.inference_result().insert(InferenceResult {
        request_id,
        response_text: format!("ERROR: {}", error),
        model_used: String::new(),
        provider_used: String::new(),
        input_tokens: 0,
        output_tokens: 0,
        latency_ms: 0,
        cost_estimate: 0.0,
        completed_at: ctx.timestamp,
    });
}

/// Set token budget for an agent.
#[reducer]
pub fn set_agent_budget(
    ctx: &ReducerContext,
    agent_id: String,
    max_tokens: u64,
    max_cost_usd: f64,
) {
    // Upsert
    if let Some(mut budget) = ctx.db.agent_token_budget().agent_id().find(&agent_id) {
        budget.max_tokens = max_tokens;
        budget.max_cost_usd = max_cost_usd;
        ctx.db.agent_token_budget().agent_id().update(budget);
    } else {
        ctx.db.agent_token_budget().insert(AgentTokenBudget {
            agent_id,
            max_tokens,
            used_tokens: 0,
            max_cost_usd,
            used_cost_usd: 0.0,
            period_start: ctx.timestamp,
        });
    }
}

/// Register or update a provider route.
#[reducer]
pub fn register_provider_route(
    ctx: &ReducerContext,
    provider_id: String,
    provider_type: String,
    base_url: String,
    models_json: String,
    priority: u32,
    rpm_limit: u32,
) {
    if let Some(mut route) = ctx.db.provider_route().provider_id().find(&provider_id) {
        route.base_url = base_url;
        route.models_json = models_json;
        route.priority = priority;
        route.rpm_limit = rpm_limit;
        route.last_health_check = ctx.timestamp;
        ctx.db.provider_route().provider_id().update(route);
    } else {
        ctx.db.provider_route().insert(ProviderRoute {
            provider_id,
            provider_type,
            base_url,
            models_json,
            healthy: true,
            priority,
            rpm_limit,
            current_rpm: 0,
            last_health_check: ctx.timestamp,
        });
    }
}
