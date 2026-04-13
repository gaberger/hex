---
name: hex-inference
description: Manage inference providers (Ollama, vLLM, OpenAI-compatible) and map local models to hex agent tiers. Use when the user asks to "add ollama", "use local model", "configure inference", "qwen", "minimax", "ollama setup", "model mapping", or "inference provider".
---

# Hex Inference — Local & Remote Model Management

hex is model-agnostic. It works with frontier models (Anthropic, OpenAI), self-hosted models (Ollama, vLLM), and free models. This skill guides inference provider setup, model-to-agent tier mapping, and capability-aware agent assignment.

## Quick Start

```bash
# Discover Ollama instances on your network
hex inference discover

# Register a provider
hex inference add ollama http://localhost:11434 --model qwen3:32b

# Test connectivity + latency
hex inference test http://localhost:11434

# List all providers
hex inference list
```

## Provider Types

| Type | Protocol | Auth | Use Case |
|------|----------|------|----------|
| `ollama` | Ollama API + OpenAI-compat `/v1/` | None | Local/LAN self-hosted |
| `vllm` | OpenAI-compat `/v1/` | Optional API key | GPU server, high throughput |
| `openai-compat` | OpenAI `/v1/` | API key | Any OpenAI-compatible endpoint |

## Environment Variables

| Variable | Purpose | Example |
|----------|---------|---------|
| `HEX_OLLAMA_HOST` | Ollama base URL | `http://bazzite.local:11434` |
| `HEX_OLLAMA_MODEL` | Default Ollama model | `qwen3:32b` |
| `HEX_VLLM_HOST` | vLLM base URL | `http://gpu-server:8000` |
| `HEX_VLLM_MODEL` | Default vLLM model | `Qwen/Qwen3-32B` |
| `HEX_INFERENCE_URL` | Generic inference URL | `http://host:8080` |
| `HEX_INFERENCE_MODEL` | Generic model name | `default` |
| `ANTHROPIC_API_KEY` | Anthropic API (frontier) | `sk-ant-...` |

## Model-to-Agent Tier Mapping

hex agents have a `model_tier` field (1-3) that determines which model class they need. When using local models, map tiers to your available models:

### Tier Definitions

| Tier | Capability Needed | Frontier Model | Minimum Local Model |
|------|-------------------|----------------|---------------------|
| **1** (fast) | Simple lookups, status checks, formatting | Haiku | Any 1B-7B (qwen3:4b, phi4-mini) |
| **2** (balanced) | Code generation, review, analysis | Sonnet | 14B-32B (qwen3:32b, qwen3.5:27b, minimax-m1:80b) |
| **3** (deep) | Planning, architectural reasoning, validation | Opus | 70B+ (qwen3:235b, deepseek-r1:70b) or use frontier |

### Agent → Tier Mapping

| Agent | Tier | Why | Can Run Local? |
|-------|------|-----|----------------|
| status-monitor | 1 | Simple status aggregation | Yes — any model |
| dev-tracker | 1 | Progress tracking | Yes — any model |
| scaffold-validator | 1 | File existence checks | Yes — any model |
| hex-coder | 2 | Code generation within one adapter | Yes — 14B+ recommended |
| dead-code-analyzer | 2 | Pattern matching + grep | Yes — 14B+ |
| adr-reviewer | 2 | Template validation | Yes — 14B+ |
| dependency-analyst | 2 | Dependency analysis | Yes — 14B+ |
| rust-refactorer | 2 | Rust-specific refactoring | Yes — 32B+ recommended |
| integrator | 2 | Merge + test orchestration | Yes — 14B+ |
| behavioral-spec-writer | 3 | Domain reasoning for specs | Risky — needs frontier or 70B+ |
| planner | 3 | Architectural decomposition | Risky — needs frontier or 70B+ |
| validation-judge | 3 | Deep semantic validation | Risky — needs frontier or 70B+ |
| swarm-coordinator | 3 | Multi-agent orchestration | Risky — needs frontier or 70B+ |
| feature-developer | 3 | Full lifecycle orchestration | Risky — needs frontier or 70B+ |

### Recommended Model Configurations

#### Budget: Ollama Only (No API Keys)

```bash
# Use a capable open model for coding
hex inference add ollama http://localhost:11434 --model qwen3:32b

# Agent mapping
# Tier 1-2: qwen3:32b (handles most coding tasks)
# Tier 3: qwen3:32b with /think mode, or qwen3:235b if you have 128GB+ RAM
```

**Limitations**:
- Tier 3 agents (planner, validation-judge) will produce lower-quality plans
- Behavioral specs may miss edge cases
- Swarm coordination may be less reliable
- Workplans may have dependency errors

**Mitigations**:
- Use **Interactive mode** instead of Swarm mode (human reviews each phase)
- Run `hex analyze .` after each coding step (catches boundary violations)
- Review behavioral specs manually before workplan creation
- Use `hex plan validate` to catch workplan structural issues

#### Hybrid: Ollama + Frontier API

```bash
# Local model for coding (tier 1-2)
hex inference add ollama http://localhost:11434 --model qwen3:32b

# Frontier for planning/validation (tier 3)
# Set ANTHROPIC_API_KEY in environment
```

**Best of both worlds**:
- Tier 1-2 agents run locally (free, fast, private)
- Tier 3 agents use Anthropic API (high quality planning/validation)
- Token cost reduced by ~70% vs. all-frontier

#### Full Frontier (Anthropic API)

```bash
# All agents use Claude models
# Set ANTHROPIC_API_KEY in environment
# No hex inference setup needed — uses Anthropic directly
```

### MiniMax Models

MiniMax models (minimax-m1) are available through Ollama:

```bash
# Pull MiniMax model
ollama pull minimax-m1:80b

# Register with hex
hex inference add ollama http://localhost:11434 --model minimax-m1:80b
```

**MiniMax tier mapping**:
- `minimax-m1:80b` → Tier 2-3 (strong reasoning, competitive with frontier for coding)
- Good for: hex-coder, planner, behavioral-spec-writer
- Caveat: Context window may be smaller than frontier — reduce `token_budget.max` in agent definitions

### Qwen Models

```bash
# Qwen 3 (recommended for hex)
ollama pull qwen3:32b        # Tier 2 — best balance of speed/quality
ollama pull qwen3:4b          # Tier 1 — fast, status checks only
ollama pull qwen3:235b        # Tier 3 — if you have the VRAM/RAM

# Qwen 3.5 (latest)
ollama pull qwen3.5:27b       # Tier 2 — newer, potentially better coding

# Register the primary model
hex inference add ollama http://localhost:11434 --model qwen3:32b
```

## Configuring hex-agent for Local Models

When running hex-agent with local inference:

```bash
# Set the Ollama endpoint
export HEX_OLLAMA_HOST=http://localhost:11434
export HEX_OLLAMA_MODEL=qwen3:32b

# Run hex-agent
hex-agent --project-dir .
```

### Context Window Adjustments

Local models typically have smaller context windows. Adjust agent token budgets:

| Model | Context Window | Recommended `token_budget.max` |
|-------|---------------|-------------------------------|
| qwen3:4b | 32K | 20000 |
| qwen3:32b | 128K | 60000 |
| qwen3:235b | 128K | 80000 |
| minimax-m1:80b | 128K | 60000 |
| Claude Sonnet | 200K | 80000 |
| Claude Opus | 200K+ | 120000 |

When using a local model with a smaller context window:
- Use L0/L1 summaries instead of L2 (less context consumed)
- Break features into smaller steps
- Reduce the number of parallel agents (fewer concurrent context windows)

## Inference Routing

The hex nexus daemon routes inference requests based on provider priority:

1. **Registered providers** (via `hex inference add`) — checked first
2. **Environment variables** (`HEX_OLLAMA_HOST`, etc.) — fallback
3. **Anthropic API** (`ANTHROPIC_API_KEY`) — final fallback

When SpacetimeDB is running, the `inference-gateway` WASM module handles routing. The module stores request/response state; the hex nexus daemon performs the actual HTTP calls (WASM can't make network requests).

## Troubleshooting

| Issue | Fix |
|-------|-----|
| "Connection refused" | Ensure Ollama is running: `ollama serve` |
| "Connection timeout" | Ollama may be bound to localhost. Fix: `OLLAMA_HOST=0.0.0.0 ollama serve` |
| Slow inference | Check GPU utilization. Ollama defaults to CPU if no GPU detected |
| Out of memory | Use a smaller model or enable partial offloading |
| Model not found | Pull it first: `ollama pull qwen3:32b` |

## Quick Reference

| Command | What it does |
|---------|-------------|
| `hex inference discover` | Auto-discover Ollama instances on LAN |
| `hex inference add ollama <url> --model <model>` | Register Ollama provider |
| `hex inference add vllm <url> --model <model>` | Register vLLM provider |
| `hex inference list` | List all registered providers |
| `hex inference test <url>` | Test connectivity + run inference probe |
| `hex inference remove <id>` | Remove a provider |
