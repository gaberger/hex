# Inference

> Back to [README](../README.md) | See also: [Architecture](ARCHITECTURE.md) | [Getting Started](GETTING-STARTED.md) | [Developer Experience](DEVELOPER-EXPERIENCE.md) | [Comparison](COMPARISON.md)

---

## Tiered Inference Routing with RL Self-Improvement

hex classifies every task by complexity and routes it to the right model — local 4B for typo fixes, local 32B for code generation, frontier for multi-file features. The tier->model mapping starts static and **self-optimizes via reinforcement learning** as the system accumulates dispatch outcomes ([ADR-2604120202](adrs/ADR-2604120202-tiered-inference-routing.md)).

| Tier | Model | Task Type | tok/s | Pass Rate | Best-of-N |
|:-----|:------|:----------|------:|----------:|----------:|
| **T1** | qwen3:4b (Q4) | Trivial edits, renames, typo fixes | ~68 | 100% | 1 |
| **T2** | qwen2.5-coder:32b (Q4) | Single function + tests | ~11 | 100% | 3 |
| **T2.5** | qwen3.5:27b (Q4) | Multi-function, agentic | ~11 | 100% | 5 |
| **T3** | Frontier (Claude) | Multi-file features | -- | -- | 1 |

*Benchmarked on one reference machine (AMD Ryzen AI Max+ 395 / Strix Halo, 64 GB, Vulkan-accelerated Ollama, Q4 quantized) over 4 pipeline runs of a 9-task corpus (3 Rust + 3 TypeScript + 3 Go). Numbers will differ on other hardware — CPU-only Ollama is ~5–10× slower. Reproduce your own via `examples/standalone-pipeline-test/run.sh --verbose`; see [EVIDENCE.md](EVIDENCE.md) for details.*

**Best-of-N + Compile Gate**: For T2/T2.5 tasks, hex generates N completions and returns the first that passes `rustc`/`cargo check`. In the 9-task reference corpus every task compiled on the first attempt — the retry scaffolding wasn't exercised. The ADR predicted ~85% one-shot; observed on this corpus was 100%. Generalizing the 100% figure to arbitrary codebases is **not claimed** — larger or more novel tasks will fail candidates and exercise the retry loop.

**RL Q-Learning closes the loop.** The SpacetimeDB `rl-engine` module records rewards after every dispatch and updates Q-values via the Bellman equation. After 3 pipeline runs, the learned Q-table:

```
tier:T1|rename_variable    model:qwen3:4b            Q=+1.308  visits=3
tier:T1|fix_typo           model:qwen3:4b            Q=+1.308  visits=3
tier:T2|single_function    model:qwen2.5-coder:32b   Q=+0.110  visits=2
tier:T2|function_w_tests   model:qwen2.5-coder:32b   Q=+0.110  visits=2
tier:T2.5|multi_fn_cli     model:qwen3.5:27b         Q=+0.110  visits=2
```

Local models get a `LOCAL_SUCCESS_BONUS` (+0.1) per successful dispatch, and Q-values compound with each run. The `select_action` reducer uses epsilon-greedy (90% exploit, 10% explore) to occasionally try alternative models — discovering better pairings automatically. When a model fails, its Q-value drops and the router shifts traffic to alternatives.

| Scenario | Frontier-Only | With Tiered Routing | Savings |
|:---------|:-------------|:-------------------|:--------|
| 10-agent swarm (code + analysis) | $22.50 | $2.10 | **91%** |
| Bulk summarization (50 files) | $15.00 | $1.50 | **90%** |
| Mixed interactive + analysis | $8.00 | $3.00 | **63%** |

```bash
hex inference list                              # Available providers + tiers
hex inference discover                          # Scan for local/remote models
hex inference add ollama http://host:11434 --model qwen2.5-coder:32b
hex inference bench bazzite-ollama --model qwen3.5:27b  # Benchmark quality + speed -> tier
hex inference bench bazzite-ollama --compare bazzite-m27 # Side-by-side comparison
```

---

## Model Benchmarking

([ADR-2604131238](adrs/ADR-2604131238-inference-bench-command.md))

`hex inference bench` evaluates any model against hex-specific prompts — Rust code generation (async adapters with thiserror/reqwest/tests), architectural reasoning (cross-adapter violation detection), and identity probes. Each prompt produces a quality checklist score, and the combined result maps to a hex agent tier recommendation.

```
-- hex inference bench: minimax-m2.7:cloud via bazzite-ollama --

  OK  Identity       3.5s  (1/1 quality, 60 tok/s)
  OK  Code-gen     147s    (10/10 quality, 43 tok/s)
  OK  Reasoning    132s    (5/5 quality, 34 tok/s)

  Overall score:    0.92
  Recommended:      Tier 3 (Opus-equivalent)
```

| Flag | Effect |
|:-----|:-------|
| `--quick` | Skip code-gen prompt (fast triage) |
| `--compare <id>` | Run same suite against a baseline model |
| `--save` | Persist score + tier to nexus for the model router |
| `--model <name>` | Override the provider's registered model |

---

## Code-First Execution

hex is **code-first**: inference is an accelerator, not a gate ([ADR-2604131630](adrs/ADR-2604131630-code-first-execution.md)). Before calling any model, the executor checks whether the task can be completed through deterministic means — template codegen, AST transforms, or script execution. Workplan tasks carry a `strategy_hint` field that guides the executor:

| Strategy Hint | What Happens | Example |
|:--------------|:-------------|:--------|
| `template` | Mustache/Handlebars template expansion | Scaffold a new adapter |
| `ast_transform` | Tree-sitter parse -> transform -> emit | Rename symbol across files |
| `script` | Run a shell script | `cargo fmt`, `rustfmt`, linting |
| `inference` | LLM call (tiered routing) | Generate new function body |
| *(none)* | Auto-detect: try deterministic paths first, fall back to inference | Default behavior |

This means a workplan with 20 tasks might only need inference for 6 of them — the rest are template expansions, renames, and formatting passes that complete in milliseconds with zero token cost.

---

## Phase 2: Scaffolding Layer

Phase 1 (tier routing + RL) is live. Phase 2 adds three techniques that close the remaining quality and latency gaps:

### GBNF Grammar Constraints (live)

Local models generate verbose output — a 4B model produces ~5000 tokens of chain-of-thought reasoning for a one-line typo fix (89 seconds). GBNF (GGML BNF) grammars apply a **hard mask on token logits** at decode time, constraining output to only grammar-valid tokens. This isn't a prompt instruction the model can ignore — it's a physical constraint on the decoder.

A/B test results on T1 typo fix (qwen3:4b):

| Metric | Without Grammar | With Grammar | Improvement |
|:-------|:---------------|:-------------|:------------|
| Tokens | 5,096 | 1,968 | **2.6x reduction** |
| Time | 88.6s | 31.2s | **2.8x faster** |
| tok/s | 58.2 | 63.8 | 10% throughput gain |
| Correct | YES | YES | Same quality |

Four built-in grammars ship in `hex-nexus/src/orchestration/grammars.rs`:

| Agent Role | Grammar | Effect |
|:-----------|:--------|:-------|
| `hex-coder` | `CODE_ONLY_RUST` | Pure Rust code block, no prose |
| `planner` | `ANALYSIS` | Structured markdown with required section headings |
| General | `CODE_AND_COMMIT` | JSON: `{"code": "...", "commit_msg": "..."}` |

The grammar field flows through `InferenceRequest.grammar` -> `OllamaInferenceAdapter` -> Ollama's `/api/generate` `grammar` parameter -> llama.cpp GBNF decoder. Other backends ignore the field gracefully.

### Error-Feedback Retry Loop

When all N compilation attempts fail, the best compiler error is fed back to the model for up to 2 retries. Demonstrated in practice: the weather-cli example's mock provider had a mismatched brace — the `rustc` gate caught it, the error was fed back, and the model fixed it on the next pass. Implemented in `ScaffoldedDispatch::dispatch()` (`hex-nexus/src/orchestration/scaffolding.rs`).

### Cascading Escalation

When a T2 task exhausts all attempts + retries, the scaffolding layer automatically escalates to frontier via `ScaffoldedDispatch::with_frontier()`. Escalation rates are tracked per task-type in the RL engine — if a task-type escalates >50% of the time, the tier classifier reclassifies it as T3.
