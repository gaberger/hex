# ADR-2606071500: ReAct tool-use loop as the default single-agent execution model (with safeguards + context compression)

**Status:** Completed
**Date:** 2026-06-07
**Drivers:** The single-agent executor (`hex do` / `direct_exec.rs`) was single-shot per attempt — it could not explore before editing (grep callers, read neighbours, `cargo check` mid-reasoning), so it failed on any task needing more than a localized edit. The OpenClaw/Hermes direction (ADR-2606061359) is explicitly "tools + context + memory feeding one strong loop"; this ADR builds that loop.
**Relates-To:** ADR-2606061359 (collapse org-sim to a single gateway-mediated agent loop — Implementation step 2), ADR-2026-06-04-1740 (the direct executor / evidence gate).

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

`direct_exec.rs` ships the irreducible loop — task → read file → ONE inference call for an
edit → apply → evidence (exit 0) → commit / retry. It is fast, cheap, and reproducible for
localized edits, but the agent gets all its context up front and cannot *act* to gather more.
Tasks that require tracing consumers, reading a sibling module, or iterating against a compiler
error before editing are out of reach.

The proven pattern for this is **ReAct** (reason → act → observe → loop): the model interleaves
reasoning with tool calls, accumulating observations until it can act decisively. hex already
has every piece needed:

- `hex-nexus/src/orchestration/simple_agent.rs` — a working flat tool-use loop with the exact
  protocol we want: **native function-calling with a text-mode JSON fallback** (`extract_tool_uses`
  tries Anthropic blocks → OpenAI `tool_calls` → the inference fast-path → fenced-JSON), tool
  dispatch via `ToolRegistry::execute`, duplicate-success detection, and iteration/token budgets.
- `hex-nexus/src/tools/` — 20 tools whose shell/FS access is **already guarded**: path-traversal
  rejection + repo-root canonicalize (`repo_read`, `code_patch`), `is_critical_path` block,
  subprocess timeouts (`repo_grep` 5s, `cargo_check`/`typescript_check` 60s), output size caps.
- `direct_exec.rs` — the evidence→commit guarantee (`apply_edit`, `run_evidence` under
  `set -o pipefail`, pathspec-scoped `commit`) and graph-context + ranked-lesson assembly.

**The one missing capability — and the real risk — is context bloat.** A multi-step loop
accumulates tool observations (grep hits, file reads, cargo errors) in the message history;
unbounded, they overflow the model's context window and inflate the cost of every subsequent
call. Naive truncation loses signal; naive accumulation is unaffordable.

**Research framing.** Recursive Language Models (Zhang, Kraska, Khattab, arXiv:2512.24601) show
that *recursive decomposition* of an over-long prompt — the model programmatically examining and
re-calling itself over snippets — handles inputs ~100× beyond the context window and beats
**compaction by 26%** and CodeAct by 130% on GPT-5. Compaction (cap + summarize) is the cheaper
baseline; recursive decomposition is the frontier. We adopt the affordable baseline now and note
the frontier as a future direction.

## Decision

**Make a ReAct tool-use loop the default execution path for the single agent, with curated
guarded tools, an evidence-gated terminal action, and bounded context via hybrid compression.**

1. **The loop** (`hex-nexus/src/direct_react.rs`). Reuse `simple_agent`'s parsing/dispatch.
   Seed = task + graph context + ranked lessons + the windowed current file. Each turn: compress
   the transcript, call inference with the curated `tools` schema, dispatch tool calls, append
   observations, repeat — until the terminal action or `max_steps`.
2. **Curated tool allowlist (safeguard).** The loop exposes ONLY read/verify tools —
   `repo_read`, `repo_grep`, `cargo_check`, `typescript_check`, `dep_audit`, `secret_scan` — plus
   the terminal `propose_edit`. No arbitrary shell; no persona/side-effecting tools
   (`adr_draft`, `workplan_emit`, `delegate`, `escalate_to_operator`, `web_search`).
3. **Evidence-gated terminal.** `propose_edit` applies the edit, runs the task's evidence command,
   and **commits only on exit 0** (rejecting vacuous passes); on failure the edit is *reverted*
   (each proposal is atomic) and the error is returned so the agent corrects. The evidence gate
   remains the sole authority on what commits — unchanged from the single-shot path.
4. **Hybrid context compression** (`hex-nexus/src/compress.rs`). Deterministic mechanical pass —
   per-observation head+tail cap + a rolling window that keeps the last K observation turns
   verbatim and collapses older ones to one-line gists — applied before every call. When the
   compressed transcript still exceeds a token budget, ONE cheap-model call summarizes the older
   region. (RLM-style recursive decomposition over large observations is the future enhancement.)
5. **Default, with an escape hatch.** ReAct is the default for `hex do`; `--fast` keeps the
   single-shot path for trivial edits. Conservative `max_steps` (12) and token budgets bound cost.

## Consequences

**Positive.**
- The agent can explore before editing — tracing consumers, reading neighbours, iterating against
  the compiler — so it handles tasks the single-shot path could not.
- Reuses proven, already-tested machinery (`simple_agent` parsers, guarded `ToolRegistry`,
  `direct_exec` evidence gate); little net-new surface beyond the compressor.
- Context stays bounded and predictable; cost does not grow with exploration depth.
- The evidence→commit guarantee is preserved exactly — exploration cannot weaken the gate.

**Negative / trade-offs.**
- More tokens/latency than single-shot for trivial edits — mitigated by `--fast` and conservative
  budgets.
- Multi-step loops have more failure modes (drift, churn) — mitigated by duplicate-call detection,
  a no-progress guard, the curated allowlist, and `max_steps`.
- Mechanical+LLM compaction is lossy vs. recursive decomposition (RLM); accepted as the
  affordable baseline.

## Implementation

1. `compress.rs` — `cap_str`, `estimate_tokens`, `compress_messages` (pure, unit-tested).
2. `direct_react.rs` — `react_execute`; reuse `simple_agent::{extract_tool_uses,
   normalize_tool_input, strip_metadata_fields, assistant_turn_content}` and `direct_exec::{apply_edit,
   run_evidence, commit, evidence_is_vacuous, gather_context, ground_window}`; curated schema +
   `propose_edit`; `summarize_overflow` for the LLM half.
3. `direct_exec::execute_direct` dispatches: default → `react_execute`, `--fast` → single-shot;
   `DirectTask` gains `fast`/`max_steps`; `DirectRun` gains `steps`.
4. CLI `hex do run --fast`/`--max-steps`; the run feed surfaces step count.

## References

- **Recursive Language Models** — A. L. Zhang, T. Kraska, O. Khattab. arXiv:2512.24601.
  https://arxiv.org/abs/2512.24601 — recursive prompt decomposition handles inputs ~100× beyond
  the context window and beats compaction by 26% / CodeAct by 130% on GPT-5; the frontier our
  mechanical+LLM compaction is the affordable baseline of, and the future direction for handling
  large tool observations.
- **ReAct: Synergizing Reasoning and Acting in Language Models** — Yao et al., 2022. The
  reason→act→observe paradigm this loop implements.
- **Building effective agents** — Anthropic. Prefer simple, composable, observable loops over
  heavyweight frameworks.
- Relates-To: ADR-2606061359 (single-agent loop / retire org-sim), ADR-2026-06-04-1740 (direct
  executor + evidence gate).
