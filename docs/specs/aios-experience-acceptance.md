# AIOS Experience — Acceptance Specification

**ADR**: ADR-2604131500
**Workplan**: wp-aios-experience-p1
**Date**: 2026-04-13

This spec defines how we know Phase 1 is done — not "code exists" but "the experience works."

---

## The Claim

> A developer can manage 3 active hex projects with ≤15 minutes of total hex interaction per day, without losing situational awareness or control.

## Scenario Test: "Sarah's Monday"

This is a scripted end-to-end scenario that a human evaluator walks through. It is the primary acceptance gate. The scenario must be completable without reading source code, checking SpacetimeDB directly, or opening any tool other than the terminal and (once) the browser.

### Setup

Three projects exist in hex with varying states:
- **Project Alpha** — actively building (Phase CODE, 4 agents running)
- **Project Beta** — has 2 pending decisions (dependency approval + architecture choice)
- **Project Gamma** — idle (completed yesterday)

The evaluator has NOT looked at hex for 8 hours (simulating overnight).

### Act 1: Glance (< 5 seconds)

**Action**: Evaluator opens terminal. The Pulse statusline is visible.

**Pass criteria**:
- [ ] Pulse renders within 2 seconds of terminal open
- [ ] Alpha shows `●` (active) with agent count
- [ ] Beta shows `◐` (decisions pending) with decision count
- [ ] Gamma shows `○` (idle) or `✓` (complete)
- [ ] Evaluator can answer "do I need to do anything?" by looking at ONE line
- [ ] No scrolling, no commands typed yet

**Fail if**: Evaluator has to type a command to know whether anything needs attention. Pulse shows raw JSON or error. More than one line of output per project.

### Act 2: Read (< 60 seconds)

**Action**: Evaluator types `hex brief`.

**Pass criteria**:
- [ ] Output appears within 3 seconds
- [ ] Alpha section tells a story: what happened overnight (N tasks completed, health score change, spend)
- [ ] Beta section shows exactly 2 decisions, each with:
  - [ ] Numbered identifier usable in `hex decide`
  - [ ] What hex chose as the default
  - [ ] Why hex chose it (one sentence)
  - [ ] Deadline for auto-resolution
  - [ ] Copy-pasteable `hex decide` commands
- [ ] Gamma section is ≤2 lines ("completed, no action needed")
- [ ] Total output fits in one terminal screen (≤40 lines for 3 projects)
- [ ] Evaluator understands the state of all 3 projects without scrolling up

**Fail if**: Output is a wall of JSON. Decisions don't include copy-pasteable commands. No deadline shown. Evaluator needs to run a second command to understand any project's state. Output exceeds 60 lines.

### Act 3: Decide (< 30 seconds)

**Action**: Evaluator resolves both of Beta's decisions.

**Pass criteria**:
- [ ] `hex decide beta 1 approve` completes in <2 seconds
- [ ] Confirmation message says what will happen ("✓ Redis 0.25.0 added to dependencies")
- [ ] `hex decide beta 2 override "use token bucket"` completes in <2 seconds
- [ ] Confirmation says what changed and what hex will do next
- [ ] Running `hex brief --project beta` now shows 0 pending decisions
- [ ] Pulse updates: Beta changes from `◐` to `●`

**Fail if**: Decision commands require IDs the evaluator can't find in the briefing. Confirmation is generic ("decision resolved") with no specifics. Pulse doesn't update after decisions.

### Act 4: Steer (< 20 seconds)

**Action**: Evaluator steers Alpha's priorities.

**Pass criteria**:
- [ ] `hex steer alpha "finish the API adapter first, I need to demo Wednesday"` completes in <3 seconds
- [ ] Confirmation shows what changed: "✓ API adapter prioritized. N tasks reordered."
- [ ] Subsequent `hex brief --project alpha` reflects the new priority
- [ ] No agent IDs, task IDs, or workplan internals appear in any output

**Fail if**: Evaluator must know internal task/agent IDs. Steer silently succeeds with no confirmation. No evidence the priority changed in the briefing.

### Act 5: Trust (< 30 seconds)

**Action**: Evaluator checks and adjusts trust levels.

**Pass criteria**:
- [ ] `hex trust show alpha` displays a readable tree:
  ```
  alpha
  ├── domain/          act     (elevated 2d ago)
  ├── ports/           act     (elevated 2d ago)
  ├── adapters/
  │   ├── primary/     suggest
  │   └── secondary/   suggest
  ├── dependencies/    suggest
  └── deployment/      observe
  ```
- [ ] Tree uses color: green (act/silent), yellow (suggest), red (observe)
- [ ] `hex trust elevate alpha/adapters/secondary act` completes in <2 seconds
- [ ] Subsequent `hex trust show alpha` reflects the change
- [ ] Evaluator understands what each trust level means without reading docs

**Fail if**: Trust shows raw database rows. No color coding. Scope paths don't match hex's architecture layers. Elevate requires a scope string the evaluator can't derive from `show` output.

### Act 6: Walk Away

**Action**: Evaluator closes terminal and leaves for 2 hours.

**Pass criteria**:
- [ ] Beta's auto-resolution daemon resolves any future decisions at deadline
- [ ] Briefing buffer accumulates events while evaluator is away
- [ ] When evaluator returns, `hex brief` shows what happened in the last 2 hours
- [ ] No work was blocked waiting for the evaluator (all decisions have defaults)

**Fail if**: Any project stopped working because the evaluator wasn't present. Briefing shows no history of what happened while away.

### Act 7: Investigate (when needed)

**Action**: During the day, Alpha hits an architecture violation. Pulse changes to `◉`.

**Pass criteria**:
- [ ] Pulse shows `◉` for Alpha within 10 seconds of the violation
- [ ] `hex brief --project alpha` explains the block in plain language: what broke, why, what the options are
- [ ] `hex console alpha` opens the dashboard in the browser with context pre-loaded
- [ ] Console shows: the violation (which file, which boundary), the agent that caused it, the agent's reasoning
- [ ] Console offers actionable options (steer, override, pause)
- [ ] After resolution, Pulse returns to `●`

**Fail if**: Evaluator must decode the violation from raw hex analyze output. Console opens to a generic dashboard with no context. No clear path from "I see a problem" to "I fixed it."

### Timing Gate

**The entire sequence (Acts 1-5) must complete in under 5 minutes.** This includes reading the briefing, making 2 decisions, steering once, and checking/adjusting trust. If it takes longer, the UX is too complex.

A timer starts when the evaluator opens the terminal (Act 1) and stops when they've completed Act 5. Acts 6-7 are not timed (they test async behavior and edge cases).

---

## Property Tests (Automated)

These run as part of CI and verify invariants that the scenario test can't cover exhaustively.

### P1: Decisions Never Block Indefinitely

```
GIVEN a developer_inbox entry with deadline_at = now - 1 minute
AND the entry is not resolved
WHEN the expire_decisions() daemon runs
THEN the entry is marked auto_resolved = true
AND resolved_action = default_action
AND resolved_by = "auto"
AND a briefing_buffer entry with severity "notable" is created
```

### P2: Trust Decay Is Scoped

```
GIVEN delegation_trust for alpha/adapters/secondary/stripe at level "act"
AND delegation_trust for alpha/adapters/secondary/postgres at level "act"
WHEN a test regression is attributed (via git blame) to the stripe adapter agent
THEN alpha/adapters/secondary/stripe decays to "suggest"
AND alpha/adapters/secondary/postgres remains at "act"
AND alpha/domain/ remains unchanged
```

### P3: Trust Floor on Destructive Operations

```
GIVEN delegation_trust for alpha/deployment at level "silent"
WHEN any deployment action is attempted
THEN the system enforces a floor of "act" (notify in briefing)
AND the "silent" level is downgraded to "act" with a warning
```

### P4: Briefing Completeness

```
GIVEN 3 projects with mixed states (active, decisions pending, idle)
WHEN hex brief is called
THEN the output contains exactly 3 project sections
AND each section contains: status, phase, agent count or "idle"
AND projects with pending decisions show the decision count
AND the total output is ≤60 lines
```

### P5: Pulse Consistency

```
GIVEN project state changes (task completes, decision surfaces, agent dies)
WHEN /api/pulse is called
THEN the returned state matches the derived state from SpacetimeDB:
  - blocked: any unresolved critical developer_inbox entry
  - decision: any unresolved developer_inbox entry (non-critical)
  - active: any in_progress swarm_task AND no pending decisions
  - complete: all swarm_tasks done
  - idle: no swarm_tasks exist
AND state transitions are monotonic (no flickering between states within 10s)
```

### P6: Steer Idempotency

```
GIVEN a steer directive "prioritize the API adapter"
WHEN the same directive is sent twice
THEN the workplan task order is identical after both calls
AND only one steer_directive entry is created (or the second is marked duplicate)
AND the briefing_buffer contains one reordering event, not two
```

### P7: Decision Commands in Briefing Are Valid

```
GIVEN a developer_inbox entry with decision_id = 42
WHEN hex brief renders this decision
THEN the output contains "hex decide <project> 42 approve"
AND the output contains "hex decide <project> 42 override"
AND executing the approve command resolves the decision
AND executing the override command with a value resolves with the override
```

### P8: Cross-Project Pulse

```
GIVEN 5 projects in varying states
WHEN the Pulse statusline renders
THEN all 5 projects are represented (or 4 + "...+1 more")
AND the total width is ≤80 characters
AND each project shows the correct symbol for its state
AND the render completes in <500ms
```

---

## Non-Functional Requirements

| Metric | Target | How to measure |
|---|---|---|
| Pulse render latency | < 500ms | Time from terminal open to statusline visible |
| `hex brief` latency | < 3 seconds | Time from command to full output |
| `hex decide` latency | < 2 seconds | Time from command to confirmation |
| `hex steer` latency | < 3 seconds | Time from command to confirmation |
| Briefing length | ≤ 60 lines for 5 projects | `hex brief \| wc -l` |
| Decision throughput | 10 decisions/minute | Scripted approval test |
| Auto-resolution reliability | 100% of expired decisions resolved | Count unresolved past-deadline entries |
| Pulse accuracy | State matches SpacetimeDB within 10 seconds | Compare /api/pulse with direct table query |

---

## What Is Explicitly NOT Tested in Phase 1

These are deferred to Phase 2+ and should not gate Phase 1 acceptance:

- Taste Graph learning (Phase 2)
- Architecture Pressure Map visualization (Phase 2)
- Token Market / inference economy (Phase 2)
- Trust decay from test regressions (Phase 2 — basic trust set/elevate/reduce only in P1)
- Cross-project trust inheritance (Phase 2)
- Console investigation panels beyond basic context loading (Phase 2)
- Counterfactual branching (Phase 3)
- Codebase Instincts (Phase 3)
- Mobile/Slack/email notification channels (Phase 3)
- Multi-developer trust negotiation (Phase 3)
- Narrative briefing via inference (Phase 3 — template-based in P1)
