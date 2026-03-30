# ADR-2603281000: Context Pipeline Compression

**Status:** Accepted
**Date:** 2026-03-28
**Drivers:** hex agents have no mechanism to manage context window pressure during long multi-phase pipeline runs. The only existing mitigation is a hard file truncation by byte count (`token_budget.max`). There is no prompt compression, KV-cache quantization, or context pruning — agents simply run out of context window mid-task and degrade silently.

## Context

### The problem

hex's pipeline runs multi-phase agent tasks (specs → plan → code → validate → merge) where each phase accumulates context: prior phase outputs, file reads, tool call results, and error recovery loops. By the mid-to-late phases of a complex workplan, an agent may be operating at 70–90% of its model's context window with no awareness of that pressure.

Current state:
- **File truncation only**: `supervisor.rs:read_file_truncated()` caps individual files at `token_budget.max` bytes before they enter the prompt. This is a blunt instrument — it truncates uniformly regardless of which parts of a file are relevant.
- **No prompt compression**: The assembled prompt (system instructions + context files + prior tool outputs + conversation history) is sent as-is. No deduplication, summarization, or relevance filtering.
- **No KV-cache awareness**: When an agent makes repeated inference calls in a feedback loop (TDD red→green→refactor), it re-sends the same unchanged context each time with no caching benefit and growing output history.
- **No context pressure signal**: Agents do not know how full their context window is. The `status-monitor` YAML declares an 80% threshold alert but it is not wired to a real token count signal.
- **Silent degradation**: When context fills, models start hallucinating, forgetting earlier constraints, or producing truncated outputs. There is no detection or recovery path.

### What TurboQuant and related research unlocks

TurboQuant (Google Research, 2026) demonstrates that non-uniform quantization grids can compress model weights to 2-bit precision with near-FP16 accuracy. The same principle — aggressive lossy compression with quality-floor guarantees — applies to context management:

- **KV-cache quantization**: The attention key-value cache for long contexts can be quantized to 4-8 bits, reducing memory pressure and allowing more context to fit within the same window.
- **Prompt compression** (LLMLingua, selective context): Irrelevant tokens in the prompt can be pruned or summarized. A 10,000-token file with 3,000 relevant tokens can be compressed to ~3,500 tokens with minimal quality loss.
- **Tiered context loading**: Rather than loading all context upfront, load summaries (L1 AST) by default and promote to full source (L3) only for sections actively being edited — hex already has this L0–L3 model in agent YAMLs but does not enforce it dynamically.

### Alternatives considered

- **Increase `token_budget.max`**: Kicks the problem down the road. Does not help when context fills from conversation history and tool output accumulation, only from file size.
- **Split into more worktrees**: Already done (ADR-004). Worktrees isolate phases but context still fills within a single phase for large adapters.
- **Switch to models with larger context windows**: Mitigates but does not solve. A 200K context window fills too on sufficiently complex tasks, and larger windows incur higher inference cost.
- **Rely on model's built-in context management**: Models internally down-weight distant context but give no signal and make no guarantees. Not controllable.

## Decision

### 1. Context pressure tracking

Add a `ContextPressureTracker` in `hex-nexus/src/orchestration/` that maintains a running token estimate for each active agent session:

```rust
pub struct ContextPressureTracker {
    pub session_id: String,
    pub model_context_limit: u32,       // from provider record or model defaults
    pub estimated_used_tokens: u32,     // sum of input_tokens from session messages
    pub pressure_pct: f32,              // estimated_used_tokens / model_context_limit
}
```

The tracker updates on every inference call using `input_tokens` already returned by the inference route. At 70% pressure it emits a `ContextPressureWarning` to the hex inbox (ADR-060). At 90% it blocks the next inference call and triggers the compression pipeline.

### 2. Tiered context loader (enforce what agent YAMLs already declare)

The supervisor's context loading currently loads all declared context levels on agent start. Change it to enforce the `load: on_demand` flag already present in agent YAMLs:

- `load: always` → load at agent start (L1 AST summaries only)
- `load: on_demand` → load only when the agent explicitly requests the file via a `Read` tool call
- `load: active_edit` → load full source only for the file currently being edited

This alone reduces initial context load for hex-coder by ~60% (port interfaces + domain entities at L1 instead of L3).

### 3. Prompt compression for accumulated tool output

Add a `PromptCompressor` port in `hex-core/src/ports/` with a `compress_tool_output(output: &str, budget_tokens: u32) -> String` method. The secondary adapter implements selective context pruning:

1. If tool output is under `budget_tokens`, return as-is.
2. If over budget: extract code blocks and error messages verbatim; summarize prose sections to 1–3 sentences; deduplicate repeated file paths and stack traces.
3. The compression ratio target is 3:1 for prose-heavy outputs (test runner logs, git diffs with large unchanged sections).

This is applied to all tool outputs before they are appended to the conversation history in a feedback loop.

### 4. KV-cache hint headers for Anthropic API

When the inference provider is Anthropic (Cloud tier), add `cache_control: {"type": "ephemeral"}` markers on the system prompt and static context sections. This enables Anthropic's prompt caching, reducing re-sent tokens on repeated calls within the same feedback loop by up to 90% for the static prefix.

For non-Anthropic providers, this is a no-op (the field is stripped before sending).

### 5. Context pressure signal in agent YAML

Extend agent YAML `token_budget` with pressure thresholds:

```yaml
token_budget:
  max: 100000
  reserved_response: 20000
  pressure:
    warn_at_pct: 70       # emit hex inbox warning
    compress_at_pct: 80   # trigger prompt compression
    block_at_pct: 90      # pause and await context relief
    relief: summarize_history  # strategy: summarize_history | drop_oldest | escalate
```

The supervisor reads `pressure.relief` to decide recovery strategy when the block threshold is hit.

### What changes where

| Component | Change |
|-----------|--------|
| `hex-core/src/ports/` | Add `IContextCompressorPort` trait |
| `hex-core/src/` | Add `ContextPressure` value type |
| `hex-nexus/src/orchestration/` | Add `ContextPressureTracker`; integrate with workplan executor |
| `hex-nexus/src/adapters/` | Add `PromptCompressorAdapter` implementing `IContextCompressorPort` |
| `hex-nexus/src/routes/chat.rs` | Add Anthropic cache_control headers on static context sections |
| `hex-cli/src/pipeline/supervisor.rs` | Enforce `load: on_demand` for context files; pass pressure thresholds |
| `hex-cli/assets/agents/hex/hex/*.yml` | Add `token_budget.pressure` block to coder, planner, swarm-coordinator |
| `hex-cli/assets/schemas/` | Update agent YAML JSON schema for `token_budget.pressure` |

## Consequences

**Positive:**
- Agents get an actionable signal before context fills — no more silent mid-task degradation
- Tiered loading alone reduces initial prompt size by ~60% for hex-coder on typical adapters
- Prompt caching eliminates redundant token spend on repeated inference calls in TDD loops
- Compression pipeline extends effective context budget 2–3x on output-heavy phases (validate, integrate)

**Negative:**
- Compression is lossy — summarized tool output may omit details the agent needed
- Token estimation is approximate (byte-based heuristic until a real tokenizer is integrated)
- Anthropic cache_control adds API complexity; must be stripped for other providers

**Mitigations:**
- Compression preserves code blocks and error messages verbatim; only prose is summarized
- Conservative pressure thresholds (warn at 70%, block at 90%) leave headroom before degradation
- Provider abstraction means cache_control is handled in the Anthropic adapter only, not in the core path
- `relief: escalate` strategy available as fallback — pauses the agent and notifies the developer rather than risking silent corruption

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P0 | `IContextCompressorPort` in hex-core; `ContextPressure` value type | Done |
| P1 | `ContextPressureTracker` in hex-nexus orchestration; update inference route to feed token counts | Done |
| P2 | Enforce `load: on_demand` in supervisor; tiered context loading | Done |
| P3 | `PromptCompressorAdapter` — prose summarization + code block preservation | Done |
| P4 | Anthropic cache_control header injection in chat route | Done |
| P5 | Agent YAML `token_budget.pressure` schema + supervisor reads thresholds | Done |
| P6 | Integration tests: pressure tracking accuracy, compression quality floor | Done |

## References

- TurboQuant: Google Research, 2026 — non-uniform quantization grids, compression with quality-floor guarantees
- ADR-2603271000: Quantization-Aware Inference Routing — provider-level quantization (this ADR is orthogonal: context-level compression)
- ADR-060: Inbox notifications — used for context pressure warnings
- ADR-004: Git worktree isolation — phase isolation reduces but does not eliminate context accumulation
- ADR-027: HexFlo native coordination
- `hex-cli/assets/agents/hex/hex/hex-coder.yml` — `token_budget` and `context.levels` declarations
- `hex-cli/src/pipeline/supervisor.rs:317` — existing file truncation (`read_file_truncated`)
