# Pipeline Failure Root Cause Synthesis
_Generated 2026-03-25 by investigate-dev-pipeline-failures swarm_

## TL;DR

The `hex dev` pipeline has **4 independent bugs** that compound into total failure. Even if one were fixed, the others would still prevent success. All 4 must be addressed together.

---

## Bug 1 — `resolve_model_id` is a stub (CRITICAL)
**File**: `hex-cli/src/pipeline/agent_def.rs:166`

```rust
fn resolve_model_id(_name: &str) -> &str {
    "openai/gpt-4o-mini"  // ignores argument entirely
}
```

Every agent — regardless of YAML declaring `sonnet`, `haiku`, or `opus` — gets GPT-4o Mini. The free-tier fallback chain (`gpt-4o-mini → gemma-2-9b-it:free → qwen-2.5-7b:free`) cannot reliably generate hexagonal architecture TypeScript/Rust that passes `hex analyze`. RL Q-learning is also completely bypassed because the YAML model is passed as a hard `model_override`, suppressing the Q-table entirely.

**Fix**: Implement the `sonnet → claude-sonnet-4-6`, `haiku → claude-haiku-4-5`, `opus → claude-opus-4-6` mappings. Remove the stub.

---

## Bug 2 — Structural task wiring gap (CRITICAL)
**Files**: `hex-cli/src/tui/mod.rs:689`, `hex-cli/src/pipeline/supervisor.rs`

`SwarmPhase` creates P0.1–P4.1 HexFlo tasks (domain → ports → adapter → composition root → integration test) and returns `task_id_map` in `SwarmPhaseResult`. The TUI stores it in `self.task_id_map`. **But `task_id_map` is never passed to `Supervisor::with_tracking()`.**

When the Supervisor runs and finds no `src/` files, it calls `create_tracking_task()` which creates **new shadow tasks** named `hex-coder: CodeGenerated [iteration N]` — running monolithic code generation instead of the structured P* sequence.

`CodePhase::execute_all_tracked()` — the method that would execute P* tasks in order — was **orphaned** when the TUI was refactored to the objective-loop Supervisor.

**Fix**: Thread `task_id_map` from `SwarmPhaseResult` into `Supervisor::with_tracking()`. Use existing P* task IDs in `execute_agent_tracked()` instead of creating shadow tasks. Resurrect `execute_all_tracked()` as the execution path.

---

## Bug 3 — Fix agent is blind to source code (HIGH)
**File**: `hex-cli/src/pipeline/fix_agent.rs:134`

```rust
source_file: input.target_file,  // set to the TEST FILE path, not src/main.rs
```

When fixing compile errors caused by wrong stdout assertions in tests, the fixer receives the test file path as `source_file` — but cannot see `main.rs` to know what the code actually prints. Every fix attempt operates in the dark.

Additionally, `prior_results` only tracks a boolean (was this objective attempted before?) — **no error history is carried between iterations**. Each of the 15 fix attempts starts completely fresh.

**Fix**: Pass `src/main.rs` (or all relevant source files) as context to the fix agent. Carry the last N error messages forward in `prior_results`.

---

## Bug 4 — Generated tests are structurally broken (HIGH)
**Files**: `hex-cli/assets/prompts/agent-tester.md`, `hex-cli/assets/prompts/test-generate.md`

Two compounding issues:
1. **`CARGO_BIN_EXE_` name mismatch** — `test-generate.md` Rule 6 says *never* use `CARGO_BIN_EXE_`; `agent-tester.md` Rule 10 says to use it with the exact binary name. The model uses `agent-tester.md` but invents a short alias (`temp_converter`) instead of the full crate name. This is a **compile-time error** — `cargo test` never executes.
2. **Tests assert wrong stdout** — The test expects text that differs from what `main.rs` prints.
3. **Blocking stdin** — Tests call `.output()` on a binary that reads from `io::stdin().read_line()`, causing every test to hang.

**Fix**: Resolve the prompt contradiction (pick one rule). Inject the binary name explicitly into the tester prompt from Cargo.toml. Add a pre-test stdin mock requirement to the prompt.

---

## Secondary Issues

| Issue | Impact | File |
|-------|--------|------|
| Single-file fix targeting | Each iteration fixes only the first error's file; multi-file projects need N iterations minimum | `supervisor.rs` `infer_fix_target()` |
| No escalation at max_iterations | `run_tier()` returns `TierResult::MaxIterations` and logs "continuing" — no inbox notify, no model upgrade | `supervisor.rs` `run_tier()` |
| Three tiers on same broken output_dir | Tier 1+2 fail because Tier 0 never compiled, not because they have new problems | `supervisor.rs` |
| No cleanup of failed runs | Each failed run creates a new `examples/<uuid>/` directory | pipeline start |
| RL Q-table receives zero training data | `report_outcome()` is never called from `code_phase.rs` | `code_phase.rs` |

---

## Failure Chain (what actually happens today)

```
hex dev "build a temperature converter"
  ↓
SwarmPhase: creates P0.1-P4.1 tasks in HexFlo ← NEVER EXECUTED (Bug 2)
  ↓
Supervisor: creates shadow task "hex-coder: CodeGenerated"
  ↓
CodePhase: calls GPT-4o Mini (Bug 1) → generates monolithic Rust
  ↓
Generated tests use wrong binary name → compile error (Bug 4)
  ↓
Fix agent: receives test file path, not main.rs (Bug 3)
  ↓
5 iterations × 3 tiers = 15 blind fix attempts → still broken
  ↓
"hex-fixer: failed — fix agent failed" (likely inference quota exhausted)
  ↓
New example directory created, P0.1-P4.1 still pending, swarm still "active"
```

---

## Recommended Fix Order

1. **Fix `resolve_model_id` stub** — immediate unblocking of all inference quality issues. 10-line change.
2. **Thread `task_id_map` to Supervisor** — restores structured P* execution order. Architectural fix.
3. **Fix fix_agent source_file + add error carry-forward** — makes fix iterations cumulative.
4. **Resolve prompt contradiction in agent-tester.md/test-generate.md** — eliminates compile-time test failures.
5. **Add max_iterations escalation** — inbox notify + model upgrade at iteration 3.
6. **Add failed run cleanup** — remove stale example dirs on pipeline start.

Items 1+3+4 can be done in a single focused PR. Item 2 is the architectural change and warrants its own PR.
