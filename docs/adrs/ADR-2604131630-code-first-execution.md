# ADR-2604131630: Code-First Execution — Inference as Accelerator, Not Gate

**Status:** Accepted
**Date:** 2026-04-13
**Drivers:** Worker poll loop phase failed silently (all 4 tasks, empty error strings) when inference provider was unreachable. hex must work reliably outside Claude Code environments. Inference dependency is currently a single point of failure for standalone workplan execution.
**Supersedes:** None (refines ADR-2604112000 Standalone Dispatch)

## Context

hex is an AIOS — it must run reliably in two environments:

1. **Inside Claude Code** — Agent tool dispatch is available, inference is optional
2. **Standalone** — no Claude Code, inference is the primary code generation path

Today, standalone workplan execution routes ALL code tasks through inference (Ollama/vLLM). When inference fails:

- Tasks fail with **empty error strings** — the executor swallows the real error
- There is **no fallback** — the entire phase dies
- There are **no code-first primitives** — hex has no execution strategy that doesn't require an LLM chat round-trip
- The developer gets zero diagnostic information about what went wrong

This violates hex's operating-system promise: an OS must degrade gracefully, not crash because one subsystem is temporarily unavailable.

### Alternatives Considered

1. **Make inference more reliable** — Doesn't solve the architectural problem. Inference will always be an external dependency that can fail.
2. **Require Claude Code** — Abandons the standalone path. Unacceptable per ADR-2604112000.
3. **Code-first with inference acceleration** — (Chosen) Build execution primitives that work without inference, use inference to enhance when available.

## Decision

### 1. Execution Strategy Hierarchy

hex SHALL execute workplan tasks using a **tiered strategy** that exhausts code-first approaches before reaching for inference:

| Priority | Strategy | Requires Inference | Examples |
|----------|----------|-------------------|----------|
| 1 | **Template codegen** | No | Scaffold ports, adapters, modules from schema/spec |
| 2 | **AST transform** | No | Rename, move, extract, inject (tree-sitter powered) |
| 3 | **Script execution** | No | Build gates, test suites, linting, formatting |
| 4 | **Inference-assisted codegen** | Yes | Novel logic, complex implementations |

The executor SHALL attempt strategies in priority order. A task that can be completed by template codegen MUST NOT be routed to inference.

### 2. Inference Failures Must Be Loud

When inference IS used and fails, the system SHALL:

- **Capture the full error** — HTTP status, response body, timeout details, provider name
- **Store the error in the task record** — never empty strings
- **Retry with backoff** — 3 attempts with exponential backoff (1s, 4s, 16s)
- **Surface diagnostics** — `hex doctor inference` must show the failure chain
- **Log to session events** — for post-mortem via `hex brief`

### 3. Task Classification for Strategy Selection

Each workplan task SHALL carry a `strategy_hint` field:

```json
{
  "id": "P1.1",
  "title": "Add /api/hexflo/tasks/poll endpoint",
  "strategy_hint": "codegen",
  "template": "axum-endpoint",
  "fallback": "inference"
}
```

Strategy hints:
- `scaffold` — use template codegen (port, adapter, module scaffolds)
- `transform` — use AST transform (renames, moves, extractions)
- `script` — run a command (test, build, lint, format)
- `codegen` — requires code generation (try template first, fall back to inference)
- `inference` — explicitly requires LLM reasoning (design decisions, complex logic)

When no hint is provided, the executor SHALL classify based on task title heuristics (same approach as T1/T2/T3 tier routing in ADR-2604131500).

### 4. Standalone Composition Path Update

The standalone composition (`AgentManager` + `OllamaInferenceAdapter`) SHALL be updated:

- `AgentManager::execute_task()` checks strategy_hint before routing to inference
- Template registry (`hex-cli/assets/templates/`) provides codegen templates
- AST transform registry reuses existing tree-sitter infrastructure in `hex-nexus/src/analysis/`
- Inference adapter wraps errors with full context before returning

### 5. Claude Code Path — Inference as Optimizer

When `CLAUDE_SESSION_ID` is set (Claude Code available):

- Strategy 1-3 tasks still use code-first primitives (faster, no token cost)
- Strategy 4 tasks route to Agent tool dispatch (not inference)
- Inference is available as an explicit opt-in for batch/background work via `hex plan execute --prefer-inference`

## Consequences

**Positive:**
- hex works without any inference provider for scaffold/transform/script tasks
- Inference failures degrade to slower execution, not total failure
- Error messages always contain actionable diagnostic information
- Standalone path becomes genuinely reliable for real workloads
- Reduced token cost — template codegen is free

**Negative:**
- Template registry requires upfront investment (templates for common patterns)
- Strategy classification adds complexity to the executor
- Two code paths (template vs inference) need testing

**Mitigations:**
- Start with 5-10 high-value templates (port, adapter, endpoint, test, module) — cover 80% of scaffold tasks
- Strategy classifier reuses existing T1/T2/T3 heuristics
- Integration tests run both paths for every template-eligible task

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Fix empty error strings in inference adapter — capture full error context | Pending |
| P2 | Add retry with backoff to `OllamaInferenceAdapter` | Pending |
| P3 | Add `strategy_hint` field to workplan task schema | Pending |
| P4 | Template registry — 5 core templates (port, adapter, endpoint, test, module) | Pending |
| P5 | Executor strategy router — try code-first before inference | Pending |
| P6 | `hex doctor inference` failure chain diagnostics | Pending |
| P7 | Integration tests — both paths for template-eligible tasks | Pending |

## References

- ADR-2604112000: Standalone Dispatch (establishes standalone as first-class)
- ADR-2604131500: AIOS Developer Experience (T1/T2/T3 tier routing)
- ADR-2604130010: Worker Local Inference Discovery
- ADR-2604131238: Inference Bench Command
- Worker poll loop failure: all 4 tasks failed with empty inference errors (2026-04-13)
