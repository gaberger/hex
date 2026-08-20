# The Self-Reorganizing Hexagon

> A vision plan for hex as substrate for LLM-native applications.

## Context

The user's prompt: *"Imagine a future where applications are built around large language models — they reorganize themselves around new features, performance, capabilities, demand. What would that look like in terms of how hex can reorganize the domain, ports, and adapters, to satisfy an emerging type of application?"*

**Thesis:** applications "built around LLMs" reorganize themselves under demand. Hex is the substrate that makes that reorganization safe by holding the hexagon's invariants (ADR-001) constant while everything else — domain shape, port catalog, adapter selection, agent behavior, swarm topology — becomes data the system can mutate.

The point of this plan is not to imagine hex remaking itself for its own sake. Hex is installed *into* target projects. The climax is **the target app reorganizing itself**; everything else is scaffolding for that.

## What hex already is

Honest inventory, three columns. The pattern is consistent: telemetry is wired, consumption is missing.

| Wired & load-bearing | Scaffolded but inert | Hardcoded — must become data |
|---|---|---|
| Tier routing T1/T2/T2.5/T3 — `InferenceRouterAdapter` reads tier hint, picks model from `TierModelConfig` | RL Q-values — written by `report_outcome()`, read by no inference decision | Boundary rules `TIER0..TIER4` in `hex-cli/src/pipeline/supervisor.rs:37-65` |
| Architecture fingerprint injection — `hex-cli/src/nexus_client.rs:219`, written every session, in every prompt | `spacetime-modules/neural-lab/` — tables + scheduled reducers exist, GPU bridge deferred | Workplan schema in `hex-core/src/domain/workplan.rs` — Rust types |
| Brain daemon — `hex-cli/src/commands/sched.rs`, polls workplans, marks done from git evidence | Escalation rate tracking — planned ADR-2026-04-12-0202 P2 step 8, not implemented | RL reward function in `hex-cli/src/.../model_selection.rs` — hardcoded constants |
| Auto-fix loop — `fix_agent.rs`, max 2 retries | | Composition root in `hex-nexus` — env-gated only, no hot-swap |
| Declarative agent/swarm YAMLs (ADR-2026-03-24-0130) | | MCP tool registry — embedded at compile, project-local override only |
| HexFlo memory across sessions | | |
| Process-boundary adapters — stash sidecar (ADR-2026-04-26-1430) | | |

**The missing closed loop.** Execution outcomes (spec failures, validation rejections, merge conflicts, latency, cost) feed back to *nothing that mutates future runs*. Telemetry exists. Consumers do not.

## The trajectory

Five forward stages. Stages 1 and 2 interleave. Each stage carries a **Target-App Benefit** line — if blank, the stage doesn't ship.

### Stage 1 — Close the existing loops

**What changes.** Connect telemetry to consumers. RL Q-values feed tier routing decisions (advisory → active under flag). Escalation rate becomes a fingerprint input. Auto-fix outcomes mutate agent YAML thresholds. Validation patterns mutate swarm topology weights. *No new infrastructure* — just connect existing wires.

**Invariants preserved.** ADR-001 hexagonal rules; declarative YAML as agent source of truth (ADR-2026-03-24-0130).

**Target-app benefit.** Lower cost-per-task on target work; faster first-pass success on repeat feature shapes.

**Acceptance signal.** One week of advisory logs showing RL-recommended model vs. YAML-default, with cost/latency delta measured against real target-app workloads.

**Files most affected.**
- `hex-cli/src/adapters/secondary/inference_router.rs` (or wherever `InferenceRouterAdapter` is wired)
- `spacetime-modules/rl-engine/src/lib.rs` (add a read-side subscription pattern)
- `hex-cli/src/pipeline/model_selection.rs` (the handoff point)
- `hex-cli/src/nexus_client.rs:219` (fingerprint extension)

### Stage 2 — Data-fy the hardcoded surfaces

**What changes.** Boundary rules → SpacetimeDB tables. Workplan schema → versioned JSON Schema with migrations. Reward function → user-editable expression. Composition root → config-driven adapter selection.

**Precedent.** ADR-2026-04-14-2243 already established "Rust constants → SpacetimeDB tables" as a sanctioned migration pattern (classifier rules), with structural-assertion testing. Boundary rules follow the same path.

**Invariants preserved.** ADR-001 still holds — but now its rules are queryable, versionable, and per-project overridable instead of edit-recompile.

**Target-app benefit.** Target apps need different boundary rules than hex itself (e.g. a game has different layering than a SaaS API). Data-fy unlocks per-target customization without forking the kernel.

**Acceptance signal.** A target app running with locally overridden boundary rules from `.hex/project.json`, validated by an unmodified `hex analyze`.

**Files most affected.**
- `hex-cli/src/pipeline/supervisor.rs:37-65` (boundary-rules read site)
- `spacetime-modules/hexflo-coordination/src/lib.rs` (new boundary-rules table)
- `hex-core/src/domain/workplan.rs` (schema versioning)
- `hex-nexus/src/main.rs` (composition selector)

### Stage 3 — The adapter plane becomes a registry

**What changes.** Adapters move from compile-time wiring to runtime registration. Two coexisting paths: **WASM adapters** (in-process fast path, extending what SpacetimeDB modules already do) and **process-boundary adapters** (polyglot heavy path, per ADR-2026-04-26-1430). Adapter registry lives in `spacetime-modules/agent-registry/`. Adapters can be replaced under load without restart.

**Invariants preserved.** Adapters still implement port traits exactly as today. The registry is a discovery mechanism, not a contract change.

**Target-app benefit.** A target app under cost pressure can swap a Claude Sonnet adapter for a local Qwen adapter for one workload without redeploying. A target app gaining a new capability (vector search, payments, telemetry) gains a port + adapter without a kernel rebuild.

**Acceptance signal.** Live adapter swap in a running target app: same port, two adapters, traffic shifts based on a live policy.

**Files most affected.**
- `spacetime-modules/agent-registry/src/lib.rs` (new adapter-registry table)
- `hex-nexus/src/composition/mod.rs` (registry-driven composition)
- `hex-nexus/src/orchestration/` (sidecar lifecycle, already established for stash)

### Stage 4 — Meta-architecture proposals (target-app-first)

**What changes.** Hex observes the target app's metrics, drafts ADRs proposing port additions/swaps in *the target's hexagon*, runs the workplan, instantiates the adapter. Human ADR-approval gate preserved. Hex's own self-modification is the special case where target = hex.

**Invariants preserved.** ADR human-approval is non-negotiable. Specs-first pipeline (ADR → spec → workplan → code) holds.

**Target-app benefit.** The target app evolves on observed signal, not on developer intuition. The "I never thought to look at that" feature gap closes.

**Acceptance signal.** An ADR drafted by hex, approved by a human, that materially improves a measurable target-app KPI within one week of merge.

**Files most affected.**
- `hex-cli/src/pipeline/adr_phase.rs` (extend to take target-app metrics as input)
- `hex-cli/src/commands/adr.rs` (`hex adr propose-from-metrics`)
- `spacetime-modules/hexflo-coordination/src/lib.rs` (target-metric subscription)

### Stage 5 — Continuous reorganization

**What changes — and this is the climax.** Domain types in the target app reorganize on observed access patterns. Adapters swap on cost/latency. Ports get added when a capability gap appears in usage. Workplan templates self-tune from win/loss history. The target app *is* the LLM-native application the user asked about — and it falls out of Stages 1–4 applied to user code instead of hex itself.

**Invariants preserved.** Same hexagonal contract. Same human gate at the ADR layer. Same `hex analyze` rejection of cross-layer imports.

**Target-app benefit.** The target app becomes an organism rather than a product. New users → new ports. Cost pressure → adapter swap. Capability emergence → domain extension. The developer tends the substrate; hex tends the shape.

**Acceptance signal.** A target app where the domain, port catalog, and adapter set at month 6 differ measurably from month 1 — and every diff is traceable to an ADR, a workplan, and a target-app metric that justified it.

## The first step (lowest-risk, ships in week 1)

**Wire RL Q-values into `InferenceRouterAdapter` tier routing as an advisory signal behind a feature flag.**

Concretely:
- `InferenceRouterAdapter` reads the existing `q_value` row from `spacetime-modules/rl-engine` for `(tier, task_type, candidate_model)`.
- `HEX_RL_ROUTING=advisory` (default): log RL pick alongside YAML/default choice. **No behavior change.**
- `HEX_RL_ROUTING=active` and `visit_count >= N`: use the RL pick.
- Reversible by env var.
- Zero new tables, zero schema changes, zero domain changes.

This is the smallest possible demonstration that **feedback signals are no longer dead-ended** — and the first concrete proof the trajectory works.

## Risks and invariants

- **Recursive-introspection trap.** Hex must not become about hex. Every stage's acceptance signal is a *target-app* metric, not a hex-internal metric. If a stage can't show a target-app win, it doesn't ship.
- **WASM ↔ sidecar coexistence.** They are not competing. WASM is the in-process fast path; sidecars are the polyglot/heavy path. The decision rule is language and weight: Rust-on-WASM if it fits, sidecar otherwise.
- **ADR-001 is non-negotiable** at every stage. Domain only knows itself; ports only know domain; adapters only know ports.
- **Rust kernel stays canonical** (ADR-2026-04-26-1800 — phantom TS surface excised).
- **Apache-2.0 / MIT discipline** holds. Stash and any future sidecar carry their license verbatim.
- **Human ADR gate stays** through Stage 4. Autonomous code generation, gated decision-making.

## What this is not

- **Not a rewrite.** Every stage extends what already exists.
- **Not a research project.** Every stage has a target-app acceptance signal before it ships.
- **Not autonomy without gates.** ADR human-approval is preserved through Stage 4 and the climax in Stage 5.
- **Not "hex becomes self-aware."** Hex remains a substrate. The reorganization happens to *target apps*; hex's own evolution is the special case.

## Verification

How to test that the vision is being realized end-to-end:

1. **Stage 1 evidence.** `target/release/hex` running with `HEX_RL_ROUTING=advisory` for one week against a target app. Check `~/.hex/rl-routing-log.jsonl` for non-empty advisory log; verify cost/latency delta. Pass: ≥10% measurable cost reduction without quality regression in `validation-judge` outcomes.
2. **Stage 2 evidence.** A target app overrides boundary rules in `.hex/project.json` and `hex analyze` passes against the override, fails when reverted. Pass: rule precedence is queryable via SpacetimeDB, not source-grep-able only.
3. **Stage 3 evidence.** `hex agent swap <port> <new-adapter>` mid-flight; same port, two adapters, traffic policy honored. Pass: zero restart, observable in `hex pulse`.
4. **Stage 4 evidence.** One ADR in `docs/adrs/` authored by hex, approved by a human, traceable from a target-app metric to the merged port + adapter.
5. **Stage 5 evidence.** A target app's `hex analyze` fingerprint at month 6 differs from month 1 in port count or adapter mix, with every delta cross-referenced to an ADR + workplan + target-app metric in `docs/evidence/`.

## Critical files (read order for an implementer)

1. `docs/adrs/ADR-2026-04-14-2243-classifier-rule-tables.md` — the precedent pattern for Stage 2.
2. `hex-cli/src/pipeline/supervisor.rs:37-65` — the boundary rules to data-fy.
3. `hex-cli/src/pipeline/model_selection.rs` — the RL handoff point for Stage 1.
4. `hex-cli/src/nexus_client.rs:219` — the fingerprint extension point.
5. `spacetime-modules/rl-engine/src/lib.rs` — the dead-end to revive.
6. `spacetime-modules/agent-registry/src/lib.rs` — the seed of Stage 3.
7. `hex-cli/assets/agents/hex/hex/feature-developer.yml` — the agent YAML that will gain mutation surfaces in Stage 1.
8. `docs/adrs/ADR-2026-04-26-1430-stash-consolidation-memory-port.md` — the WASM/sidecar coexistence precedent for Stage 3.

---

**Concluding image.** A target app running on hex one year from now: a hexagon whose six edges have shifted in shape since launch, every shift traceable to an ADR, a workplan, a measured signal. Developer intuition still authors the original constraints; hex tends to the rest. The kernel hasn't been rebuilt. The application has reorganized itself.
