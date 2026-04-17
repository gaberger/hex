# Evidence

> Back to [README](../README.md) | See also: [Inference](INFERENCE.md) | [Architecture](ARCHITECTURE.md)

Each claim in the README is backed by a test, fixture, or example you can run locally. This page maps claim → reproducer → expected output. No marketing — just commands and what they print.

## Prerequisites

```bash
cargo build --workspace                # builds hex-cli and hex-nexus (debug)
# optional, only for the Ollama-backed reproducers:
ollama serve &                         # or set OLLAMA_HOST=<remote>:11434
hex nexus start                        # starts SpacetimeDB + the daemon
```

Reproducers are split into two groups:

- **Hermetic** — run offline with `cargo test`. No LLM, no daemon, no network.
- **Live** — require a running nexus + Ollama (or Claude Code). Results depend on the model.

---

## 1. Tree-sitter hexagonal boundary analyzer

**Claim:** `hex analyze` parses source with tree-sitter, classifies each file by layer, and fails the exit code on cross-layer imports.

**Hermetic — 12 inline tests in the analyzer:**
```bash
cargo test -p hex-cli --lib commands::analyze
```
Reads from `hex-cli/src/commands/analyze.rs` (tests at the bottom of the file). Covers layer classification, import extraction, cycle detection, and score computation. Expected: `12 passed`.

**Live — analyze a real sample project:**
```bash
cd examples/hex-weather
hex analyze .
echo "exit=$?"
```
Expected: reports `0 boundary violations` across the domain / ports / adapters split. Exit code `0`.

**Live — verify a violation is caught:**
```bash
cd examples/hex-weather
# simulate an agent introducing a cross-adapter import
echo 'use crate::adapters::other_adapter::*;' >> src/adapters/mod.rs
hex analyze .
echo "exit=$?"
git checkout src/adapters/mod.rs
```
Expected: analyzer reports the cross-adapter import with file:line; exit code non-zero. The pre-commit hook invokes the same command, so the commit would be blocked.

---

## 2. Compile-gated best-of-N code generation

**Claim:** For T1/T2/T2.5 tasks the executor generates N completions and only accepts one that passes `cargo check` / `tsc --noEmit`. Compiler errors from failed candidates are fed back into the next attempt.

**Hermetic — tiered routing end-to-end test:**
```bash
cargo test -p hex-nexus --test tiered_routing_e2e
```
File: `hex-nexus/tests/tiered_routing_e2e.rs`. Uses a fake inference adapter to simulate multiple compile-fail candidates before a passing one; asserts the fail→retry→accept path and that escalation does not trigger when a lower tier succeeds.

**Live — full pipeline across T1/T2/T2.5:**
```bash
cd examples/standalone-pipeline-test
./run.sh --verbose                    # all tiers
./run.sh --tier T2 --verbose          # just the code-generation tier
```
Requires: `hex nexus start` + Ollama + `rustc`. The script dispatches real prompts per tier, runs the compile gate after each generation, and prints pass/fail plus generation time. Numbers are hardware-dependent — the script records what your box actually produces.

---

## 3. Task tier classifier (T1 / T2 / T3)

**Claim:** `classify_work_intent` routes every user prompt to T1 (Todo), T2 (mini-plan), or T3 (workplan) via regex + structural heuristics.

**Hermetic — inline tests in hook.rs:**
```bash
cargo test -p hex-cli --lib commands::hook::tests::
```
File: `hex-cli/src/commands/hook.rs`, tests at line 3069+ (functions prefixed `t1_*`, `t2_*`, `t3_*`, `cross_tier_*`, `test_rule_table_invariants`). Asserts concrete prompts → tier mappings, including:
- questions (`how does the planner work?`) → T1
- trivial edits (`fix typo in README`, `rename getCwd to …`) → T1
- empty / whitespace → T1
- feature prompts (`add password reset`, `implement authentication`) → T3

**Live — see classification on a real prompt:**
```bash
echo '{"prompt":"add SSO support"}' | hex hook user-prompt-submit
echo '{"prompt":"rename getCwd to getCurrentWorkingDirectory"}' | hex hook user-prompt-submit
```
Expected: the first prints a T3 draft notification and creates a file under `docs/workplans/drafts/`; the second prints nothing visible (T1 is silent).

---

## 4. Workplan reconciler (no self-reported completion)

**Claim:** A task is "done" only when the reconciler finds commits in the task's scope. Self-reported `status: done` without `evidence.commits[]` is rejected.

**Hermetic — evidence-required regression test:**
```bash
cargo test -p hex-cli --test reconcile_evidence
```
File: `hex-cli/tests/reconcile_evidence.rs`. Uses `hex-cli/tests/fixtures/reconcile/wp-partial.json`, which marks P1 done (with real files committed) and P2/P3 pending (target files absent). Asserts:
- P1 stays done (evidence present)
- P2/P3 are **not** auto-promoted despite other heuristics firing (no target files → no evidence)

**Live — reconcile against a real workplan:**
```bash
hex plan reconcile hex-cli/tests/fixtures/reconcile/wp-partial.json
```
Expected output mentions `P1` confirmed and explicitly lists P2/P3 as not advanced. Exit code `0` even though the workplan isn't fully done — reconciliation is a read, not a gate.

---

## 5. HexFlo swarm coordination

**Claim:** In-process Rust coordination (no subprocess IPC) with CAS task claims and heartbeat-based stale-agent reclamation.

**Hermetic — coordination + memory tests:**
```bash
cargo test -p hex-nexus --test hexflo_memory_e2e
cargo test -p hex-nexus --test agent_coordination
```
Files:
- `hex-nexus/tests/hexflo_memory_e2e.rs` — memory store / retrieve / search, scope enforcement
- `hex-nexus/tests/agent_coordination.rs` — agent registration, heartbeat tick, stale detection, task reclamation

**Live — observe a swarm running a workplan:**
```bash
hex plan execute examples/hex-weather/workplan.json
hex task list
```
The dashboard at `http://localhost:5555` subscribes to SpacetimeDB and shows task state transitions in real time.

---

## 6. Standalone mode (no Claude Code)

**Claim:** When `CLAUDE_SESSION_ID` is unset, hex composes with an Ollama adapter + HexFlo dispatch and runs the same workplans. Source: ADR-2604112000.

**Hermetic — composition tests:**
```bash
cargo test -p hex-nexus --test composition_standalone
cargo test -p hex-nexus --test standalone_dispatch_e2e
```
Files assert the `AgentManager` wiring swaps to `OllamaInferenceAdapter` when the session ID is absent, and that a workplan dispatches through the standalone path.

**Live — diagnose which composition is active:**
```bash
hex doctor composition
```
Prints the active variant (`Standalone` or `Claude-integrated`), the inference adapter in use, and which prerequisites are satisfied.

**Live — gate the standalone path in CI:**
```bash
hex ci --standalone-gate
```
Runs the P2/P3/P6 test suites with the standalone composition to prove the path still works without Claude Code.

---

## Hardware context for benchmark numbers

The token rates and timings in `docs/INFERENCE.md` (`~68 tok/s`, `~11 tok/s`, `88.6s → 31.2s` GBNF, `100% first-attempt compile`) were measured on one reference machine:

- AMD Ryzen AI Max+ 395 (Strix Halo), 64 GB unified memory
- Vulkan-accelerated Ollama (llama.cpp) with Q4 quantization
- 9-task pipeline corpus (3 Rust + 3 TypeScript + 3 Go)
- Models: `qwen3:4b`, `qwen2.5-coder:32b`, `qwen3.5:27b`

These numbers are reproducible on similar hardware but will not generalize:
- CPU-only Ollama is roughly 5–10× slower
- Larger / different corpora will not hit 100% first-attempt compile
- Compile-gate pass rates depend on the model's training coverage of the target language

Run the pipeline yourself to generate numbers for your own environment:

```bash
cd examples/standalone-pipeline-test
./run.sh --verbose 2>&1 | tee my-benchmark.log
```

The script prints model, tier, generation time, compile result, and total elapsed. Share `my-benchmark.log` if your numbers differ materially — we're interested.

---

## What isn't proven yet

Honest inventory of gaps between the docs and the reproducers:

- **`70% of tasks run free`** — this is a claim about the aggregate tier distribution on *real workplans*. We can measure it per-workplan (`hex plan analyze <wp.json>` groups steps by tier) but there is no corpus-wide audit yet. Treat the 70% figure as "observed on the maintainers' own workplans", not an industry statistic.
- **`RL Q-learning self-improves routing`** — the Q-table is persisted in SpacetimeDB (`rl-engine` module) and updated per dispatch outcome. Convergence on real workloads has not been published. Run `hex inference escalation-report` to inspect your own tier escalation rates.
- **TLA+ models in `docs/algebra/`** — model-checked for safety and liveness (see [FORMAL-VERIFICATION.md](FORMAL-VERIFICATION.md)), but the CI pipeline does not run TLC on every PR yet.

If a claim elsewhere in the docs lacks a reproducer on this page, it's not yet validated. File an issue and we'll either add a reproducer or remove the claim.
