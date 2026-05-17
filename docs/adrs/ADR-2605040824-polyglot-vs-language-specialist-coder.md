# ADR-2605040824 — Polyglot hex-coder vs per-language specialist agents

**Status:** Accepted
**Date:** 2026-05-04
**Supersedes:** none
**Superseded by:** none
**Related:** ADR-2603240130 (Declarative Swarm Behavior)

## Context

The current `hex-coder.yml` is polyglot. It accepts a `language` input enum
(`typescript | go | rust`) and dispatches per-language compile/lint/test
commands inside the feedback loop:

```yaml
feedback_loop:
  gates:
    - name: compile
      command:
        typescript: "npx tsc --noEmit"
        go: "go build ./..."
        rust: "cargo check"
```

Constraints, prompt-suffix reminders, and workflow phases are
language-agnostic. The shared model preference is `gpt-4o-mini` (T2)
with sonnet upgrade. One YAML serves all three languages.

The roster-v2 design discussion (2026-05-04) raised the question:
should we split this into `rust-coder.yml`, `go-coder.yml`,
`typescript-coder.yml`?

## Decision

**Defer.** Keep the polyglot `hex-coder` as the default. Specialize
only when a measurable trigger fires (see below). This ADR records
the decision and triggers; it does not authorize the split.

## Triggers that would justify the split

A specialist split is warranted when ANY of these is observed in
production usage of `hex-coder`:

1. **Per-language feedback-loop divergence.** If one language's
   `max_iterations: 5` is consistently insufficient (>20% of tasks
   in that language escalate), the per-language gate semantics
   (Rust ownership errors vs TS type-narrowing vs Go nil-checks)
   may need a language-specific prompt that the polyglot can't
   express.

2. **Per-language tier divergence.** If Go tasks consistently
   succeed at T1 (`qwen3:4b`) but Rust tasks need T2.5
   (`devstral-small-2:24b`), forcing them through the same model
   tier wastes inference dollars on Go and undertrains on Rust.

3. **Per-language prompt-engineering iteration cost.** If we find
   ourselves wanting to A/B test prompt variants per language and
   the shared `prompt_suffix` becomes a coordination bottleneck.

4. **Per-language tooling divergence.** If new languages are
   added (Python, Swift, Kotlin) and the per-language `command:`
   maps inside `feedback_loop.gates[].command` start dominating
   the YAML — that's a smell that the polyglot abstraction is
   leaky.

## Cost analysis (why defer)

**Cost of splitting (3x maintenance):**
- 3 YAMLs to keep in sync with shared_prefix changes
- Supervisor must dispatch on `language` input (currently a no-op)
- README integrity check + agent registry + 3 sets of feedback-loop
  test fixtures
- Skill / hook coupling: any code that references `hex-coder` by
  name needs a routing layer

**Cost of NOT splitting (current state):**
- Suboptimal model tier per language (rust paying for Go-tier model
  or vice versa) — we don't have data on this yet
- Single prompt must capture all three idiom-sets — measurable risk
  if iteration counts diverge

**Inflection point:** when one language's escalation rate exceeds
20% over a 50-task sample, we have evidence to act. Until then the
3x cost outweighs the speculative benefit.

## Alternatives considered

1. **Split immediately** — rejected. No production data on
   divergence; we'd be optimizing a hypothesis.

2. **Inheritance model** (`extends: hex-coder` in language YAMLs,
   override only what differs) — possible, but the declarative
   YAML schema doesn't currently support inheritance and adding it
   is non-trivial. Reconsider when the inflection point hits.

3. **Per-task prompt overrides** (workplan task can specify a
   `coder_prompt_override`) — too granular, easy to abuse, defeats
   the purpose of declarative agents.

## Consequences

**If accepted:** Status: Accepted (operator action required).
- We commit to instrumenting `hex-coder` per-language metrics:
  escalation rate, mean iteration count, mean tokens consumed
- We add a recurring check (monthly?) of those metrics against the
  20% threshold
- We commit to revisiting this ADR when the threshold trips, NOT
  reactively when one Rust task goes badly

**If rejected:** specify which trigger is already firing and why
the polyglot is no longer viable; that's a different ADR (the
split decision itself).

## Implementation (deferred)

When triggered, the implementation work is:

1. Add inheritance to the YAML loader (`extends: <agent_name>` →
   merge child into parent, child wins on conflicts)
2. Create `rust-coder.yml`, `go-coder.yml`, `typescript-coder.yml`
   each `extends: hex-coder` and override only `model.tier`,
   `model.preferred`, language-specific constraints
3. Add language-dispatch routing in supervisor: when a workplan
   task has `language: rust` and a `rust-coder` exists, prefer it
4. Keep `hex-coder` as the fallback for any language without a
   specialist (multilingual tasks, scripting languages)

## Triggers matched

(For pm-agent classification audit — this ADR is itself an
architectural commitment because it changes the agent topology
contract.)

- Hard: "Changes the inference routing topology (tier model,
  escalation policy)" — yes, the deferred implementation would
  change per-language tier routing
- Soft: "Adds a new event topic or message contract" — no
- Workplan-only: "Refactor that preserves all interfaces" — no,
  this introduces a new dispatch contract
