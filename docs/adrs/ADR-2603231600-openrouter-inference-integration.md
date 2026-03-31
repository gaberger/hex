# ADR-2603231600: OpenRouter Inference Integration

**Status:** Implemented
**Date:** 2026-03-23
**Drivers:** Need access to 300+ open-source models (Llama 4, Qwen 3, DeepSeek R1, Mistral, Command R+, etc.) via a single API key, without managing individual provider accounts
**Supersedes:** None (extends ADR-030)

## Context

hex's inference layer currently supports three provider classes:

1. **Anthropic** (native) — Opus, Sonnet, Haiku via `AnthropicPort`
2. **OpenAI-compatible** (ADR-030) — MiniMax M2.5, Groq, Together AI via `OpenAiCompatAdapter`
3. **Local** — Ollama, vLLM on localhost

For open-source models (Llama 4 Maverick, Qwen 3 235B, DeepSeek R1, Mistral Large, etc.), we currently need either:
- A local GPU powerful enough to run them (not always available)
- Individual accounts with Together AI, Fireworks, Groq, etc. (N API keys, N billing dashboards)

**OpenRouter** (openrouter.ai) solves this by providing a unified OpenAI-compatible API that routes to 300+ models across dozens of providers. One API key, one billing dashboard, automatic provider failover.

### Forces

- **Agent swarms need diverse models**: ADR-031's RL model selection engine benefits from a wider model pool — different tasks have different cost/quality optima. A summarization task might use Llama 4 Scout ($0.15/M input), while a complex refactor uses Opus.
- **OpenRouter uses the OpenAI chat completions format**: Our `OpenAiCompatAdapter` (ADR-030) already handles this wire protocol.
- **Provider-of-providers complexity**: OpenRouter itself routes between providers (e.g., Llama 4 might run on Together, Lambda, or Fireworks). This creates a two-level routing situation: hex selects the model, OpenRouter selects the hosting provider.
- **Cost transparency**: OpenRouter reports actual cost per request, which our budget enforcement (inference-gateway WASM module) needs for accurate tracking.
- **Rate limits differ per model**: Unlike Anthropic where we know our limits, OpenRouter rate limits depend on the upstream provider and your account tier.

### Alternatives Considered

| Alternative | Pros | Cons |
|------------|------|------|
| Direct provider accounts (Together, Fireworks, Groq) | Full control, lowest latency | N API keys, N billing, N rate limit configs |
| Local-only via Ollama | Zero cost, full privacy | Requires GPU, limited model sizes, slow on CPU |
| OpenRouter | Single key, 300+ models, auto-failover | Adds intermediary hop, markup on pricing |

## Decision

### 1. OpenRouter as a First-Class Provider

Add `OpenRouter` as a new variant in the inference provider taxonomy, distinct from generic `OpenaiCompatible`:

```rust
// inference-gateway WASM module — inference_provider table
enum ProviderKind {
    Anthropic,
    OpenaiCompatible,
    Ollama,
    Vllm,
    OpenRouter,        // NEW — provider-of-providers
}
```

OpenRouter is not just another OpenAI-compatible endpoint. It requires special handling for:
- **Model ID format**: `meta-llama/llama-4-maverick`, `deepseek/deepseek-r1`, `qwen/qwen3-235b`
- **Extra headers**: `HTTP-Referer`, `X-Title` (for OpenRouter's leaderboard/analytics)
- **Cost reporting**: Response includes `usage.cost` field (actual USD cost)
- **Provider preferences**: `provider.order`, `provider.allow`, `provider.deny` in request body
- **Fallback routing**: `route: "fallback"` to auto-retry on different providers

### 2. Adapter Implementation

Extend the existing `OpenAiCompatAdapter` with an OpenRouter specialization rather than creating a new adapter. The wire protocol is identical; only metadata handling differs:

```rust
impl OpenAiCompatAdapter {
    fn openrouter_headers(&self) -> HeaderMap {
        // X-Title: "hex-agent" (identifies our app in OpenRouter dashboard)
        // HTTP-Referer: project URL (for analytics)
    }

    fn openrouter_body_extensions(&self, req: &InferenceRequest) -> Value {
        // provider.order: prefer low-latency providers
        // route: "fallback" for non-interactive workloads
        // transforms: ["middle-out"] for long-context models
    }

    fn extract_openrouter_cost(&self, resp: &Value) -> Option<f64> {
        // OpenRouter returns actual cost in response — use this
        // instead of calculating from token counts × published rates
    }
}
```

### 3. Model Registry

Rather than hardcoding models, OpenRouter integration includes a model discovery mechanism:

```
hex inference discover --provider openrouter
```

This calls `GET https://openrouter.ai/api/v1/models` and populates the `inference_provider` SpacetimeDB table with available models, their context windows, pricing, and supported features (tool_use, vision, etc.).

**Recommended starter models** (for RL model selection pool):

| Model | Context | Input $/M | Output $/M | Best For |
|-------|---------|-----------|------------|----------|
| `meta-llama/llama-4-maverick` | 1M | $0.25 | $1.00 | General coding, large context |
| `meta-llama/llama-4-scout` | 512K | $0.15 | $0.60 | Summarization, analysis |
| `qwen/qwen3-235b` | 128K | $0.20 | $0.80 | Multilingual, reasoning |
| `deepseek/deepseek-r1` | 128K | $0.55 | $2.19 | Complex reasoning, math |
| `google/gemini-2.5-pro` | 1M | $1.25 | $10.00 | Long-context analysis |
| `mistralai/mistral-large` | 128K | $2.00 | $6.00 | European compliance tasks |

### 4. Budget & Cost Tracking

OpenRouter returns actual cost in the response body (`usage.cost`). The inference-gateway WASM module's `complete_inference()` reducer will use this field directly when the provider is `OpenRouter`, rather than computing cost from `token_count × rate`:

```rust
// In complete_inference() reducer
let cost = if provider_kind == ProviderKind::OpenRouter {
    response.openrouter_cost  // Actual cost from OpenRouter
} else {
    compute_cost(tokens, model_rates)  // Our calculation
};
```

This is more accurate because OpenRouter pricing varies by upstream provider and includes their margin.

### 5. RL Model Selection Integration (ADR-031)

The RL engine's model pool expands to include OpenRouter models. The reward signal incorporates OpenRouter-specific data:

- **Cost efficiency**: actual cost from response (not estimated)
- **Latency**: end-to-end including OpenRouter routing overhead
- **Quality**: existing task-completion scoring
- **Availability**: track per-model success rates (some models go offline on specific providers)

New `ModelSelection` variants:

```rust
enum ModelSelection {
    // Existing
    Opus, Sonnet, Haiku, MiniMax, MiniMaxFast, Local,
    // New — OpenRouter models
    OpenRouter(String),  // Model ID, e.g. "meta-llama/llama-4-maverick"
}
```

### 6. Secret Management

Single secret: `OPENROUTER_API_KEY`, managed via hex's existing secrets vault (ADR-026):

```bash
hex secrets set OPENROUTER_API_KEY sk-or-...
```

### 7. Fallback Chain Update

The provider failover chain (ADR-030) extends to include OpenRouter as a cost-effective middle tier:

```
Task Classification → Provider Selection:

Interactive (user-facing):   Sonnet → Opus → OpenRouter(deepseek-r1) → Haiku
Batch (analysis, summary):   OpenRouter(llama-4-scout) → MiniMax → Haiku
Complex reasoning:           Opus → OpenRouter(deepseek-r1) → Sonnet
Code generation:             Sonnet → OpenRouter(llama-4-maverick) → MiniMax
Budget-constrained:          OpenRouter(llama-4-scout) → Local → Haiku
```

## Consequences

### Positive
- **300+ models via single API key** — no per-provider account management
- **RL model selection gets a much richer pool** — better cost/quality optimization
- **Automatic upstream failover** — if Together AI is down, OpenRouter routes to Lambda or Fireworks
- **Accurate cost tracking** — OpenRouter reports actual cost per request
- **Minimal adapter changes** — reuses existing `OpenAiCompatAdapter` wire protocol

### Negative
- **Added latency** — extra hop through OpenRouter adds ~50-100ms per request
- **Pricing markup** — OpenRouter charges a margin over base provider prices
- **Dependency on third-party routing** — OpenRouter outage = no open-source model access
- **Rate limit opacity** — harder to predict/manage limits when they depend on upstream providers

### Mitigations
- **Latency**: Use `route: "fallback"` only for batch; for interactive, prefer direct Anthropic
- **Cost**: OpenRouter markup is typically <5%; the convenience of single-key outweighs this
- **Availability**: Local models (Ollama) remain as zero-dependency fallback; Anthropic/MiniMax direct remain for critical paths
- **Rate limits**: Cache model metadata including rate limit hints; implement exponential backoff with provider rotation

## Implementation Notes

Implemented in:
- `hex-nexus/src/adapters/spacetime_inference.rs` — OpenRouter provider adapter (OpenAI-compatible wire protocol, actual cost tracking from response)
- `hex-cli/src/commands/inference.rs` — `hex inference add/list/test` commands for OpenRouter provider management

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `OpenRouter` to `ProviderKind` enum in inference-gateway WASM module | Pending |
| P2 | Extend `OpenAiCompatAdapter` with OpenRouter headers + cost extraction | Pending |
| P3 | Implement `hex inference discover --provider openrouter` model sync | Pending |
| P4 | Wire OpenRouter cost into `complete_inference()` reducer budget tracking | Pending |
| P5 | Add OpenRouter models to RL model selection pool (ADR-031) | Pending |
| P6 | Update fallback chains in composition root | Pending |
| P7 | Dashboard: show OpenRouter models + per-request cost in inference panel | Pending |

## References

- [OpenRouter API Docs](https://openrouter.ai/docs/api-reference)
- [OpenRouter Model List](https://openrouter.ai/models)
- ADR-030: Multi-Provider Inference Broker (OpenAI-compatible adapter)
- ADR-031: RL-Driven Model Selection & Token Management
- ADR-026: Secure Secret Distribution (API key management)
- ADR-028: API Optimization Layer (rate limiting, caching)
- ADR-035: Architecture V2 — Inference as Pluggable Adapters
