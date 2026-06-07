# ADR-2606071340: hex-nexus must comply with hexagonal architecture — excise org-sim, split into crates behind ports, stay one daemon

**Status:** Accepted
**Date:** 2026-06-07
**Epoch:** single-agent
**Drivers:** `hex analyze hex-nexus` — hex's own boundary checker, run on its own daemon — grades nexus **F (30/100) with 7 boundary violations**. At **117,033 LOC in a single crate**, nexus is a god-daemon that violates the architecture it enforces on every target project. ~31% of it (`orchestration/`, 35,934 LOC) is **retired org-sim machinery** whose ADRs were superseded 2026-06-07 (ADR-2606061359 capstone + the 22 supersessions). The platform is not eating its own dog food.
**Relates-To:** ADR-2606061359 (collapse org-sim to single-agent loop — the org-sim code excised here is its retired surface), ADR-2606071323 (autonomous worktree isolation — `hex-exec` is one of the bounded contexts), ADR-2026-04-05-0900 (trace ALL consumers before deleting), ADR-2026-05-09-1430 (workspace boundary enforcement), ADR-001 (hexagonal architecture), ADR-2026-04-06-1000 (tree-sitter as the enforcement engine).

<!-- LIFECYCLE: Proposed → Accepted → Completed. Change Status only via adr_status_set / the adr-steward. -->

## Context

hex enforces hexagonal rules (`domain` ⊳ `domain` only; `ports` ⊳ `domain`; `usecases` ⊳
`domain`+`ports`; adapters ⊳ `ports` only, never other adapters; the composition root is the
ONLY importer of adapters) on every project it installs into, checked by `hex analyze`. Run
against nexus itself, the verdict is **F, 30/100**:

```
✗ ports/agent_lifecycle.rs   → remote/transport/*  (ports/ may only import from domain/)
✗ ports/inference_router.rs  → remote/transport/{  (ports/ may only import from domain/)
✗ usecases/remote_agent_orchestrator.rs → remote/transport/*  (usecases/ may only import domain/+ports/)
  … 7 total — ports and usecases reaching into a transport adapter
```

Beyond the literal violations, the crate's scale is the deeper problem. **117,033 LOC** mix
many bounded contexts in one binary:

| Subsystem | LOC | Character |
|---|---|---|
| `orchestration/` | 35,934 (31%) | **mostly retired org-sim**: `org_responder` (2675), `sop_executor` (2058), `drafter` (2093), `twin_reviewer` (1033), `hive_improver`, `gap_dispatcher`, `persona_prompt_*`, `swarm_task_*`, `commitment_parser`, `adversarial_swarm` |
| `routes/` | 26,155 (22%) | HTTP (a primary adapter) tangled with logic; `sop.rs`, `swarms.rs`, `brain_chat` (2567) |
| `adapters/` | 14,103 | STDB + SQLite state |
| `tools/` / `analysis/` | 4,911 / 4,833 | the single-agent loop's tools; tree-sitter (near-standalone) |
| `coordination/` / `git/` | 1,589 / 1,422 | HexFlo; worktree ops |

Consequences of the monolith already observed this session and in memory: ~2-minute compile
cycles, busy-spin from arena contention with everything co-located in one process, and a STDB
deserialize fault that would take the dashboard + executor down together.

**Forces:**
- hex's credibility depends on its own daemon passing the rules it enforces.
- The single-agent pivot (ADR-2606061359) retired ~25–30k LOC that still ships in nexus —
  dead weight that confuses readers and inflates the surface.
- Decomposition must not recreate the operational fragility that the org-sim retirement was
  *about* (unbounded process churn, dispatch that routes but never engages).

**Alternatives considered:**
- *Split nexus into multiple daemons/processes.* Rejected as the default — multiplies process
  management, IPC, heartbeats, and deploy complexity, recreating org-sim-era fragility at the
  infra layer. Reserved for a single concern later, behind measured need.
- *Just fix the 7 violations.* Necessary but insufficient — leaves the 117k-LOC monolith and
  the retired org-sim surface intact.
- *Split first, delete dead code later.* Rejected — carries corpses into new crates and
  triples the tracing surface. Excision MUST precede the split.

## Decision

Bring hex-nexus into compliance with hex's own hexagonal rules, in a strict order, while
keeping it a **single daemon**.

### Phase 0 — Excise the retired org-sim code FIRST
Delete the org-sim machinery superseded by ADR-2606061359 and the 2026-06-07 supersessions:
`sop_executor`, `org_responder`, `drafter`, `twin_reviewer`, `twin_deterministic`,
`hive_improver`, `gap_dispatcher`, `persona_prompt_*`, `swarm_task_bridge/drainer`,
`commitment_parser`, `adversarial_swarm`, and their routes (`routes/sop`, `routes/swarms`,
the org-sim half of `brain_chat`) and adapters (`adapters/spacetime_persona`). Each removal
**traces consumers across the ENTIRE workspace first** (ADR-2026-04-05-0900 — feature-gated
imports broke hex-agent before) and ends with `cargo check --workspace` + `hex analyze`. This
removes ~20–25k LOC before any restructuring.

### Phase 1 — Split the surviving concerns into crates behind ports
Extract the clean bounded contexts as workspace crates, each behind its port trait:
- `hex-analysis` — tree-sitter + boundary checking (near-standalone; the CLI may consume it directly)
- `hex-git` — worktree + git operations
- `hex-exec` — `direct_exec` / `direct_react` / `direct_workspace` + the curated tools (the single-agent loop)
- `hex-coordination` — HexFlo
- `hex-state` — STDB + SQLite adapters behind `IStatePort` / `ICoordinationPort`

`hex-nexus` (the binary) becomes the **composition root**: axum/HTTP as the primary adapter,
the dashboard host (rust-embed), and DI wiring — the only place that imports adapters
(hex rule #6). The 7 `ports → remote/transport` violations are fixed here (transport is an
adapter; ports must not see it — move the shared types to domain or invert the dependency).
Target: `hex analyze hex-nexus` grade ≥ B and 0 boundary violations.

### Phase 2 — Stay one daemon
Do NOT process-split. nexus remains a single binary, internally decomposed. A specific
concern (e.g. inference routing under load) may become its own process **later, only behind
measured need** — never speculatively. This preserves the single-agent thesis: fewer moving
parts, not more.

## Consequences

**Positive:**
- nexus passes the architecture hex enforces — the platform eats its own dog food.
- ~20–25k LOC of dead org-sim code gone; faster compiles; clear bounded contexts; concerns
  independently testable; `hex-analysis`/`hex-git`/`hex-exec` reusable beyond the daemon.
- Fault surfaces become legible (a state-adapter fault is contained to `hex-state`).

**Negative:**
- Large, multi-phase effort touching the hottest crate; risk of dropping a feature-gated
  consumer during excision.
- Crate extraction churns imports across the workspace and the CI/build scripts.

**Mitigations:**
- Strict sequencing (excise → split → never process-split) with `cargo check --workspace` +
  `hex analyze` as a gate **between every step** (the "build gates between phases" lesson).
- Trace consumers workspace-wide before each deletion (ADR-2026-04-05-0900).
- One bounded context per workplan/PR; merge in dependency order (analysis/git → exec →
  coordination → state → composition root).

## Implementation

| Phase | Description | Status | Verification |
|-------|------------|--------|--------------|
| P0 | Excise retired org-sim modules (orchestration + routes + adapters) | Pending | test:cargo check --workspace, test:hex analyze hex-nexus |
| P1a | Extract `hex-analysis` + `hex-git` crates behind ports | Pending | code:hex-analysis/Cargo.toml, code:hex-git/Cargo.toml |
| P1b | Extract `hex-exec` (direct_*, tools) + `hex-coordination` | Pending | code:hex-exec/Cargo.toml |
| P1c | Extract `hex-state` (STDB+SQLite behind IStatePort); fix 7 ports→transport violations | Pending | test:hex analyze hex-nexus |
| P1d | nexus binary = composition root (HTTP + dashboard + DI only) | Pending | test:hex analyze hex-nexus |
| P2 | Single-daemon invariant documented; no process split | Pending | code:ARCHITECTURE.md |

## References

- `hex analyze hex-nexus` (2026-06-07): grade F, 30/100, 7 boundary violations
- ADR-2606061359 — single-agent loop (defines the org-sim surface excised in P0)
- ADR-2606071323 — autonomous worktree isolation (`hex-exec` bounded context)
- ADR-2026-04-05-0900 — trace ALL consumers before deleting
- ADR-001 — hexagonal architecture; ADR-2026-04-06-1000 — tree-sitter enforcement engine
