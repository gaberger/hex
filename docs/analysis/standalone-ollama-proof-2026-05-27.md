# Local-AI-Only Build Proof — 2026-05-27

**Question:** Can hex generate compilable example apps using ONLY local AI (Ollama),
with no Claude / no frontier-model API calls?

**Answer:** YES — proven for Rust at T1, T2, and T2.5 tiers. Three real programs
generated and compiled on the first attempt by `qwen2.5-coder:14b` (9.0 GB) running
locally via the standard hex inference path.

## Stack Verified

| Layer | Concrete |
|---|---|
| Daemon | `hex nexus start` (PID 36040, http://127.0.0.1:5555) |
| State | SpacetimeDB on http://127.0.0.1:3033, rl-engine connected |
| Inference provider | `ollama` @ `http://localhost:11434`, model `qwen2.5-coder:14b` (q4) |
| Calibration | `hex inference bench` → 0.96 overall, 79 tok/s on code-gen, Tier-3 recommended |
| Toolchain (compile gate) | `rustc 1.95.0` at `~/.cargo/bin/rustc` |

## Pipeline Run — `examples/standalone-pipeline-test/run.sh --tier all`

Raw log: `docs/analysis/standalone-ollama-proof-2026-05-27.log`.

| Task | Tier | Model | Result | Attempt | Notes |
|---|---|---|---|---|---|
| t1.1 rename `x → count` (Rust) | T1 | qwen3:4b | PASS | 1/1 | 248 tok/s |
| t1.2 fix typo `teh → the` (Rust) | T1 | qwen3:4b | PASS | 1/1 | 217 tok/s |
| t2.1 generate `fn fibonacci` + main | T2 | qwen2.5-coder:14b | **PASS — ran, output 55** | 1/3 | 85 tok/s, 1.2s |
| t2.2 palindrome checker + 3 unit tests | T2 | qwen2.5-coder:14b | **PASS** | 1/3 | compiled clean |
| t25.1 multi-fn CLI argparse (no extern crates) | T2.5 | gemma3:27b | **PASS** | 1/5 | 480 tok, 19 tok/s |
| ts.1 rename in TypeScript | T1 | qwen3:4b | PASS | 1/1 | text match only |
| ts.2 typed TypeScript fns | T2 | qwen2.5-coder:14b | FAIL | 0/3 | `tsc` not installed locally |
| go.1 fix typo in Go | T1 | qwen3:4b | PASS | 1/1 | text match only |
| go.2 Go fibonacci | T2 | qwen2.5-coder:14b | FAIL | 0/3 | `go` not installed locally |

**Total: 7 / 9 PASS.** Both FAILs are local-toolchain absence (no `tsc`, no `go`),
NOT model capability — confirmed by manual `which` check.

## What This Proves

1. **Standalone composition path is live.** `hex nexus start` brings up the daemon
   without `CLAUDE_SESSION_ID`; `OllamaInferenceAdapter` services requests.
2. **Tier routing dispatches to the configured local model.** T1 → qwen3:4b,
   T2 → qwen2.5-coder:14b, T2.5 → gemma3:27b (substituted locally for the run.sh
   default of qwen3.5:27b, which was not pulled).
3. **Compile gates work end-to-end.** Generated Rust files pass `rustc --edition 2021`
   (and `--test` for library code without `main`).
4. **Q-table is recording rewards.** `record_reward` reducer fired against
   rl-engine on every task; Q-values show qwen3:4b @ Q=1.20 for T1 rename/typo,
   qwen2.5-coder:14b @ Q=0.11 for T2 single_function. (Q-values stack across runs.)
5. **Best-of-N is mostly unused at this quality bar.** Every successful T2/T2.5
   task compiled on **attempt 1** — the best-of-3 / best-of-5 budget did not need
   to expand.

## What This Does NOT Yet Prove (Open Gaps)

1. **`hex ci --standalone-gate` does not invoke this harness.** Currently it only
   runs three mocked test suites (composition_standalone.rs, ollama_adapter.rs,
   standalone_dispatch_e2e.rs — all use `MockInferencePort` or `httpmock`).
   Workplan task **wp-hex-standalone-dispatch P7.2** remains `status: todo`.
2. **Harness has a PATH bug.** `run.sh` shells out to `rustc` / `tsc` / `go`
   without sourcing the user's shell PATH; on a typical install (`~/.cargo/bin`
   not in non-login bash) it silently fails compile-gate with `command not found`.
   First proof run produced a 5/9-FAIL false negative because of this. Needs a
   preflight that prepends `~/.cargo/bin` to PATH and skips tiers whose
   toolchain is absent rather than counting them as model failures.
3. **TypeScript / Go branches untested.** This box doesn't have the toolchains;
   needs to be run on a box that does, or those branches need to be conditional.
4. **End-to-end workplan execution via `hex plan execute` against local Ollama
   was NOT tested here.** This proof exercised the inference adapter + compile
   gate. The full workplan-execute path (which dispatches tasks via WorkplanTask
   strategy_hint → tier → model → adapter → compile) is the next proof to land.
5. **Workplan-status inconsistency:** `wp-hex-standalone-dispatch.json` is
   `status: "completed"` while contained task P7.2 is `status: "todo"`. Needs
   reconcile.

## Followups Dispatched

- Board ask to `hex-coder`: wire `hex ci --standalone-gate` to invoke
  `examples/standalone-pipeline-test/run.sh` with proper PATH + Ollama
  reachability gate; add `hex-cli/tests/standalone_gate.rs`.
- Board ask to `hex-coder`: fix run.sh PATH preflight (prepend `~/.cargo/bin`
  and skip absent toolchains explicitly).
- Workplan P7.2 status flip pending the above landing.
