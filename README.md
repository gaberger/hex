# hex — AI-Assisted Integrated Development Environment

hex is an **AAIDE** — an opinionated development framework built around **hexagonal architecture** (Ports & Adapters). It is not an application you deploy; it is the framework + CLI toolchain that gets installed into target projects to enforce architecture and coordinate AI-driven development.

## System Components

| Component | Role |
|-----------|------|
| **hex-cli** | CLI binary — all `hex` commands, MCP server |
| **hex-nexus** | Filesystem bridge daemon + dashboard (port 5555) |
| **hex-agent** | Architecture enforcement runtime for AI agents |
| **hex-core** | Shared domain types and port traits (zero external deps) |
| **spacetime-modules** | 18 WASM modules for coordination, inference, swarms |

SpacetimeDB must be running. All clients connect via WebSocket for real-time state sync.

## Quick Start

```bash
# Build
cargo build -p hex-cli
cargo build -p hex-nexus --release

# Start the nexus daemon
hex nexus start

# Check status
hex status
hex nexus status
```

## Inference Routing

hex includes quantization-aware inference routing (ADR-2603271000). Inference requests are automatically routed to the best available provider based on task complexity and provider quality scores.

### How it works

1. **Complexity scoring** — every inference request is scored on prompt length, cross-file keywords, security signals, and architectural keywords
2. **Minimum tier selection** — complexity maps to a minimum quantization tier (Low→Q2, Medium→Q4, High→Q8, Critical→Cloud)
3. **Quality-ranked selection** — providers meeting the minimum tier are ranked by calibrated `quality_score` (0.0–1.0)
4. **Escalation on failure** — if a provider fails, the router retries at the next tier (Q4→Q8→Fp16→Cloud)

### Quantization tiers

| Tier | Bits | Use cases |
|------|------|-----------|
| Q2 | 2-bit | Scaffolding, docstrings, formatting |
| Q4 | 4-bit | General coding, test generation |
| Q8 | 8-bit | Complex reasoning, security review |
| Fp16 | 16-bit | Cross-file planning |
| Cloud | — | Frontier APIs (Anthropic, OpenAI, OpenRouter) |

### Provider registry

```bash
# Discover and register OpenRouter models
hex inference discover --provider openrouter

# List all registered providers with quality scores
hex inference list

# Calibrate a provider (runs real inference, writes quality_score)
hex inference test <provider-id>

# Batch-calibrate all uncalibrated providers
# POST http://localhost:5555/api/neural-lab/experiments/quant-calibration

# Add a local Ollama provider (quantization auto-detected from model tag)
hex inference add ollama http://localhost:11434 llama3.2:3b-q4_k_m
```

Quality scores are stored in SpacetimeDB and used immediately for routing. Once calibrated, the model router uses `SelectionSource::RegistryRanked` — no hardcoded model strings.

### Agent quantization policy

Agent YAMLs declare quantization requirements:

```yaml
# hex-coder.yml — routine coding
inference:
  quantization:
    default: q4
    minimum: q2
    on_complexity_high: q8
    on_failure: cloud

# planner.yml — architectural planning
inference:
  quantization:
    default: q8
    minimum: q4
    on_complexity_high: cloud
    on_failure: cloud
```

## Development Pipeline

Features follow a specs-first lifecycle:

```
ADR → Specs → Workplan → Code (TDD, parallel worktrees) → Validate → Merge
```

```bash
hex adr list              # List all ADRs
hex adr status <id>       # Show ADR detail
hex swarm init <name>     # Start a swarm
hex task list             # List swarm tasks
hex memory store <k> <v>  # Persist key-value
hex inbox list            # Agent notification inbox
```

## Architecture Rules

Enforced by `hex analyze .`:

- `domain/` — pure business logic, no external imports
- `ports/` — typed interfaces, imports from domain only
- `adapters/secondary/` — driven adapters, import from ports only
- `adapters/primary/` — driving adapters, import from ports only
- **Adapters must never import other adapters**
- `composition-root` is the only file that imports from adapters

```bash
hex analyze .             # Architecture health check
cargo test -p hex-nexus --test quant_routing   # Routing tests (19/19)
cargo check -p hex-cli -p hex-nexus -p hex-core
```

## Key ADRs

| ADR | Description | Status |
|-----|-------------|--------|
| ADR-2603271000 | Quantization-Aware Inference Routing | Implemented |
| ADR-2603261000 | Secure Inference Provider Registry | Implemented |
| ADR-2603301200 | Architecture Context Injection (ACI) | Active |
| ADR-2603300100 | hex-agent SpacetimeDB WebSocket Client | Active |
| ADR-2603301600 | Batch Command Execution Context Indexing | Active |
| ADR-027 | HexFlo Native Coordination | Implemented |
