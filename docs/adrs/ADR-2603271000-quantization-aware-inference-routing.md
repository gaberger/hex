# ADR-2603271000: Quantization-Aware Inference Routing

**Status:** Implemented
**Date:** 2026-03-27
**Drivers:** TurboQuant (Google Research) demonstrates sub-4-bit quantization (2-bit) achieving near-FP16 accuracy at 4-8x memory reduction. hex's inference routing treats all registered providers as equivalent — it has no concept of quantization level, cannot prefer a cheaper quantized local model for simple tasks, and cannot upgrade to a full-precision provider when quality demands it.

## Context

### The opportunity TurboQuant unlocks

TurboQuant and its antecedents (GPTQ, AWQ, GGUF Q2_K/Q3_K) show that a 2-4 bit quantized model running locally can match a cloud FP16 model on many routine tasks (code scaffolding, docstring generation, type stub completion) while costing zero tokens and running offline. The performance gap only opens on high-complexity tasks: cross-file reasoning, multi-step planning, security-sensitive generation.

hex's agent YAMLs already encode a `preferred → opus` upgrade path for iteration failures. This ADR adds a **downgrade** path for cost savings and a **quantization tier** axis orthogonal to model identity.

### Current routing has no quality/cost axis

`hex inference add` registers providers (Ollama, OpenRouter, vLLM, Anthropic) but stores no quantization metadata. The routing in `hex-nexus/src/routes/inference.rs` selects a provider by name or falls back down a static list. There is no:

- Awareness that `llama3.2:3b-q2_K` is a 2-bit quantized model vs `llama3.2:3b` (FP16)
- Mechanism to prefer lower quantization for simple tasks and escalate for complex ones
- Integration with Neural Lab to measure quality degradation at each tier and tune thresholds automatically

### What "quantization level" means operationally

| Tier | Bits | Typical format | Memory (7B model) | Use cases |
|------|------|----------------|-------------------|-----------|
| Q2   | 2    | GGUF Q2_K      | ~2 GB             | scaffolding, formatting, docstrings |
| Q3   | 3    | GGUF Q3_K_M    | ~3 GB             | type completion, simple refactors |
| Q4   | 4    | GGUF Q4_K_M    | ~4.5 GB           | general coding, test generation |
| Q8   | 8    | GGUF Q8_0      | ~8 GB             | complex reasoning, security review |
| FP16 | 16   | safetensors    | ~14 GB            | cross-file planning, novel architecture |
| N/A  | —    | Cloud API      | —                 | frontier (Anthropic, OpenAI) |

### Alternatives considered

- **Ignore quantization, trust model identity alone**: Loses the cost/quality optimization surface. A `llama3.2:3b-q2_K` and `llama3.2:3b` are the same model at different fidelity — treating them identically is wrong.
- **Let users manually specify provider per task**: Defeats the purpose of automated routing. Agent YAMLs should express intent; routing should resolve.
- **Only support cloud providers**: Leaves the local-first story incomplete. Many hex users run entirely offline or on air-gapped machines.

## Decision

### 1. QuantizationLevel as a first-class type in hex-core

Add `QuantizationLevel` enum to `hex-core/src/`:

```rust
pub enum QuantizationLevel {
    Q2,       // 2-bit (TurboQuant / GGUF Q2_K)
    Q3,       // 3-bit
    Q4,       // 4-bit (default for local models)
    Q8,       // 8-bit
    Fp16,     // Full precision
    Cloud,    // External API — quantization not applicable
}
```

Implements `Ord` (Q2 < Q3 < Q4 < Q8 < Fp16 < Cloud), `Display` as lowercase string, `FromStr` for YAML/JSON parsing.

### 2. Inference provider records gain quantization_level

Extend the inference provider record (stored in SQLite/SpacetimeDB) with:

```rust
pub struct InferenceProvider {
    // existing fields ...
    pub quantization_level: QuantizationLevel,  // default: Q4 for local, Cloud for APIs
    pub context_window: Option<u32>,             // tokens; used in routing
    pub quality_score: Option<f32>,              // 0.0–1.0; updated by Neural Lab
}
```

`hex inference add` gains `--quantization <level>` flag. For Ollama, auto-detect from model tag (`:q2_k`, `:q4_k_m`, etc.) if not explicitly provided.

### 3. Agent YAML gains quantization policy

Extend agent YAML inference block:

```yaml
inference:
  task_type: code_generation
  model: preferred
  quantization:
    default: q4          # use q4 by default
    minimum: q2          # never go below q2
    on_complexity_high: q8   # escalate when task scored complex
    on_failure: cloud    # upgrade to cloud on repeated failures
```

The supervisor reads `quantization` policy and passes `min_quantization_level` to the hex-nexus routing API with each inference request.

### 4. Routing logic in hex-nexus: complexity-aware provider selection

Add a `QuantizationRouter` in `hex-nexus/src/routes/inference.rs`:

1. **Score task complexity** from the request body: prompt length, number of files referenced, presence of cross-adapter dependency keywords → `ComplexityScore { level: Low | Medium | High | Critical }`.
2. **Map complexity to minimum quantization tier** using the agent YAML policy (or global defaults from `~/.hex/inference.toml`).
3. **Filter registered providers** to those meeting `quantization_level >= minimum`. Among candidates, prefer highest quality_score, then lowest quantization (cheapest that passes the floor).
4. **Escalate on failure**: if the selected provider returns an error or the response fails the downstream gate, retry with next higher quantization tier.

This is transparent to callers — same `/api/inference/complete` endpoint, routing is internal.

### 5. Neural Lab integration: automatic quality score calibration

Add a `hex neural-lab experiment --type quant-calibration` experiment type that:
1. Generates a benchmark suite from the project's existing test cases and ADR descriptions
2. Runs the same prompts through Q2 → Cloud in sequence
3. Scores outputs against the behavioral specs (`docs/specs/`) using the `validation-judge` agent as oracle
4. Writes `quality_score` back to each provider record in SpacetimeDB
5. Stores calibration results in HexFlo memory under `neural-lab:quant-calibration:{provider_id}:{date}`

This closes the feedback loop: Neural Lab learns the actual quality surface for the installed models, and the router uses real scores — not hardcoded tier assumptions.

### 6. Registry-driven default selection

When the RL engine is unavailable (cold start, nexus not running), `ModelSelector::select_model`
now queries the live SpacetimeDB registry (`GET /api/inference/endpoints`) for the best
calibrated OpenRouter model instead of falling back to a hardcoded string.

Selection logic:
1. Filter to `provider == "openrouter"` and `quality_score >= 0.0` (calibrated via `hex inference test`)
2. Apply minimum capability floor (`is_adequate_for_task` — no `:free` models for code generation)
3. Rank by `quality_score` descending; break ties by `context_window` descending
4. If no calibrated models exist, fall through to hardcoded defaults (safe cold-start behaviour)

`SelectionSource::RegistryRanked` is emitted when this path fires — visible in pipeline logs.

**Activation**: registry selection only activates once at least one model has been calibrated
via `hex inference test <provider-id>`. Until then, hardcoded defaults remain in effect.

Models are registered with real `context_window` values via `hex inference discover --provider openrouter`
(previously this was hardcoded to 100_000 — fixed alongside this ADR).

### What changes where

| Component | Change |
|-----------|--------|
| `hex-core/src/` | Add `QuantizationLevel` enum with `Ord`, `Display`, `FromStr` |
| `hex-nexus/src/routes/inference.rs` | Add `QuantizationRouter`; extend provider record; complexity scoring |
| `hex-nexus/src/routes/secrets.rs` | **DONE** — `context_window` persisted from discover; serde alias added |
| `hex-nexus/src/orchestration/` | Pass `min_quantization_level` from agent YAML through workplan executor |
| `hex-cli/src/commands/inference.rs` | **DONE** — `--quantization` flag; `context_window` in registration body |
| `hex-cli/src/pipeline/model_selection.rs` | **DONE** — `query_registry_best` + `SelectionSource::RegistryRanked` |
| `hex-cli/assets/agents/hex/hex/*.yml` | Extend `inference.quantization` block in coder/planner/reviewer YAMLs |
| `hex-cli/assets/schemas/` | Update agent YAML JSON schema for `inference.quantization` |
| `hex-nexus/src/` | Add `neural_lab_quant.rs`: quant-calibration experiment runner |

## Consequences

**Positive:**
- Local Q2/Q4 models handle routine tasks at zero token cost; cloud reserved for genuinely hard tasks
- Agent YAMLs express intent declaratively — no hardcoded routing logic per agent
- Neural Lab calibration makes quality thresholds data-driven, not guesswork
- Routing is fully transparent; `hex inference list` shows quantization level per provider
- Offline/air-gapped workflows remain first-class

**Negative:**
- Complexity scoring heuristic is approximate — may over- or under-escalate until Neural Lab calibrates
- Auto-detection of quantization from Ollama model tags requires tag naming conventions (`:q2_k`, `:q4_k_m`) — non-standard names fall back to `Q4` default
- Adds Neural Lab calibration step to onboarding for users who want optimal routing

**Mitigations:**
- Ship conservative defaults (complexity threshold biased toward escalation) — better to use a stronger model than produce wrong output
- `hex inference add` warns when quantization cannot be auto-detected and suggests explicit `--quantization`
- Calibration is optional; uncalibrated providers use tier-based defaults (Q2=0.6, Q4=0.8, FP16=0.95, Cloud=1.0)

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P0 | Add `QuantizationLevel` to hex-core; extend provider record schema | **Complete** |
| P0.1 | Persist `context_window` from `hex inference discover`; fix serde alias | **Complete** |
| P0.2 | `SelectionSource::RegistryRanked` + `query_registry_best` in model_selection.rs | **Complete** |
| P0.3 | `hex inference test` runs real inference, computes quality_score, PATCHes nexus via `PATCH /api/inference/endpoints/:id` | **Complete** |
| P0.4 | Production calibration: 89/91 OpenRouter endpoints calibrated with real quality scores (2 `:free` blocked by upstream rate limit) | **Complete** |
| P1 | Routing logic in hex-nexus: complexity scoring + tier-filtered provider selection | **Complete** |
| P2 | Agent YAML schema extension + supervisor reads quantization policy | **Complete** |
| P3 | CLI: `--quantization` flag + Ollama auto-detect | **Complete** |
| P4 | Neural Lab quant-calibration experiment type (`POST /api/neural-lab/experiments/quant-calibration`) | **Complete** |
| P5 | Integration tests + calibration smoke test (19/19 passing) | **Complete** |

## References

- TurboQuant: Google Research, 2026 — extreme compression with non-uniform quantization grids
- ADR-2603261000: Secure Inference Provider Registry — establishes `key_ref`, fallback chains, model aliases (this ADR builds on that provider schema)
- ADR-027: HexFlo native coordination
- ADR-044: Config sync repo → SpacetimeDB
- `hex-cli/assets/agents/hex/hex/hex-coder.yml` — current agent YAML inference block
