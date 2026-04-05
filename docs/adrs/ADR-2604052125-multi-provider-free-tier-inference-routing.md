# ADR-2604052125: Multi-Provider Free-Tier Inference Routing

**Status:** Accepted
**Date:** 2026-04-05
**Deciders:** Gary, Claude
**Relates to:** ADR-030 (Multi-Provider Inference Broker), ADR-031 (RL-Driven Model Selection), ADR-2603271000 (Quantization-Aware Inference Routing), ADR-2603231600 (OpenRouter Integration)

## Context

### The Cost Problem

Frontier model inference (Anthropic Opus, OpenAI GPT-4o) costs $15-75/M output tokens. A 10-agent swarm running code generation burns through $20+ per session. The RL engine (ADR-031) already achieves 63-91% cost reduction by routing simple tasks to cheaper models, but "cheaper" still means paid when the only registered providers are Anthropic and OpenRouter.

### The Free Tier Landscape (2026)

Multiple inference providers now offer **free tiers with OpenAI-compatible APIs**. These aren't toy models — they serve Llama 4 Scout, Qwen3 235B, and DeepSeek-Coder-V3 at production speeds:

| Provider | Free Quota | Speed | Best Models |
|:---------|:-----------|:------|:------------|
| **Cerebras** | 1M tokens/day, 30 RPM | 3000+ tok/s | Llama 3.3 70B, Qwen3 235B, GPT-OSS 120B |
| **Groq** | 30 RPM, 1K daily (70B) | 300-1000 tok/s | Llama 4 Scout, Qwen3 32B, Kimi K2 |
| **SambaNova** | $5 credit + rate-limited free | Very fast | Llama 3 (8B-405B), DeepSeek V3 |
| **Together.ai** | $25 signup credit | 2x serverless | DeepSeek V3, Llama 4 Maverick |
| **OpenRouter** | 50 req/day (1K with $10 spend) | Varies | 28 free models incl DeepSeek R1 |
| **Ollama (local)** | Unlimited | Hardware-bound | Qwen3-Coder, DeepSeek-Coder-V2 |

Combined, these provide **~2M+ free tokens/day** and **300+ free requests/day** — enough to run multi-agent swarms at zero marginal cost for development workloads.

### What hex Has Today

The inference architecture (ADR-030, ADR-035) is already provider-agnostic:

- All providers register via `hex inference add <type> <url> --model <name>`
- Quantization-aware routing (ADR-2603271000) selects providers by tier
- RL engine (ADR-031) learns optimal `(model, context_strategy)` pairs per task
- OpenAI-compatible adapter handles Groq, Cerebras, Together, SambaNova without code changes

What's **missing** is the intelligence to exploit free tiers across providers simultaneously:

1. **No sliding-window rate limit tracking** — when Groq hits 30 RPM, hex retries instead of shifting to Cerebras
2. **No circuit breaker** — a provider returning 429s gets hammered until timeout
3. **No free-tier-aware scheduling** — Cerebras's 1M tokens/day should absorb batch work overnight
4. **No provider templates** — users must manually discover base URLs, model names, and rate limits for each provider
5. **No cost attribution** — can't compare actual spend vs. what frontier-only would have cost

## Decision

### 1. Provider Template Registry

Ship built-in templates for known free-tier providers. Users provide only an API key:

```bash
# Instead of this:
hex inference add openai-compatible https://api.groq.com/openai/v1 \
  --model llama-4-scout-17b-16e-instruct --key $GROQ_KEY --quantization cloud

# Just this:
hex inference add groq --key $GROQ_KEY
# Auto-registers all available models with correct base URL, rate limits, and tiers
```

Templates stored in `hex-cli/assets/inference-providers/` as YAML:

```yaml
# groq.yml
name: groq
display_name: Groq
base_url: https://api.groq.com/openai/v1
api_key_env: GROQ_API_KEY
openai_compatible: true
rate_limits:
  rpm: 30
  daily_requests: 1000
  daily_tokens: null
models:
  - id: llama-4-scout-17b-16e-instruct
    tier: cloud
    context_window: 131072
    coding_optimized: false
  - id: qwen-qwq-32b
    tier: cloud
    context_window: 131072
    coding_optimized: true
  - id: meta-llama/llama-4-scout-17b-16e-instruct
    tier: cloud
    context_window: 131072
    coding_optimized: false
```

Initial templates: `groq.yml`, `cerebras.yml`, `sambanova.yml`, `together.yml`, `openrouter.yml`, `ollama.yml`.

### 2. Sliding-Window Rate Limit Tracker

Track per-provider rate consumption in a 60-second sliding window:

```rust
struct ProviderRateState {
    provider_id: String,
    requests_this_minute: u32,
    tokens_this_minute: u64,
    requests_today: u32,
    tokens_today: u64,
    last_reset: Instant,
    daily_reset: DateTime<Utc>,
}
```

Before dispatching a request, check `should_route(provider_id) -> bool`. If the provider is at or near its limit (>80% RPM consumed), **preemptively route to the next provider** instead of waiting for a 429.

Store rate state in hex-nexus memory (not SpacetimeDB — this is ephemeral, per-instance state). Reset daily counters at midnight UTC.

### 3. Circuit Breaker Per Provider

Three states: **Closed** (normal) → **Open** (failing, skip for cooldown) → **Half-Open** (test one request):

| Condition | Action |
|:----------|:-------|
| 3 consecutive failures (429, 5xx, timeout) | Open circuit → 5-minute cooldown |
| Cooldown expires | Half-open → send 1 probe request |
| Probe succeeds | Close circuit → resume normal routing |
| Probe fails | Re-open → 10-minute cooldown (exponential backoff, max 30 min) |

This prevents hammering a provider that's rate-limiting or down, while auto-recovering when it comes back.

### 4. Free-Tier-Aware Routing Strategy

Extend the quantization router with a **cost-aware provider selection policy**:

```
Priority order for equivalent-tier providers:
1. Free provider with remaining daily quota (Cerebras, Groq, SambaNova)
2. Free provider in half-open circuit (testing recovery)
3. Local provider (Ollama, vLLM) — zero cost, higher latency
4. Paid provider with lowest $/token (OpenRouter free models, MiniMax)
5. Frontier provider (Anthropic, OpenAI) — only when lower tiers exhausted or task is complex
```

For **batch/non-interactive** work (summarization, docstring generation, test scaffolding):
- Route to Cerebras first (1M tokens/day, fastest throughput)
- Overflow to Together.ai / SambaNova

For **interactive** work (agent conversations, real-time code generation):
- Route to Groq first (lowest latency)
- Overflow to Cerebras → local

The RL engine's reward function already penalizes cost. Exposing provider cost metadata (`cost_per_input_token`, `cost_per_output_token`, `is_free_tier`) lets the RL agent learn these preferences organically.

### 5. Cost Attribution Dashboard

Track and report:

| Metric | Source |
|:-------|:-------|
| Actual cost per request | Provider response headers or token count × rate |
| Counterfactual cost | "What this would have cost on Opus" |
| Savings percentage | `1 - (actual / counterfactual)` |
| Free tier utilization | `tokens_used_today / daily_limit` per provider |
| Provider distribution | Pie chart of requests by provider |

Surface in `hex status` CLI output and dashboard inference panel.

### 6. Auto-Discovery Enhancement

Extend `hex inference discover` to probe known free-tier providers:

```bash
hex inference discover --free
# Checks: Groq, Cerebras, SambaNova, Together, OpenRouter
# For each: validates API key from env, lists available models, reports rate limits
# Registers all discovered providers automatically
```

Environment variable convention: `GROQ_API_KEY`, `CEREBRAS_API_KEY`, `SAMBANOVA_API_KEY`, `TOGETHER_API_KEY`, `OPENROUTER_API_KEY`.

## Consequences

### Positive

- **Zero-cost development inference** — 2M+ free tokens/day across providers
- **Automatic failover** — rate limit on one provider shifts to the next in <100ms
- **RL learns provider preferences** — no manual tuning of routing rules
- **One-command setup** — `hex inference add groq --key $KEY` vs. manual URL/model configuration
- **Cost visibility** — teams see exactly what they're saving vs. frontier-only

### Negative

- **Free tier instability** — providers reduce limits under load (Groq seasonally cuts quotas). Mitigation: circuit breaker + local fallback.
- **Model quality variance** — Qwen3 32B on Groq ≠ Claude Opus. Mitigation: RL reward penalizes low-quality completions; frontier remains available for complex reasoning.
- **Provider template maintenance** — base URLs, model names, and rate limits change. Mitigation: templates are YAML in hex-cli/assets, trivially updatable per release.
- **Ephemeral rate state** — rate limit tracking resets on nexus restart. Mitigation: daily counters could persist to SpacetimeDB if needed, but transient loss is acceptable.

### Risks

- **Provider deprecation** — free tiers could disappear. Mitigation: always maintain local fallback (Ollama/vLLM) and at least one paid provider.
- **Terms of service** — some free tiers prohibit automated/bulk usage. Mitigation: respect rate limits, implement backoff, read provider ToS.

## Implementation

### Phase 1: Provider Templates + Auto-Discovery (2 days)
- Create YAML templates in `hex-cli/assets/inference-providers/` for 6 providers
- Implement `hex inference add <template-name> --key <key>` shorthand
- Implement `hex inference discover --free` to probe all known providers

### Phase 2: Rate Limit Tracker + Circuit Breaker (2 days)
- Add `ProviderRateState` to hex-nexus in-memory state
- Implement sliding-window RPM/TPM tracking
- Add circuit breaker (Closed/Open/Half-Open) per provider
- Integrate with quantization router's `select_provider()`

### Phase 3: Free-Tier-Aware Routing (1 day)
- Add `cost_per_input_token`, `cost_per_output_token`, `is_free_tier` to `InferenceProvider` table
- Update `select_provider()` to prefer free providers at equivalent tier
- Expose cost metadata to RL engine reward function

### Phase 4: Cost Attribution (1 day)
- Track actual vs. counterfactual cost per request in inference-gateway
- Add `hex inference stats` CLI command
- Add provider distribution chart to dashboard inference panel
