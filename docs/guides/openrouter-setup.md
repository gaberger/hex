# OpenRouter Setup Guide

OpenRouter provides access to 300+ open-source and commercial models (Llama 4, DeepSeek R1, Qwen 3, Gemini, Mistral, etc.) through a single API key and OpenAI-compatible endpoint. hex treats OpenRouter as a first-class inference provider with special support for cost tracking, provider routing, and RL model selection.

> **ADR**: [ADR-2603231600 — OpenRouter Inference Integration](../adrs/ADR-2603231600-openrouter-inference-integration.md)

## Prerequisites

- hex-nexus running (`hex nexus start`)
- An OpenRouter account at [openrouter.ai](https://openrouter.ai)
- An API key from OpenRouter → Settings → Keys

## Quick Start

```bash
# 1. Store your API key
hex secrets set OPENROUTER_API_KEY sk-or-v1-your-key-here

# 2. Discover available models
hex inference discover --provider openrouter

# 3. Verify a model works
hex inference test openrouter-meta-llama-llama-4-maverick

# 4. List all registered providers
hex inference list
```

That's it. Models are now available to hex agents via the inference layer.

## Step-by-Step Setup

### 1. Store the API Key

hex uses a secrets vault (managed by hex-nexus) so keys aren't stored in environment variables or `.env` files.

```bash
hex secrets set OPENROUTER_API_KEY sk-or-v1-your-key-here
```

Verify it's stored:

```bash
hex secrets status
```

You should see `OPENROUTER_API_KEY` listed as available.

**Alternative** — if you prefer environment variables (e.g., in CI):

```bash
export OPENROUTER_API_KEY=sk-or-v1-your-key-here
```

The CLI checks `hex secrets` first, then falls back to the environment variable.

### 2. Discover Models

The discover command fetches the full model catalog from OpenRouter and registers matching models with hex-nexus:

```bash
# All available models
hex inference discover --provider openrouter

# Filter by name (e.g., only Llama models)
hex inference discover --provider openrouter --filter llama

# Only models with at least 128K context
hex inference discover --provider openrouter --min-context 128000

# Combine filters
hex inference discover --provider openrouter --filter deepseek --min-context 64000
```

Each discovered model is registered as a provider with the ID pattern `openrouter-<org>-<model>` (e.g., `openrouter-meta-llama-llama-4-maverick`).

### 3. Manually Register a Specific Model

If you want to add a single model without running discovery:

```bash
hex inference add openrouter https://openrouter.ai/api/v1 \
  --model meta-llama/llama-4-maverick \
  --key sk-or-v1-your-key-here
```

### 4. Test Connectivity

```bash
hex inference test openrouter-meta-llama-llama-4-maverick
```

This sends a small test prompt and confirms the model responds. If it fails, check:
- Is `OPENROUTER_API_KEY` set? (`hex secrets status`)
- Does the model ID match? (`hex inference list`)
- Do you have OpenRouter credits? (check [openrouter.ai/credits](https://openrouter.ai/credits))

### 5. Remove a Provider

```bash
hex inference remove openrouter-meta-llama-llama-4-maverick
```

## Recommended Models

These models offer good cost/quality tradeoffs for development tasks:

| Model ID | Context | Input $/M | Best For |
|----------|---------|-----------|----------|
| `meta-llama/llama-4-maverick` | 1M | $0.25 | General coding, large context |
| `meta-llama/llama-4-scout` | 512K | $0.15 | Summarization, analysis |
| `qwen/qwen3-235b` | 128K | $0.20 | Multilingual, reasoning |
| `deepseek/deepseek-r1` | 128K | $0.55 | Complex reasoning, math |
| `google/gemini-2.5-pro` | 1M | $1.25 | Long-context analysis |
| `mistralai/mistral-large` | 128K | $2.00 | European compliance tasks |

Prices are approximate and vary by upstream provider. OpenRouter reports actual cost per request.

## How It Works

### Architecture

OpenRouter uses the same OpenAI chat completions wire protocol that hex already supports (`OpenAiCompatAdapter`). The adapter detects OpenRouter endpoints and adds:

- **Extra headers**: `HTTP-Referer` and `X-Title: hex-agent` (for OpenRouter analytics)
- **Provider preferences**: `provider.order` for latency optimization, `route: "fallback"` for batch workloads
- **Cost extraction**: OpenRouter returns actual USD cost in `usage.cost` — hex uses this instead of estimating from token counts

### Fallback Chains

hex's inference layer uses fallback chains based on task type. With OpenRouter enabled:

```
Interactive (user-facing):   Sonnet → Opus → OpenRouter(deepseek-r1) → Haiku
Batch (analysis, summary):   OpenRouter(llama-4-scout) → MiniMax → Haiku
Complex reasoning:           Opus → OpenRouter(deepseek-r1) → Sonnet
Code generation:             Sonnet → OpenRouter(llama-4-maverick) → MiniMax
Budget-constrained:          OpenRouter(llama-4-scout) → Local → Haiku
```

### RL Model Selection

When the RL engine (ADR-031) is active, OpenRouter models participate in the model selection pool. The RL reward signal uses the actual cost from OpenRouter (not estimated), so the engine learns which OpenRouter models give the best quality-per-dollar for each task type.

### Cost Tracking

OpenRouter costs appear in:
- **Dashboard** → Inference panel (indigo badge, "Actual Cost" label)
- **SpacetimeDB** → `inference_response` table (`openrouter_cost_usd` field)
- **RL engine** → reward signal for model selection optimization

## Dashboard

The Inference panel at `http://localhost:5555` shows OpenRouter providers with an indigo badge. When a request uses OpenRouter, the cost column displays the actual cost reported by OpenRouter rather than an estimate.

## Troubleshooting

### "OPENROUTER_API_KEY not set"

```bash
# Check if it's in the vault
hex secrets status

# Re-set it
hex secrets set OPENROUTER_API_KEY sk-or-v1-...
```

### Discovery finds 0 models

- Check your API key has credits at [openrouter.ai/credits](https://openrouter.ai/credits)
- Try without filters first: `hex inference discover --provider openrouter`
- Check hex-nexus is running: `hex nexus status`

### Model returns 402 (Payment Required)

Your OpenRouter account needs credits. Add credits at [openrouter.ai/credits](https://openrouter.ai/credits).

### Model returns 429 (Rate Limited)

OpenRouter rate limits depend on the upstream provider. The adapter handles retries with exponential backoff automatically. For batch workloads, the `route: "fallback"` setting enables automatic provider rotation.

### High latency

OpenRouter adds ~50-100ms of routing overhead. For latency-sensitive interactive work, the fallback chain prefers direct Anthropic/MiniMax. OpenRouter is best for batch/analysis workloads where the model diversity matters more than raw latency.
