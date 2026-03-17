# ADR-005: Compile-Lint-Test Feedback Loop with Quality Gates

## Status: Accepted
## Date: 2026-03-15

## Context

LLM-generated code frequently contains type errors, lint violations, and logical bugs. Without structured feedback, agents waste tokens on trial-and-error. We need a deterministic pipeline that gives agents structured, machine-readable errors after each generation attempt, with a convergence guarantee.

## Decision

Every code generation cycle passes through a **6-gate pipeline** with a maximum of **5 iterations** before escalation.

### The 6-Gate Pipeline

| Gate | Port Method | Timeout | Output on Failure |
|------|------------|---------|-------------------|
| 1. **Compile** | `IBuildPort.compile()` | 5s | `BuildResult.errors[]` — file, line, message |
| 2. **Lint** | `IBuildPort.lint()` | 2s | `LintResult.errors[]` — file, line, rule, severity |
| 3. **Unit Test** | `IBuildPort.test(unit)` | 10s | `TestFailure[]` — test name, expected vs actual |
| 4. **Integration Test** | `IBuildPort.test(integration)` | 30s | `TestFailure[]` — cross-adapter failures |
| 5. **AST Diff** | `IASTPort.diffStructural()` | 1s | `StructuralDiff` — unexpected export changes |
| 6. **Token Budget** | Budget check | 1s | Over-budget warning with current vs max tokens |

Gates execute sequentially. Failure at any gate short-circuits: the agent receives structured errors and re-enters at `ICodeGenerationPort.refineFromFeedback()`.

### Iteration Limits and Escalation

- **Max 5 iterations** per code unit before escalation
- Each iteration must produce a **quality score** (weighted sum: compile=40, lint=20, unit=25, integration=10, ast=3, budget=2)
- **Convergence rule**: Quality score must strictly improve on at least one gate per iteration. If score stagnates or regresses for 2 consecutive iterations, escalate immediately.
- **Escalation path**: Hand off to a higher-tier model (Sonnet to Opus) or flag for human review with the full error history attached.

### Structured Error Format for LLM Consumption

All errors are normalized to a common JSON format:

```json
{
  "gate": "lint",
  "iteration": 3,
  "qualityScore": 72,
  "errors": [
    {
      "file": "src/adapters/secondary/git-adapter.ts",
      "line": 45,
      "column": 12,
      "severity": "error",
      "message": "Property 'commit' is missing in type 'GitAdapter'",
      "rule": "typescript/interface-impl",
      "suggestion": "Add method: async commit(msg: string): Promise<string>"
    }
  ]
}
```

The `suggestion` field is populated by lint rules where available, giving the LLM a direct fix hint.

## Consequences

### Positive

- Deterministic feedback loop prevents infinite retry spirals
- Structured errors compress well and are directly actionable by LLMs
- Quality score convergence rule catches agents that are not making progress
- Sequential gate execution minimizes wasted compute (fail fast at compile)

### Negative

- 5-iteration cap may be insufficient for complex refactors; requires escalation handling
- Integration tests at gate 4 are slow (30s); most iterations will short-circuit earlier
- Quality score weights are heuristic and may need tuning per language
