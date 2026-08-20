# ADR-2606191907: A layered token-compression subsystem in the hook path — shrink context, not just route models

**Status:** Proposed
**Date:** 2026-06-19
**Epoch:** single-agent
**Drivers:** hex's cost lever is *model tiering* (T1–T3) — pick a cheaper model for a cheaper task. It has no productized lever for *context size*: every call pays for the full transcript it is handed. The ReAct loop (ADR-2606071500) flagged "context bloat … the real risk" and shipped a single cap+summarize compaction step as the affordable baseline. Meanwhile reasoning-model failures in our own bench (Nemotron, qwen3-coder) are *truncation* failures — `max_tokens` exhausted before the answer — i.e. context-budget failures, not model-quality failures. Comparative analysis of a5c-ai/babysitter surfaced a concrete, layered design: four compression engines bound to four distinct hook points, claiming 50–67% end-to-end context reduction. This ADR adopts that structure, hex-native.
**Relates-To:** ADR-2606071500 (ReAct loop — hybrid compression baseline this generalizes), ADR-2605301228 (ADR-governance decision-cards injected at GROUND — a Layer-3 consumer), ADR-2606071713 (code-graph as agent tool — a Layer-2 context source), ADR-2606080915 (resource governor — compression and admission control are complementary memory levers).

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

Two independent cost dials govern an inference call: **which model runs** (price per token) and
**how many tokens it sees** (tokens per call). hex has invested heavily in the first — tiered
routing, best-of-N with compile gates, the resource governor, escalation reports. It has invested
almost nothing in the second beyond two ad-hoc measures:

- **Output caps** in the tool layer (`hex-nexus/src/tools/` — `repo_grep`, `cargo_check` truncate
  oversized output). This is *clipping*, not compression: it drops the tail and keeps the head,
  losing signal with no semantic awareness.
- **A single compaction step** in `direct_react.rs` (ADR-2606071500): cap + summarize the
  transcript once per turn. Effective, but one engine at one point, tuned for one consumer.

The forces that make this worth formalizing now:

1. **Truncation is our dominant local-model failure mode.** Per the inference memory notes,
   reasoning models score 0 in our bench because the answer is pushed past `max_tokens` by an
   over-large prompt + long reasoning trace. Smaller input context directly buys answer headroom.
2. **The ReAct loop accumulates monotonically.** Every grep hit, file read, and cargo error stays
   in the message history. Cost per turn rises with turn count — the longest, hardest tasks are
   exactly the ones that get most expensive and most likely to overflow.
3. **The GROUND phase injects large, static artifacts.** ADR decision-cards (ADR-2605301228),
   specs, ranked lessons, and code-graph context (ADR-2606071713) are assembled into the seed
   prompt. These are highly compressible (mostly prose) and *re-injected* across calls — the
   highest-leverage compression target, and the one a naive cap handles worst.
4. **We already own every hook point.** `hex hook route` fires on `UserPromptSubmit`; the tool
   dispatch loop owns command output; `direct_exec`/`direct_react` own context assembly; the
   GROUND path owns artifact injection. The architecture for layered, point-specific compression
   already exists — what is missing is a *subsystem* that owns the policy instead of four
   uncoordinated clips.

**Alternatives considered.**

- *Do nothing / keep clipping.* Cheapest, but leaves the truncation failures and monotonic
  cost growth unaddressed; loses signal silently (a clipped grep tail reads as "no more matches").
- *Recursive Language Model decomposition* (arXiv:2512.24601, cited in ADR-2606071500) — the
  frontier; beats compaction by 26%. Higher complexity (the model re-calls itself over snippets);
  premature before we have a measured compression baseline to beat. Noted as the future direction.
- *Bigger context windows / frontier models only.* Abandons the local-inference economics that
  are hex's whole point; doesn't help T1/T2 at all.

## Decision

**Build a `hex-compress` subsystem: a registry of compression engines bound to named hook points,
each with a measured reduction target and a semantic (not positional) compaction strategy.
Compression is policy-driven, on by default, and instrumented — every reduction is logged with a
before/after token count so it is auditable, not magic.**

### Four layers, four hook points

| Layer | Hook point (hex surface) | Engine | Target | What it compresses |
|-------|--------------------------|--------|--------|--------------------|
| **1a** | `UserPromptSubmit` (`hex hook route`) | `density-filter` | ~25–30% | The user prompt — strip filler, dedupe, keep imperative content |
| **1b** | Tool-observation append (ReAct dispatch loop) | `command-compressor` | ~45% | `repo_grep` / `cargo_check` / `repo_read` output — collapse repeated frames, fold identical errors, keep diagnostic lines |
| **2** | Context assembly (`direct_exec` / `direct_react` seed) | `sentence-extractor` | ~85% | Graph context + ranked lessons + windowed file — extractive summary keyed to the task |
| **3** | GROUND artifact injection (decision-cards / specs / library) | `sentence-extractor` + cache | ~90% | ADR cards, specs, skill/process templates — extract + memoize per artifact hash |

Layers map 1:1 onto the babysitter four-layer model but route through hex-owned hooks and STDB,
not a flat `.a5c/` config.

### Boundaries and invariants

1. **Compression NEVER touches what commits.** It shapes *prompts*, never artifacts. The evidence
   gate (ADR-2606071500), `code_patch` output, and the journal/workplan state are off-limits. A
   compressed observation that misleads the agent is a recoverable error (next tool call corrects);
   a compressed diff would be corruption.
2. **Semantic over positional.** Engines must compact by meaning (dedupe frames, fold identical
   errors, extract task-relevant sentences), not by clipping the tail. Clipping remains only as the
   final hard-stop backstop when an engine cannot hit the window budget.
3. **Loss is logged, never silent.** Every engine emits `{hook, before_tokens, after_tokens,
   ratio}` to STDB. `hex inference compression-report` surfaces per-layer reduction, mirroring
   `hex inference escalation-report`. A compressor that drops signal must be visible to be tuned —
   silent truncation is the failure this ADR exists to kill.
4. **On by default, declaratively opt-out.** `.hex/project.json → inference.compression` with
   per-layer `enabled` + `target` overrides, and `HEX_COMPRESS=0` for a full bypass when debugging.
5. **Caching is content-addressed.** Layer 3 memoizes by artifact hash so a stable ADR card or
   skill template is compressed once and reused — the cheapest layer because the input rarely
   changes.

### Where it lives (hexagonal placement)

A new `ICompressionPort` (hex-core) with a `compress(hook, text, budget) -> Compressed` trait;
engine implementations as **secondary adapters** in hex-nexus (`adapters/secondary/compression/`);
invoked from the hook handler, the ReAct dispatch loop, and the GROUND assembler (all primary /
use-case layer). No adapter imports another (rule 5); the composition root wires the registry.
This keeps engines swappable — `sentence-extractor` today, an RLM decomposer (arXiv:2512.24601)
behind the same port tomorrow.

## Consequences

**Positive:**
- Directly attacks the dominant local-model failure mode (truncation) by buying answer headroom on
  every call — cheaper than upgrading tiers.
- Compounds with model tiering: a T1 model on a 50%-smaller prompt is the cheapest viable cell in
  the cost matrix.
- Layer 3 caching makes the GROUND phase — currently the largest static injection — nearly free on
  repeat.
- Instrumented by construction; `compression-report` makes the win measurable and the loss
  auditable.
- Port-based placement leaves the door open to the RLM frontier without re-plumbing consumers.

**Negative:**
- Compression latency on the hot path (every prompt, every tool observation).
- A bad extractor can drop the one line that mattered, sending the agent down a wrong branch.
- Four engines + a registry + a report verb is real surface area to build and maintain.

**Mitigations:**
- Latency: engines are cheap (regex/dedupe for 1a/1b; a small extractive model or heuristic for
  2/3) and Layer 3 is cached — net token savings dwarf compute cost. Budget each engine a hard
  wall-clock cap; on timeout, pass through uncompressed rather than block.
- Signal loss: invariant #2 (semantic, keep diagnostics) + #3 (logged ratios) make regressions
  visible; the bench suite (ADR-2606071734) gains a "compression on/off" axis so a loss-causing
  engine shows up as a task-success regression, not just a token delta.
- Surface area: ship Layers 1b + 2 first (highest leverage on the ReAct loop, generalizing the
  compaction already there); 1a and 3 follow once the port + report are proven.

## Implementation

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P1 | `ICompressionPort` in hex-core + registry skeleton + `.hex/project.json` schema | Pending | code:hex-core/src/ports/compression.rs |
| P2 | Layer 1b `command-compressor` adapter, wired into the ReAct tool-dispatch append | Pending | code:hex-nexus/src/adapters/secondary/compression/, test:cargo test -p hex-nexus compression |
| P3 | Layer 2 `sentence-extractor`, replacing the ad-hoc compaction in `direct_react.rs` | Pending | code:hex-nexus/src/direct_react.rs |
| P4 | `hex inference compression-report` + STDB ratio logging | Pending | test:hex inference compression-report |
| P5 | Layer 1a (`density-filter` on `hex hook route`) + Layer 3 (content-addressed GROUND cache) | Pending | code:hex-cli/src/commands/hook.rs |
| P6 | Add compression on/off axis to the inference bench (ADR-2606071734); confirm no task-success regression | Pending | test:hex bench inference --compression-axis |

## References

- ADR-2606071500 — ReAct tool-use loop (the single-step compaction baseline this generalizes)
- ADR-2605301228 — ADR-governance decision-cards at GROUND (a Layer-3 consumer)
- ADR-2606071713 — code-graph as a first-class agent tool (a Layer-2 context source)
- ADR-2606080915 — resource governor (complementary memory lever — admission vs. compression)
- ADR-2606071734 — agentic inference benchmark suite (gains a compression on/off axis)
- a5c-ai/babysitter — four-layer compression subsystem (density-filter / command-compressor /
  sentence-extractor) claiming 50–67% context reduction; the structural prior art for this ADR
- Zhang, Kraska, Khattab — Recursive Language Models, arXiv:2512.24601 (the frontier beyond
  compaction; a future engine behind `ICompressionPort`)
