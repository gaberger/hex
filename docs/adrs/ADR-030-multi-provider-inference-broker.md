# ADR-030: Multi-Provider Inference Broker

**Status:** Accepted
**Date:** 2026-03-18
**Deciders:** Gary
**Relates to:** ADR-028 (API Optimization), ADR-029 (Haiku Preflight), ADR-026 (Secret Distribution)

## Context

Using Anthropic's API exclusively is expensive at scale. A typical hex-agent swarm session costs $2-4 via Sonnet, $15+ via Opus. Many workloads (code analysis, summarization, test generation, dead-code detection) don't require Anthropic-tier quality and can run on cheaper providers.

MiniMax M2.5 offers near-Opus quality (80.2% SWE-Bench Verified) at **10-50x lower cost** ($0.30/M input, $1.20/M output). Critically, it exposes an **OpenAI-compatible API**, meaning we can integrate it without protocol-level changes.

hex-agent already has:
- `InferenceDiscoveryPort` + `InferenceProvider::OpenaiCompatible` (ADR-026)
- `ModelSelection` enum with `Local` variant for alternative backends
- `WorkloadRouter` (ADR-028) that classifies interactive vs batch workloads
- `RateLimiterPort` (ADR-028) for per-model throttling

What's missing is an adapter that speaks OpenAI chat completions and normalizes responses into our `AnthropicResponse` type, plus the routing logic to select providers automatically.

## Decision

### 1. OpenAI-Compatible Adapter

New secondary adapter `OpenAiCompatAdapter` that implements `AnthropicPort` by translating to/from OpenAI's chat completion format:

```
AnthropicPort::send_message()
  → translate to OpenAI chat completion request
  → POST {base_url}/v1/chat/completions
  → translate response back to AnthropicResponse
```

This adapter works with any OpenAI-compatible provider: MiniMax, Together AI, Groq, local vLLM/Ollama, OpenRouter.

### 2. Inference Broker (Composition-Level Routing)

Rather than a new port, routing is handled at the **composition root** level. The `WorkloadRouter` (ADR-028) already classifies workloads. We extend `ModelSelection` to include provider-aware variants and let the composition root wire different adapters per model tier:

```rust
enum ModelSelection {
    Opus,           // → Anthropic adapter
    Sonnet,         // → Anthropic adapter
    Haiku,          // → Anthropic adapter
    MiniMax,        // → OpenAI-compat adapter (MiniMax M2.5)
    MiniMaxFast,    // → OpenAI-compat adapter (M2.5-Lightning)
    Local,          // → OpenAI-compat adapter (Ollama/vLLM)
}
```

### 3. Provider Failover

When MiniMax is unavailable, fall back to Anthropic. The existing fallback chain in `ConversationLoop` is extended:

```
MiniMax → MiniMaxFast → Sonnet → Haiku → Local
```

### 4. Response Normalization

The adapter normalizes OpenAI's response format to our domain types:

| OpenAI field | hex-agent field |
|-------------|----------------|
| `choices[0].message.content` | `ContentBlock::Text` |
| `choices[0].message.tool_calls` | `ContentBlock::ToolUse` |
| `usage.prompt_tokens` | `TokenUsage.input_tokens` |
| `usage.completion_tokens` | `TokenUsage.output_tokens` |
| `choices[0].finish_reason: "tool_calls"` | `StopReason::ToolUse` |
| `<think>` tags in content | Stripped, logged as thinking |

### 5. CLI Integration

```
--provider minimax|anthropic|auto   Provider selection (default: auto)
--minimax-model MiniMax-M2.5        MiniMax model name
```

`auto` mode: use MiniMax for batch/analysis, Anthropic for interactive.

## Cost Impact

| Scenario | Anthropic-only | With MiniMax | Savings |
|----------|---------------|-------------|---------|
| Single interactive session | $2.25 | $2.25 (Sonnet) | 0% |
| 10-agent swarm (code analysis) | $22.50 | $2.10 (MiniMax) | **91%** |
| Bulk summarization (100 files) | $15.00 | $1.50 (MiniMax) | **90%** |
| Mixed session (interactive + analysis) | $8.00 | $3.00 | **63%** |

## Files

### New
- `src/adapters/secondary/openai_compat.rs` — OpenAI-compatible adapter implementing `AnthropicPort`
- `docs/adrs/ADR-030-multi-provider-inference-broker.md`

### Modified
- `src/ports/rl.rs` — Add `MiniMax` + `MiniMaxFast` to `ModelSelection`
- `src/main.rs` — Wire MiniMax adapter, add `--provider` CLI arg
- `src/usecases/conversation.rs` — Extend fallback chain

## Consequences

### Positive
- **90% cost reduction** on batch workloads
- **Provider redundancy** — if Anthropic is rate-limited or down, MiniMax continues working
- **Same adapter works for any OpenAI-compatible API** — Together AI, Groq, local models
- **No port changes** — the `AnthropicPort` trait is provider-agnostic by design

### Negative
- **Two API keys to manage** — `ANTHROPIC_API_KEY` + `MINIMAX_API_KEY`
- **Subtle behavior differences** — tool_use format, stop reasons, thinking tags may differ
- **No prompt caching on MiniMax** — our ADR-028 caching only works with Anthropic's API

### Risks
- MiniMax's tool_use implementation may have edge cases vs Anthropic's
- Response quality on complex multi-turn reasoning may differ
- `<think>` tag handling needs careful testing to avoid leaking thinking content

## References
- [MiniMax OpenAI-Compatible API](https://platform.minimax.io/docs/api-reference/text-openai-api)
- [MiniMax M2.5 Pricing](https://platform.minimax.io/docs/pricing/overview)
- [MiniMax M2.5 Capabilities](https://www.minimax.io/news/minimax-m25)
