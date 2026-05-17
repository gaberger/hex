# ADR-2026-04-26-1311: Six-layer governance — adversarial swarms, judges, and structural opposition to accretion

**Status:** Accepted
**Date:** 2026-04-26
**Accepted:** 2026-04-26
**Drivers:** Hexagonal architecture told us *where* code goes once we'd decided we needed it. It did not tell us whether we needed it. The result is visible in the repo: 57 files in `hex-cli/src/commands/` (vs. ~10 user-facing verbs), 188 ADRs, 57 active workplans, 7 SpacetimeDB modules where 2 would do, an `inference_router` doing string-parsing on `"model:X|context:Y"` compound actions. Each addition was locally justified; the system as a whole drifted away from its founding goals (model tiering & independence, multi-host scaleout, hexagonal rigor). LLM agents amplify this failure mode because they ship faster than humans, with more confidence, and no agent's KPI rewards declining the task or deleting code.
**Supersedes:** None — this is a meta-policy ADR. It governs *how* future ADRs and workplans are produced.

## Context

### The meta-problem

Architecture rules answer "where does this code go?" They do not answer "should this exist at all?" Today's hex agents, hooks, skills, and workplan executor implicitly answer "yes" to every "should this exist" question that crosses the workflow. There is no agent in the loop whose KPI is *rejection*.

This is not an LLM-specific failure — humans drift the same way. But LLMs do it faster and with more confidence:
- Each turn is **locally optimal** (the agent solves the task it was given)
- No agent **owns global cost** (nobody's job is "the system already does this")
- Tests pass, build is green, ADR cites the prior ADR — every checkbox passes while the system gets worse
- Training rewards "ship code"; nothing in the loop rewards "delete code" or "decline the task"

### Empirical evidence the current system has the failure mode

Measured at HEAD on 2026-04-26:

| Metric | Current | Reasonable target | Overage |
|--------|---------|-------------------|---------|
| `hex-cli/src/commands/` entries | 57 | 15 | 3.8× |
| Active ADRs | 188 | 60 | 3.1× |
| Active workplans | 57 | 25 | 2.3× |
| SpacetimeDB modules | 7 | 2 | 3.5× |
| `inference_router/mod.rs` lines after RL bolt-on | 750+ | 200 | 3.7× |
| Workplans showing `status=Failed` (boot banner) | 55 of 55 | 0 | symptom |

Every overage was the result of locally-correct decisions. No single PR was the problem. **The accretion *is* the problem.**

### What current hex *has* that's adjacent but not wired as a gate

| Component | What it does today | What's missing |
|-----------|-------------------|----------------|
| `adversarial-reviewer` agent | Reviews on request | Not in the path — workplans dispatch without invoking it |
| `behavioral-spec-writer` | Produces spec JSON | No spec→test compilation; code agent reads spec directly and games it |
| `dead-code-analyzer` | Runs on demand | Not scheduled; doesn't auto-produce a deletion workplan |
| `validation-judge` | Post-build semantic check | No authority to block merge |
| `dependency-analyst` | Assumes work happens | Necessity-of-work check doesn't exist |
| Budget manifest | — | Doesn't exist |
| Shrinkage daemon | — | Doesn't exist |
| Quarterly redesign ritual | — | Doesn't exist |

**The components exist; the *gates* do not.** Advisory ≠ gate. A gate has the authority to stop the workflow.

### The unifying principle

> No single agent should have both the goal of shipping work and the authority to approve it.

This invariant is violated everywhere in the current system. Proposer == reviewer == approver in 90% of paths. Once that invariant is restored, the system stops drifting.

## Decision

We will introduce **six governance layers** that wrap the existing hex workflow. Each layer is a *structural mechanism* (a gate, a daemon, a budget, a ritual) — not a doc, not a guideline, not a prompt addendum.

### Layer 1 — System budget manifest

A declarative file (`docs/budget.yaml`) that pins maximum sizes for the system. A pre-commit hook + CI gate measures and reports on every commit. Adding a 51st ADR or a 16th `commands/` file requires either:
- Raising the budget *with justification in an ADR* (so growth is deliberate, not drift), or
- Deprecating an equivalent surface elsewhere

**Why this works:** Linux's "no regressions" rule, Go's "no new keywords," sqlite's "amalgamation must stay one file." The constraint is the design tool. Without a declared size, nothing is "too big" — every addition is locally justified.

**Initial mode:** advisory (script reports, doesn't block). Graduates to enforcing once teams adapt and the existing overages are paid down.

### Layer 2 — Necessity gate

Before any workplan executes, an adversarial agent runs the **necessity check**:

> "Given the current system at HEAD, prove this work needs to exist. Search the codebase for existing solutions. Cite three. Explain why each is insufficient. If you can't, return REJECT with the existing-solution citation."

**KPI of this agent: rejected workplans.** The more it kills, the better it's doing. This is the inverse of every other agent in the system.

Output is binary: ACCEPT (with reason) or REJECT (with the existing code that should be extended instead). REJECT is dispositive — the workplan moves to `docs/workplans/rejected/` with the reasoning attached, and the original requester sees the citation.

Runs *before* `hex plan draft` writes the JSON. Drafts that fail necessity never become workplans.

### Layer 3 — Adversarial decomposition swarm with binding judge

For workplans that pass Layer 2, decomposition becomes a **debate**, not a soliloquy. Three roles run in parallel against the same inputs:

| Role | Mandate | KPI |
|------|---------|-----|
| **Proposer** | Drafts the workplan (the existing path) | Acceptance |
| **Minimizer** | Smallest possible workplan that still satisfies the spec | Phases removed, tasks collapsed |
| **Skeptic** | Counter-scenarios that would break the design | Unhandled cases found |

A **judge** agent (different model family from the proposer — Claude judges GPT, or vice versa, to break correlated failure) reads all three outputs and produces:
- ACCEPT (proposer's version)
- ACCEPT-MODIFIED (judge issues the merged plan)
- REJECT-RESPEC (problem isn't workplan-shaped; back to spec phase)

**Judge output is binding.** The workplan executor refuses to dispatch any plan without a judge verdict file alongside it (`docs/workplans/wp-X.json` + `docs/workplans/wp-X.judge.json`).

The asymmetry is the point: proposer wants to ship, minimizer wants to cut, skeptic wants to break, judge wants to be right. **No single agent has both the goal of acceptance and the power to grant it.**

### Layer 4 — Continuous shrinkage daemon

A scheduled agent (weekly cadence) whose job is to **delete code**.

Inputs: full repo + git log of the last 30 days + every workplan marked done + telemetry on command invocation frequency.

Outputs: `docs/workplans/wp-shrink-YYYYMMDD.json` listing:
- Files unchanged for >90 days that are imported by ≤1 caller
- Adapters with zero callers in the last 60 days
- Workplans marked done whose touched files have been entirely rewritten since
- ADRs whose decision has been superseded but not formally marked
- Commands in `hex-cli/src/commands/` invoked <1× per week per telemetry (not heuristic)

Each item is its own task. The user can accept/reject per-item, but **the baseline assumption is delete**. Today, deletion requires action. Under this layer, retention requires action. Default inverts.

### Layer 5 — Spec-as-contract (not spec-as-doc)

Today's spec writer produces JSON, then code is written to satisfy it. But the spec isn't *enforced* — the code agent reads the spec for context, then writes whatever it thinks satisfies it. Same agent writes the test, so tests encode the same misunderstanding the code does. (This is your own hard-won lesson from CLAUDE.md.)

New loop:

1. **Spec writer** produces the spec. Judge reviews the *spec* (not code) for: testability, completeness, no implementation leakage. Spec author and judge are different agents.
2. **Spec compiler** (deterministic, no LLM) compiles the spec to a property-test suite. Same spec → same tests.
3. **Code agent** writes code to make the suite pass. **Never sees the spec; only sees failing tests.** Cannot game the test because it didn't write the test.
4. **Code judge** (adversarial) reviews the *passing* code against the original spec, looking for: tautological tests, tests that pass but don't test what they claim, behaviors that satisfy the test letter but violate the spec spirit.

Three different agents, three different KPIs, three different code-paths. The spec author can't game the test (didn't write code). The code agent can't game the test (didn't write spec). The judge's KPI is finding mismatches.

### Layer 6 — Periodic system-redesign ritual

Quarterly cadence. A **fresh** swarm with no memory of prior decisions reads:
- Current system at HEAD
- Original founding goals (committed file, not ephemeral)
- Layer 1 budget manifest

…and produces:

> "If you were starting from scratch today with the same goals, what would you build, and what is the delta from current?"

Output: an ADR with phased migration, ready for Layer 2's necessity gate.

Most quarters this produces small adjustments. Periodically it produces something the size of ADR-2026-04-26-1303 ("the thing you have isn't the thing you wanted, here's the path back"). The ritual is the only mechanism that catches a system drifting away from its founding goals over months.

**Why a ritual, not on-demand:** On-demand redesign happens after pain has already accumulated. Cadenced redesign catches drift before it becomes an emergency.

### How the layers compose

```
[user intent]
    │
    ▼
┌───────────────────────────┐
│ Layer 2: Necessity gate   │ ─REJECT─> docs/workplans/rejected/
│ (adversarial agent)       │           citation to existing code
└───────────┬───────────────┘
            │ ACCEPT
            ▼
┌───────────────────────────┐
│ Layer 5: Spec-first       │
│ (judge reviews spec only) │
└───────────┬───────────────┘
            ▼
┌───────────────────────────┐
│ Layer 3: Adversarial      │
│ decomposition swarm       │
│ Proposer | Minimizer      │ ─REJECT-RESPEC─> back to spec
│ Skeptic  | Judge          │
└───────────┬───────────────┘
            │ ACCEPT-MODIFIED
            ▼
┌───────────────────────────┐
│ Layer 1: Budget gate      │ ─FAIL─> requires ADR raising budget
│ (CI mechanical check)     │         or deprecating equivalent surface
└───────────┬───────────────┘
            ▼
       [execute workplan]
            │
            ▼
┌───────────────────────────┐
│ Layer 5: Code judged vs.  │
│ spec by adversarial agent │
└───────────┬───────────────┘
            ▼
       [merge / done]

[weekly]    Layer 4: Shrinkage daemon ──> wp-shrink-*.json
[quarterly] Layer 6: Redesign ritual   ──> ADR-redesign-*
```

## Consequences

**Positive:**
- The accretion failure mode becomes structurally impossible — every shipping path passes through ≥1 agent whose KPI is rejection.
- Code-as-tested is decoupled from code-as-written (Layer 5), eliminating the "tests mirror bugs" failure your CLAUDE.md already calls out.
- System size is declared, not emergent. Drift is visible the moment it starts (Layer 1).
- Existing components (`adversarial-reviewer`, `validation-judge`, `dead-code-analyzer`, `behavioral-spec-writer`) finally have authority that matches their job descriptions.
- Cross-model judging (Layer 3) breaks correlated failure modes — Claude judging Claude can't catch what both miss, but Claude judging GPT can.

**Negative:**
- Throughput drops. Every workplan now passes through 2–4 additional gate steps. Estimated 1.5–2× turnaround on small changes, ~1.2× on large ones (where gates amortize better).
- Compute cost rises proportionally — adversarial swarms double or triple inference spend per workplan.
- Agents will sometimes deadlock (necessity rejects, user disagrees, no override path). Need an explicit human-override mechanism.
- The judge agent itself is a single point of failure. Cross-family rotation (Claude/GPT/local) is mandatory, not optional.
- Budget gates initially fail loudly because the current system is over every reasonable limit. Risk: the team learns to ignore the warning. Mitigation: ship advisory first, graduate to enforcing only after current overages are paid down.

**Mitigations:**
- Each layer ships as **advisory first, enforcing later**. Layer 1 prints warnings for 30 days; only then becomes a CI hard-fail. Same pattern for Layers 2–5.
- Human-override is a single CLI command (`hex gate override --reason "..."`) that records the override + reasoning to `docs/governance/overrides.log`. Override count per layer is itself a KPI — high override counts mean the gate is mis-tuned, not that it should be removed.
- Layer 6 (redesign ritual) is the meta-check: if Layers 1–5 are mis-tuned, the quarterly ritual catches it.
- Compute cost bounded by running adversarial swarms only on workplans above a complexity threshold (small changes still go through the standard path). Threshold itself is a tunable in `docs/governance/policy.yaml`.

## Implementation

| Phase | Layer | Description | Exit criteria | Status |
|-------|-------|-------------|---------------|--------|
| **G1** | Layer 1 | Ship `docs/budget.yaml` + `scripts/check-budget.sh` (advisory). Wire into pre-commit + CI as warn-only. | Script runs in <2s; reports current overages without blocking. | Pending |
| **G2** | Layer 2 | Build necessity-gate agent (`agents/necessity-judge.md`) + integrate into `hex plan draft`. Reject path goes to `docs/workplans/rejected/`. | Agent rejects ≥1 of 5 dummy workplans designed to be unnecessary. | Pending |
| **G3** | Layer 4 | Schedule shrinkage agent (weekly via brain daemon). Output is auto-enqueued workplan. | Shrinkage agent run on current repo produces a non-empty deletion workplan within 5 min. | Pending |
| **G4** | Layer 1 graduation | Layer 1 script flips from advisory to enforcing. Existing overages either resolved or whitelisted-with-ADR. | Pre-commit + CI hard-fail on new overages. Whitelist file references an ADR per entry. | Pending |
| **G5** | Layer 3 | Build proposer/minimizer/skeptic/judge swarm. Workplan executor refuses to dispatch without judge verdict file. | Three sample workplans pass through the swarm; judge verdict file generated and binding. | Pending |
| **G6** | Layer 5 | Build spec compiler (spec JSON → property tests). Code agent runs against tests only. Spec/code/judge are different agents. | Round-trip on one feature: spec → tests → code → judged. Code agent never reads spec. | Pending |
| **G7** | Layer 6 | Schedule quarterly redesign ritual via brain daemon. Output is an ADR draft + delta report. | First ritual run produces an ADR draft on a known drift area. | Pending |

Each phase ends with a working system; the order is chosen so cheap+high-impact ships first. Layers 1, 2, 4 deliver most of the value. Layers 3, 5, 6 are the long tail — necessary but expensive.

## References

- ADR-2026-04-26-1303 — IModelProvider port + crate split (the *what* — this ADR is the *how to avoid building it again the same way*)
- CLAUDE.md "Key Lessons (from adversarial review)" — "Tests can mirror bugs" is the empirical motivation for Layer 5
- CLAUDE.md "Autonomous Operation (HARD RULES)" — those are guidelines for one agent; this ADR introduces structural opposition between agents
- The current `adversarial-reviewer`, `validation-judge`, `dead-code-analyzer`, `behavioral-spec-writer` agents — components that become *gates* under this ADR rather than advisory tools
- Linux kernel "no regressions" policy, Go "no new keywords" policy, sqlite amalgamation rule — empirical precedents for budget-as-design-tool (Layer 1)
- Boot banner symptom: 55/55 workplans `status=Failed` is exactly what unchecked accretion looks like — every workplan is locally valid; the system as a whole has stopped working
