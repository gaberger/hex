# ADR-028: API Optimization Layer

**Status:** Accepted
**Date:** 2026-03-18
**Deciders:** Gary
**Relates to:** ADR-024 (hex-nexus), ADR-027 (HexFlo)

## Context

hex-agent interfaces directly with the Anthropic Messages API. At scale (multi-agent swarms, long conversations, bulk analysis), we hit three cost/throughput walls:

1. **Input token waste** — The system prompt (~15k tokens) and tool definitions (~8k tokens) are resent identically on every API call within a conversation. At 25 tool rounds per turn, that's 575k redundant input tokens per turn.
2. **Rate limit exhaustion** — Anthropic enforces per-model RPM, input TPM, and output TPM limits. Extended thinking on Opus can consume 30k+ output tokens in a single response, hitting the 80k output TPM limit in 2-3 requests.
3. **No batch path** — Non-interactive workloads (code analysis, bulk summarization, dead-code detection) use the same real-time endpoint as interactive conversation, paying full price when 50% discount is available via the Batch API.

Claude Code (Anthropic's own CLI) addresses similar problems with: prompt caching via `cache_control`, model tiering (Sonnet for reasoning, Haiku for classification), and context compaction. We adopt these patterns natively in hex-agent.

## Decision

Implement a six-component API optimization layer in hex-agent, following hexagonal architecture (domain types → ports → adapters → usecases):

### 1. Prompt Caching (`cache_control`)

Add `cache_control: {"type": "ephemeral"}` to system prompt and tool definition blocks. The Anthropic API caches these and serves subsequent reads for free, bypassing input TPM limits.

- Domain: `ApiRequestOptions.enable_cache`, `CacheMetrics`
- Port: `AnthropicPort.send_message()` gains `options: Option<&ApiRequestOptions>`
- Adapter: `AnthropicAdapter` builds structured system blocks with cache_control, sends `anthropic-beta: prompt-caching-2024-07-31` header
- Default: **enabled** (opt-out via `--no-cache`)

### 2. Batch API Integration

New `BatchPort` for submitting non-interactive workloads at 50% cost reduction.

- Domain: `BatchRequest`, `BatchStatus`, `WorkloadClass`
- Port: `BatchPort` (submit, poll status, get results, cancel)
- Usecase: `WorkloadRouter` classifies tasks and routes to real-time or batch endpoint

### 3. Rate Limit Management

Proactive throttling with per-model tracking instead of reactive 429 handling.

- Domain: `RateLimitState` tracks RPM, input TPM, output TPM per model with 60s sliding window
- Port: `RateLimiterPort` (should_throttle, record_usage, record_rate_limit, update_from_headers)
- Adapter: `RateLimiterAdapter` — in-memory, auto-resets on window expiry
- Integration: Conversation loop checks `should_throttle()` before every API call, parses `anthropic-ratelimit-*` headers from responses

### 4. Extended Thinking Budget Control

Configurable `budget_tokens` parameter to prevent output TPM exhaustion.

- Domain: `ThinkingConfig { enabled, budget_tokens }`
- Passed via `ApiRequestOptions.thinking` to the Anthropic adapter
- CLI: `--thinking-budget 10000`

### 5. Workload Router

Usecase that classifies requests and routes to the optimal endpoint.

- `WorkloadClass::classify(task_type)` — batch-eligible: code_analysis, summarization, test_generation, etc.
- Routes to Batch API when rate limit utilization exceeds 70% threshold
- Falls back to real-time for batch-eligible work when utilization is low (speed over savings)

### 6. Token Metrics Dashboard

`TokenMetricsPort` aggregates consumption data for the hex dashboard.

- Tracks: cached vs uncached input, output, batch vs real-time, per-model breakdown
- Exposes `ApiMetricsSnapshot` with estimated savings percentage
- Feeds into hex-hub dashboard via existing hub WebSocket protocol

## Architecture

```
                          ┌─────────────────────┐
                          │   ConversationLoop   │ (usecase)
                          │  ┌───────────────┐   │
                          │  │ ApiRequestOpts │   │
                          │  │  .cache=true   │   │
                          │  │  .thinking=10k │   │
                          │  └───────────────┘   │
                          └──────┬────┬──────────┘
                   ┌─────────────┘    └─────────────┐
                   ▼                                 ▼
          ┌────────────────┐               ┌────────────────┐
          │ RateLimiterPort│               │  AnthropicPort  │
          │ should_throttle│               │  send_message   │
          │ record_usage   │               │  +cache_control │
          │ record_limit   │               │  +thinking      │
          └────────────────┘               └────────────────┘
                   │                                 │
          ┌────────────────┐               ┌────────────────┐
          │RateLimiterAdapt│               │AnthropicAdapter │
          │ per-model state│               │ SSE + headers   │
          │ 60s window     │               │ beta headers    │
          └────────────────┘               └────────────────┘
```

## Files Changed

### Domain (zero dependencies)
- `src/domain/api_optimization.rs` — NEW: WorkloadClass, ThinkingConfig, ApiRequestOptions, CacheMetrics, RateLimitState, RateLimitHeaders, BatchRequest, BatchStatus, ApiMetricsSnapshot
- `src/domain/tokens.rs` — MODIFIED: TokenUsage gains cache_read_tokens, cache_write_tokens, billable_input()

### Ports (imports domain only)
- `src/ports/anthropic.rs` — MODIFIED: send_message/stream_message gain `options` parameter
- `src/ports/rate_limiter.rs` — NEW: RateLimiterPort
- `src/ports/batch.rs` — NEW: BatchPort
- `src/ports/token_metrics.rs` — NEW: TokenMetricsPort

### Adapters (implements ports)
- `src/adapters/secondary/anthropic.rs` — MODIFIED: cache_control blocks, beta headers, rate limit header parsing, thinking parameter
- `src/adapters/secondary/rate_limiter.rs` — NEW: RateLimiterAdapter (in-memory per-model tracking)
- `src/adapters/secondary/token_metrics.rs` — NEW: TokenMetricsAdapter (in-memory aggregation)

### Usecases (composes ports)
- `src/usecases/conversation.rs` — MODIFIED: integrates rate limiter + metrics + cache options
- `src/usecases/workload_router.rs` — NEW: classifies and routes workloads

### Composition Root
- `src/main.rs` — MODIFIED: wires new adapters, adds --no-cache and --thinking-budget CLI args

## Consequences

### Positive
- **~90% input token savings** on repeated system prompts via prompt caching
- **50% cost reduction** on batch-eligible workloads
- **Fewer 429s** from proactive throttling instead of reactive retry
- **Output TPM protection** via thinking budget cap
- **Observable** — all metrics flow through TokenMetricsPort to the dashboard

### Negative
- Prompt caching requires `anthropic-beta` header (beta feature, may change)
- Batch API has up to 24h latency — not suitable for interactive work
- Rate limiter state is in-memory only (resets on process restart)

### Risks
- Cache TTL is controlled by Anthropic (currently 5 minutes) — if they change it, savings ratio drops
- Extended thinking beta header may conflict with prompt caching beta header when both are needed simultaneously

## References
- [Anthropic Prompt Caching docs](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching)
- [Anthropic Message Batches API](https://docs.anthropic.com/en/docs/build-with-claude/message-batches)
- [Anthropic Rate Limits](https://docs.anthropic.com/en/docs/about-claude/models#rate-limits)
- [Kirshatrov: Claude Code Internals](https://kirshatrov.com/posts/claude-code-internals) — reference implementation patterns
