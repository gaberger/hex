# hex-weather — Hexagonal Architecture Demo

Demonstrates hex's architecture enforcement on a multi-layer Rust CLI app.

## Architecture

```
src/
├── domain/mod.rs    Pure value objects. ZERO external deps.
│                    Weather, Temperature, Condition, Forecast
│
├── ports/mod.rs     Trait contracts. Imports only domain.
│                    WeatherService, CliParser, CliRequest
│
├── adapters/mod.rs  Implementations. Imports only ports + domain.
│                    MockWeatherService, EnvCliParser
│
└── main.rs          Composition root. The ONLY file that imports adapters.
                     Wires adapters → ports → use cases.
```

### What hex enforces

| Rule | Enforcement |
|:-----|:-----------|
| Domain imports only domain | `hex analyze` tree-sitter boundary check |
| Ports import only domain | Import path extraction → layer validation |
| Adapters import only ports + domain | Cross-adapter coupling detection |
| Adapters NEVER import other adapters | Blocked at commit time |
| Only main.rs imports adapters | Adapter import source verification |

Violations are caught by `hex analyze .` — not as linting suggestions, but as **blocking gates** that prevent code from shipping.

### How hex builds this (workplan-driven)

The `workplan.json` decomposes the app into 4 phases matching the hex layer dependency order:

```
Phase 0: Domain (Tier 0) ──→ compile gate: rustc --crate-type lib
Phase 0: Ports  (Tier 0) ──→ compile gate: rustc --crate-type lib
Phase 1: Adapters (Tier 1) → compile gate: rustc --crate-type lib
Phase 2: Composition (Tier 2) → compile gate: rustc → binary
```

Each phase has tasks classified by tier:
- **T2** tasks → routed to `qwen2.5-coder:32b` (local Ollama, 11 tok/s)
- Compile gate validates every generated file before the next phase begins
- Error-feedback retry feeds compiler errors back to the model on failure
- RL engine records every outcome for self-improving model selection

### Tier routing in action

```
P1.1 (domain types)     → T2 → qwen2.5-coder:32b → rustc gate → PASS
P2.1 (port traits)      → T2 → qwen2.5-coder:32b → rustc gate → PASS
P3.1 (adapters)         → T2 → qwen2.5-coder:32b → rustc gate → retry if needed
P4.1 (composition root) → T2 → qwen2.5-coder:32b → full compile → binary
```

### Run the pipeline test

```bash
# From the project root, run the smoke test that exercises this pattern:
cd examples/standalone-pipeline-test
./run.sh --verbose            # All tiers, Rust + TypeScript + Go
./run.sh --tier T2 --verbose  # Just the code generation tier
```

## Why this matters

Most AI agent frameworks generate code in a single pass — no architecture awareness, no boundary enforcement, no compile validation. The result works until it doesn't, and debugging requires a human to trace import violations across files.

hex enforces the architecture at every step:
1. **Workplan decomposes by layer** — domain before ports before adapters
2. **Compile gates block phase advancement** — broken code can't proceed
3. **Error-feedback retries fix errors automatically** — compiler output fed back to model
4. **Architecture analysis validates boundaries** — `hex analyze .` on every commit
5. **RL learns from every dispatch** — system gets better over time, no human tuning

The result: AI agents produce architecturally clean code — not because they understand architecture, but because the runtime won't let them do anything else.
