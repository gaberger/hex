# ADR-2606061359: Collapse the multi-agent org-sim to a single gateway-mediated agent loop; code-graph context as the differentiator

**Status:** Proposed
**Date:** 2026-06-06
**Drivers:** Operational fragility of the autonomous multi-agent "factory" observed in a live session (unbounded agent-registry growth to 378 rows, ~100-agent spawn churn on daemon restart, persona SOP dispatch not engaging), combined with architectural convergence of the two most successful recent agent frameworks (OpenClaw, Hermes Agent) on a single-agent, tool-centric, memory-driven design.
**Relates-To:** ADR-027 (HexFlo swarm coordination), ADR-2026-03-24-0130 (declarative swarm behavior YAMLs), ADR-2026-05-19-0721 (proposed Self-Improvement Loop — MAPE-K), ADR-2026-04-11-2000 (standalone mode), ADR-2606061359 supersedes none until Accepted.

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

hex today is a multi-agent **organization simulation**: ~33 declared agent types
(C-suite personas — CEO/CTO/CISO/COO/CPO — plus functional roles), a `sched`/brain
daemon that autonomously spawns and dispatches worker instances, an SOP state machine
(`hex-nexus/src/orchestration/sop_executor.rs`) routing board messages to personas,
and HexFlo swarm coordination (ADR-027) over SpacetimeDB. The behavior is declared in
YAML (ADR-2026-03-24-0130) and a self-improvement loop is proposed on top
(ADR-2026-05-19-0721, MAPE-K).

**Empirical evidence (live session, 2026-06-06).** Working in this system surfaced
that the cost lives almost entirely in the multi-agent layer, not the tools:

- The `agent-registry` STDB module grew **unbounded to 378 rows** — `run_agent_cleanup`
  only *marked* agents dead, never deleting them, so spawn churn accumulated across
  restarts (rows are durable in STDB). Fixed in this session, but the growth was a
  direct product of the autonomous-spawn model.
- Restarting the brain daemon produced a **~100-instance spawn surge** from a ~16
  baseline — churn, not steady-state work.
- A board ask dispatched to the `adr-reviewer` persona via the documented SOP path
  (`hex ops send`) **never engaged** — routed, persisted, but no reasoning or action,
  even with the daemon running. The dispatch path for generic personas is unreliable.
- Repeated quiesce/restart cycles were needed to keep the system legible.

By contrast, the **deterministic, single-purpose tools delivered value cleanly**: the
new tree-sitter knowledge-graph engine (`hex-graph`, 17k nodes over the repo in ~1.7s,
fully unit-tested), `hex graph build|query|path|explain|context`, `hex adr`,
`hex analyze`. These are invoked, not orchestrated, and they did not fail.

**External signal.** The two most successful recent open agent frameworks both reject
the multi-agent org model:

- **OpenClaw** (100k+ GitHub stars): a single central **Gateway** (WS+HTTP on one
  port) driving **one agent runtime loop** — Model Resolver (multi-provider, auto
  cooldown) + System Prompt Builder (merges instructions, tools, skills, memories) +
  the receive→tool-call→execute→resubmit loop. Extended by **plugins/skills**, not by
  spawning agents.
- **Hermes Agent** (Nous Research, 140k+ stars, most-used agent on OpenRouter): **one
  persistent agent** with three-level memory, 200+ backends, 40+ tools, and a closed
  learning loop (agent-curated memory + autonomous skill creation). Single agent +
  tools + memory.

Neither runs an org chart of personas; neither spawns swarms. Their leverage is
*tools + context + persistent memory* feeding one strong loop — exactly the part of
hex that worked, and the inverse of the part that hurt.

**Forces & constraints.** hex already owns the right substrate (nexus is effectively a
gateway; SpacetimeDB is a native durable memory/state layer; tiered inference routing
is a Model Resolver; the knowledge graph is a context source no competitor has). The
liability is what we *spawn into* that substrate, not the substrate itself. Genuine
parallelism (e.g. fan-out over many files) is occasionally useful and must remain
possible without resurrecting an always-on fleet.

**Alternatives considered.** (a) Keep the org-sim and harden it — rejected: multiplies
failure modes (coordination, dispatch reliability, cost, lifecycle) for unproven gain.
(b) Adopt OpenClaw/Hermes wholesale — rejected: hex's differentiator (structural code
context) is not in either; we would discard our moat. (c) The hybrid chosen below.

## Decision

**We will refactor hex from a multi-agent organization simulation into a single,
gateway-mediated agent loop whose differentiator is structural code-graph context,
with memory-based self-improvement kept minimal.** Specifically:

1. **Retire the org-sim and the autonomous spawner.** Remove the C-suite persona roles
   and the daemon-driven always-on worker fleet from the default operating mode. Keep a
   small set of *functional* roles (coder, reviewer, tester, planner) that are spawned
   **per task and reaped on completion** — never an idle persistent fleet. The
   bounded-registry fix (delete-on-dead) is the baseline guarantee.
2. **Make nexus the Gateway, explicitly.** Reframe `hex-nexus` as an OpenClaw-style
   gateway: one agent runtime, the existing tiered inference routing as the Model
   Resolver, and a System-Prompt Builder that merges `CLAUDE.md` + active skills +
   **graph context** + memory before each model call. These parts already exist; they
   are to be surfaced as the primary path rather than buried under SOP/swarm.
3. **Make the knowledge graph the differentiator.** Promote the `hex-graph` engine and
   its `graph context` → GROUND wiring to a first-class context provider for the single
   agent loop. Structural code context (a file's consumers, community, neighborhood) is
   hex's unique edge over OpenClaw (execution surfaces) and Hermes (memory).
4. **Keep self-improvement minimal and memory-based.** Use the existing STDB-backed
   `hex memory store lesson:` as agent-curated memory (the Hermes pattern). Do **not**
   adopt the MAPE-K control-theoretic loop (ADR-2026-05-19-0721) — it is the
   over-engineered form of an outcome Hermes achieves with simple memory + skill
   curation.
5. **Keep the substrate, slim the load.** SpacetimeDB remains the durable state/memory
   core and HexFlo remains available for *bounded, opt-in* parallel fan-out; the
   always-on autonomous swarm layer is what we retire.

## Consequences

**Positive.**
- Drastically fewer moving parts and failure modes; the system becomes legible (one
  loop, explicit tools) instead of an emergent fleet.
- Bounded, predictable resource and cost profile — no spawn churn, no ghost agents.
- The moat (code-graph context) becomes the headline capability, not a side feature.
- Aligns hex with the proven direction of OpenClaw/Hermes while keeping a genuine
  differentiator neither has.

**Negative / trade-offs.**
- Loses turnkey large-scale parallel multi-agent throughput as a default; tasks that
  genuinely need fan-out must opt into bounded parallelism explicitly.
- Retires the org-sim narrative/demo value (CEO/CTO role-play).
- Migration effort: deprecating SOP/org-responder paths, swarm defaults, and persona
  YAMLs without breaking the tools that depend on nexus/STDB.
- Risk: if a real workload needs persistent multi-agent coordination, this is a
  regression — mitigated by keeping HexFlo available opt-in.

## Implementation

Phased, tools-preserving:

1. **Stabilize (done / in progress).** Bound the agent registry (delete-on-dead +
   `purge_all_agents`, shipped this session). Default the brain/sched daemon to **off**;
   make autonomous spawning opt-in.
2. **Promote the gateway path.** Define the single agent loop entrypoint over nexus:
   System-Prompt Builder merging `CLAUDE.md` + skills + `graph context` + `lesson:`
   memory; reuse tiered inference as Model Resolver. Treat CLI verbs/skills as the
   plugin/skill surface.
3. **Deprecate the org-sim.** Mark the C-suite persona YAMLs and the always-on SOP
   dispatch as deprecated; reduce `hex-cli/assets/agents/hex/hex/` to functional roles.
   Keep `org_responder`/`sop_executor` only as long as a consumer needs them; schedule
   removal.
4. **First-class context.** Wire `graph context` into the agent loop's prompt assembly
   (beyond the GROUND hook); add a scheduled/post-commit `graph build` so context stays
   fresh.
5. **Minimal learning loop.** Standardize `lesson:`/`gap:` memory curation as the
   self-improvement mechanism; close ADR-2026-05-19-0721 (MAPE-K) as superseded by this
   direction if Accepted.
6. **Bounded parallelism.** Keep HexFlo as an explicit, capped fan-out primitive for
   the rare parallel task — not a default operating mode.

This ADR is **Proposed**: it sets direction and must be reviewed/Accepted before the
deprecations in steps 3 and 5 are executed.

## References

- OpenClaw architecture — https://sfailabs.com/guides/openclaw-ai-agent-framework ,
  https://medium.com/the-ai-language/openclaw-architecture-deep-dive-5579fc546430
- Hermes Agent — https://hermes-agent.nousresearch.com/docs/ ,
  https://codersera.com/blog/hermes-agent-guide-to-multi-agent-ai-setup/
- Anthropic, "Building effective agents" (simple composable patterns over frameworks).
- Session evidence (2026-06-06): agent-registry growth to 378 rows; ~100-agent spawn
  surge; adr-reviewer SOP dispatch non-engagement; `hex-graph` engine + `graph context`.
- Related ADRs: ADR-027 (HexFlo), ADR-2026-03-24-0130 (declarative swarm behavior),
  ADR-2026-05-19-0721 (MAPE-K self-improvement, proposed), ADR-2026-04-11-2000
  (standalone mode).
