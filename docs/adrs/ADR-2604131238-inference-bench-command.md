# ADR-2604131238: Inference Bench Command

**Status:** Accepted
**Date:** 2026-04-13
**Drivers:** Manual, ad-hoc model evaluation (curl + python scripts) is slow and not reproducible. Every new model addition requires re-inventing the same quality/latency/tier analysis. This should be a first-class CLI command.
**Supersedes:** None

## Context

When evaluating a new inference model (e.g. MiniMax M2.7 on Bazzite), the current workflow is:

1. `hex inference add` — register the endpoint
2. `hex inference test` — basic connectivity + single "reply ok" probe
3. **Manual curl + Python** — run code generation prompts, measure tok/s, check quality signals, compare against other models, decide tier mapping

Step 3 is entirely manual. It requires:
- Crafting benchmark prompts (identity, code-gen, reasoning)
- Measuring wall time, tokens, tok/s
- Checking quality signals (does it use async? thiserror? proper derives? tests?)
- Comparing against a baseline model
- Deciding the hex agent tier (1=Haiku, 2=Sonnet, 3=Opus equivalent)

This is repeatable, structured work that belongs in `hex inference bench`.

### Forces

- **Reproducibility**: Same prompts, same scoring, every time
- **Speed**: One command vs. 10 minutes of curl/python scripting
- **Tier mapping**: hex agents need to know which tier a model fits — bench should recommend this
- **Comparison**: Side-by-side benchmarks against a baseline model are common
- **Cloud vs local**: Cloud-proxied models (Ollama `:cloud` tags) have different latency profiles than local GGUF models — bench must handle both

### Alternatives Considered

1. **External benchmark tool** (e.g. llm-bench, ollama-bench): Doesn't integrate with hex tier system or hex-specific code quality checks
2. **Extend `hex inference test`**: Test is about connectivity/health; bench is about capability evaluation — different concerns
3. **Web dashboard benchmark**: Too heavy; CLI is the right interface for quick model evaluation

## Decision

We will add `hex inference bench <target>` as a new subcommand of `hex inference`.

### CLI Interface

```
hex inference bench <target> [--model <name>] [--quick] [--compare <baseline-id>] [--save]
```

- `<target>`: Provider ID, model name, or URL (same resolution as `hex inference test`)
- `--model`: Override the provider's registered model
- `--quick`: Skip the long code-generation prompt (identity + reasoning only)
- `--compare`: Run the same prompts against a baseline model for side-by-side comparison
- `--save`: Persist the quality score and tier recommendation to nexus

### Benchmark Suite (3 prompts)

| Prompt | What it measures | Quality signals |
|--------|-----------------|-----------------|
| **Identity** | Basic responsiveness, latency floor | Non-empty response, correct model awareness |
| **Code generation** | Rust code quality for hex adapter work | async fn, thiserror, tests, reqwest, Result<>, trait def, mock, derives, timeout handling, module structure |
| **Reasoning** | Architecture analysis capability | Identifies layers, boundary violations, provides actionable fix |

### Scoring & Tier Recommendation

Quality score (0.0–1.0) combines:
- **Code quality** (0–10 checklist, weighted 0.5)
- **Reasoning quality** (0–5 checklist, weighted 0.3)
- **Latency** (tok/s brackets, weighted 0.2)

Tier recommendation:
- Score >= 0.85 + reasoning >= 4/5 → **Tier 3** (Opus-equivalent: planning, specs, validation)
- Score >= 0.65 + code >= 7/10 → **Tier 2** (Sonnet-equivalent: code generation, review)
- Score >= 0.40 → **Tier 1** (Haiku-equivalent: simple edits, formatting, summarization)
- Score < 0.40 → **Not recommended** for hex agent work

### Output Format

```
── hex inference bench: minimax-m2.7:cloud via bazzite-ollama ──

  Identity     ✓  1.2s  (responsive, 38 tok/s)
  Code-gen     ✓  127s  (10/10 quality, 128 tok/s)
  Reasoning    ✓  45s   (4/5 quality, 95 tok/s)

  ── Summary ──────────────────────────────────
  Overall score:  0.87
  Avg tok/s:      87.0
  Recommended tier: 2 (Sonnet-equivalent)
  Best for: code_generation, code_edit

  ✓ Saved calibration to hex-nexus
```

## Consequences

**Positive:**
- One-command model evaluation — no more ad-hoc curl/python scripts
- Reproducible benchmarks with consistent prompts and scoring
- Automatic tier recommendation integrates with hex agent model router
- Side-by-side comparison makes model selection data-driven

**Negative:**
- Benchmark prompts are hex-specific (Rust, hexagonal architecture) — not general-purpose LLM benchmarks
- Cloud-proxied models have variable latency that may not reflect steady-state performance
- Quality scoring is heuristic-based, not ground-truth verified

**Mitigations:**
- Document that benchmarks are hex-specific and measure "fitness for hex agent work"
- Run benchmarks 2-3 times for cloud models and use median
- Quality checklist is extensible — can add more signals over time

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `Bench` variant to `InferenceAction` enum + CLI arg parsing | Done |
| P2 | Implement `bench_provider()` — target resolution (reuse from `test_provider`) | Done |
| P3 | Implement 3 benchmark prompts with quality signal extraction | Done |
| P4 | Scoring algorithm + tier recommendation logic | Done |
| P5 | `--compare` baseline support | Done |
| P6 | `--save` calibration persistence to nexus | Done |
| P7 | MCP tool `hex_inference_bench` + smoke test | Done |

## References

- ADR-2604120202: Tiered inference routing (tier→model mapping)
- ADR-2604130010: Worker local inference discovery
- `hex-cli/src/commands/inference.rs`: Existing inference subcommands
- `hex-cli/src/pipeline/model_selection.rs`: TaskType + tier mapping
