# ADR-2603232340: Validate Loop — Test, Analyze, Refactor Until Grade A

**Status:** Proposed
**Date:** 2026-03-23
**Drivers:** `hex dev` generates code but doesn't verify it works. The validate phase currently just checks `hex analyze` (which often isn't available). A real pipeline should compile, test, analyze, and refactor in a loop until the code passes all quality gates.
**Supersedes:** None (extends ADR-2603232005)

## Context

Today's `hex dev` pipeline:
```
ADR → Workplan → Swarm → Code → Validate(stub) → Done
```

The validate phase is a single pass that calls `GET /api/analyze` and moves on. It doesn't:
1. **Compile check** — does the generated code even parse?
2. **Run tests** — do unit tests pass?
3. **Grade the architecture** — what's the hex analyze score?
4. **Iterate** — if something fails, fix it and try again

### What "Grade A" means

`hex analyze` produces a health score (0-100) with specific metrics:
- Boundary compliance (domain imports only domain, adapters don't cross)
- No circular dependencies
- No dead exports
- All files parseable by tree-sitter

**Grade A = score >= 90 with zero boundary violations.**

## Decision

### 1. Replace Single-Pass Validate with Quality Loop

```
Code phase complete
  │
  ├── 1. Compile check (tsc --noEmit or cargo check)
  │     └── Fail? → call inference to fix compile errors → retry
  │
  ├── 2. Run tests (bun test or cargo test)
  │     └── Fail? → call inference to fix test failures → retry
  │
  ├── 3. hex analyze (architecture health)
  │     └── Score < 90 or violations? → call inference to fix → retry
  │
  └── All pass? → Grade A → advance to Commit
```

### 2. Maximum Iterations

Each quality gate retries up to **3 times**. If still failing after 3 attempts:
- In `--auto` mode: log the failures, advance anyway with a warning
- In interactive mode: show gate dialog with failures, user decides

### 3. Language-Aware Compile Check

| Language | Compile Command | Run In |
|----------|----------------|--------|
| TypeScript | `npx tsc --noEmit --strict` | output_dir |
| Rust | `cargo check` | output_dir |
| JavaScript | `node --check` per file | output_dir |

If no `tsconfig.json` or `Cargo.toml` exists in the output dir, the compile check is skipped (code-only generation without a project scaffold).

### 4. Test Runner

| Language | Test Command | Run In |
|----------|-------------|--------|
| TypeScript | `bun test` or `npx vitest run` | output_dir |
| Rust | `cargo test` | output_dir |

If no test runner is configured, skip. The pipeline should generate a minimal `package.json` or `Cargo.toml` if tests are present.

### 5. hex analyze Integration

Run `hex analyze <output_dir>` and parse the JSON output:

```rust
struct AnalyzeResult {
    score: u32,           // 0-100
    grade: char,          // A, B, C, D, F
    violations: Vec<Violation>,
    dead_exports: Vec<String>,
    circular_deps: Vec<String>,
    files_analyzed: usize,
}
```

Grade mapping:
| Score | Grade |
|-------|-------|
| 90-100 | A |
| 80-89 | B |
| 70-79 | C |
| 60-69 | D |
| 0-59 | F |

### 6. Fix Prompts

Each failure type gets a specialized fix prompt:

| Failure | Prompt Template | Model |
|---------|----------------|-------|
| Compile error | `fix-compile.md` — includes error output + source file | DeepSeek R1 (reasoning) |
| Test failure | `fix-tests.md` — includes test output + source + test file | DeepSeek R1 (reasoning) |
| Boundary violation | `fix-violations.md` (existing) | DeepSeek R1 (reasoning) |
| Dead exports | `fix-dead-code.md` — remove unused exports | Llama 4 (simple edit) |

### 7. Pipeline Phase Update

The validate phase becomes a multi-step quality loop:

```
── Phase 5: Quality Gate ─────────────────────────────────────
  Iteration 1:
    Compile:  PASS (tsc --noEmit)
    Tests:    FAIL (2/5 passing)
      → Fixing test failures... (deepseek-r1, $0.002)
  Iteration 2:
    Compile:  PASS
    Tests:    PASS (5/5)
    Analyze:  Score 78/100 (Grade C)
      → 2 boundary violations
      → Fixing violations... (deepseek-r1, $0.001)
  Iteration 3:
    Compile:  PASS
    Tests:    PASS (5/5)
    Analyze:  Score 94/100 (Grade A)
      → 0 violations

  Result: GRADE A (3 iterations, $0.003 fix cost)
```

### 8. TUI Display

The quality loop shows in the TUI task list:

```
  ▶ Quality Gate
    ✓ Compile check           PASS
    ▶ Tests (5/5)             PASS
    ○ Architecture analysis   Pending
    ○ Grade                   Pending
```

### 9. Report Integration

`hex report` shows the quality loop iterations:

```
── Phase 5: Quality Gate ─────────────────────────────────────
  Iterations:  3
  Final Grade: A (94/100)
  Compile:     PASS (TypeScript strict mode)
  Tests:       5/5 passing
  Violations:  0 (2 fixed automatically)
  Dead Code:   0
  Fix Cost:    $0.003 (2 inference calls)
```

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add compile check to validate phase (tsc/cargo check in output_dir) | Pending |
| P2 | Add test runner to validate phase (bun test/cargo test in output_dir) | Pending |
| P3 | Wire hex analyze JSON output into validate phase | Pending |
| P4 | Add fix-compile.md and fix-tests.md prompt templates | Pending |
| P5 | Implement retry loop (max 3 iterations per gate) | Pending |
| P6 | Add grade calculation and Grade A gate | Pending |
| P7 | Update TUI to show quality loop progress | Pending |
| P8 | Update hex report with iteration details and final grade | Pending |
| P9 | Generate minimal package.json/Cargo.toml in output_dir if tests present | Pending |

## Future: Swarm-Controlled Quality Orchestration

The current implementation embeds the quality loop inside `validate_phase.rs` as a pragmatic first pass. **The correct long-term architecture is swarm-controlled orchestration:**

```
Current (v1 — embedded loop):
  validate_phase.rs: compile → test → analyze → fix → retry (hardcoded)

Future (v2 — swarm-controlled):
  Swarm coordinator creates quality-gate tasks
  → quality-gate agent runs compile + test + analyze
  → reports result to swarm
  → swarm creates fix tasks (one per violation/error)
  → fix agents execute (tracked, costed, auditable)
  → swarm re-runs quality gate
  → loop until Grade A or max iterations
```

### Why swarm-controlled is better

1. **Each fix attempt is a tracked HexFlo task** — agent assignment, cost, result all recorded
2. **Fix agents can run in parallel** — multiple violations fixed concurrently
3. **Swarm topology controls retry strategy** — pipeline for sequential, mesh for parallel fixes
4. **Quality gate is reusable** — any swarm can include it, not just `hex dev`
5. **Dashboard visibility** — each fix attempt appears in the swarm task list in real-time

### Migration path

1. v1 (this ADR): Quality loop inside validate_phase.rs — working, tested, ships now
2. v2: Extract quality gates into standalone swarm tasks
3. v3: Swarm coordinator owns the retry topology, spawns fix agents via HexFlo

## Consequences

### Positive
- **Code actually works** — not just generated, but compiled, tested, and architecture-validated
- **Grade A guarantee** — pipeline iterates until quality meets the bar (or reports why it can't)
- **Self-healing** — LLM fixes its own mistakes using error output as context
- **Audit trail** — every fix attempt logged in tool calls with cost
- **hex analyze is mandatory** — architecture enforcement is built into every pipeline run

### Negative
- **Higher cost** — fix iterations add $0.001-0.005 per attempt (still under $0.05 total)
- **Longer runtime** — 3 iterations adds ~30-60s per gate
- **May not converge** — some LLM-generated code can't be fixed in 3 attempts
- **v1 is not swarm-native** — quality loop is embedded, not yet orchestrated by HexFlo

### Mitigations
- 3-attempt cap prevents runaway costs
- Fix prompts include the actual error output (LLM can see what went wrong)
- Grade B (80+) is accepted in `--auto` mode with a warning; only interactive mode blocks on Grade A
- v2 migration planned to move quality loop into swarm coordination

## References

- ADR-2603232005: Self-Sufficient hex-agent with TUI
- ADR-2603232220: Developer Audit Report
- ADR-2603232216: Pipeline Validation Report
- ADR-027: HexFlo Native Coordination (swarm topology for v2)
