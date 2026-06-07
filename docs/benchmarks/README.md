# hex agentic inference benchmark corpus

> Defined by [ADR-2606071734](../adrs/ADR-2606071734-agentic-inference-benchmark-suite.md).
> Run by `hex config inference bench --agentic` (runner is follow-up work).

These fixtures test whether a model can **drive hex's evidence-gated `hex do` loop** — not whether it
can write a function in isolation. A model that aces single-turn codegen can still wander the loop and
ship nothing (observed: `qwen2.5-coder:14b`, 18 steps, no edit). This corpus catches that.

## Fixture schema (`fixtures/*.json`)

```jsonc
{
  "id": "t2-humanize-duration",          // stable id
  "tier": "T2",                          // T1 | T2 | T2.5  — the dispatch tier this probes
  "axis_focus": ["evidence_gated_success", "react_convergence"],
  "instruction": "...",                  // the task, plain language (what the agent is told)
  "target_file": "hex-cli/src/fmt.rs",   // the ONLY file the agent may edit
  "oracle": {
    "kind": "cargo_test",                // cargo_test | cargo_check | tsc | grep
    "setup_files": {                     // materialized into the sandbox BEFORE the run;
      "hex-cli/tests/...": "<content>"   //   the agent never sees or can edit these
    },
    "command": "cargo test -p hex-cli --test fmt_humanize_duration"  // must exit 0 to pass
  },
  "graph_context_required": false,       // true ⇒ unsolvable without tracing the graph (T2.5 cases)
  "arms": ["react", "fast"],             // which loop arms to run
  "status": "verified",                  // verified (oracle RED→GREEN confirmed) | draft (unverified)
  "observed_baseline": { ... }           // real results we've seen, for regression
}
```

## Scoring — a vector, never one number

Per `(model × tier × arm × graph)`:

| Metric | Meaning |
|---|---|
| `protocol_ok%` | fraction of turns with well-formed tool calls |
| `edit_rate%` | fraction of cases that ever produced a `propose_edit` |
| `evidence_pass%` | fraction that passed the independent oracle |
| `mean_steps_to_edit` | convergence speed (∞ = never edited) |
| `p50_latency_per_step` | wall-time per loop step |
| `vram_fit` | fits fully on-GPU? (load-bearing for usability) |
| `Δ_graph` | `evidence_pass%(graph on) − (graph off)` — the thesis signal |

Every run is reported as **gap-to-frontier** (Claude via the ⑤ wire) — a local model is "T-N capable"
when its gap on T-N fixtures is ~0, not when its absolute score is high.

## Status

| Fixture | Tier | Probes | Status |
|---|---|---|---|
| `t2-humanize-duration` | T2 | convergence + evidence gate | **verified** (commit 49fc3b09) |
| `t1-add-derive` | T1 | mechanical transform | draft (oracle unverified) |
| `t25-trace-consumer` | T2.5 | graph-required; don't-break-the-consumer | draft (oracle unverified) |

`verified` = oracle confirmed RED before / GREEN after a known-good edit. `draft` = authored but not
yet run end-to-end. CI (per ADR) re-verifies `verified` oracles so the corpus can't silently rot.
