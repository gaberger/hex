# Operator-Acceptance SLA

**Status:** Proposed
**Date:** 2026-05-11
**Author:** operator + session 2026-05-11
**Supersedes:** nothing — this is the missing meta-spec the AIOS should have had from day one.

---

## Why this spec exists

Drift diagnosis 2026-05-11. The operator asked the exec team for a status report. The system answered: 6 execs produced replies in STDB. The operator could not see them. Mission Control rendered the SOP routing prefix as the message body and hid the actual content. The fix was ~50 LOC and shallow; the *defect* was that nobody was measuring whether the operator was being served.

Hex has been built **inside-out**: substrate first, then SOP, then twin auto-approve, then supervisors, then RL model selection. Each layer is sound. None of them point at the operator. This spec inverts the polarity: nothing new ships until the operator is demonstrably served.

---

## The single metric

> **Time from operator ask → operator sees a rendered, useful answer in Mission Control.**

| Statistic | Target |
|---|---|
| p50 (board broadcast → first useful answer rendered) | ≤ 15 s |
| p95 | ≤ 60 s |
| silent rate (asks with zero operator-visible answer within 5 min) | < 5 % |
| useless rate (asks where every answer is an escalation stub or failure badge) | < 10 % |

"Useful" = persona-content present, ≥ 40 chars, status ≠ failed/escalated, **as rendered to the operator** (server-side reply existing does not count if MC hides it).

---

## Behavioral spec

```
GIVEN  operator sends a message via Mission Control or POST /api/org/send-message
WHEN   the SOP path routes it to one or more personas
THEN   - within p95 latency, the operator sees the persona body rendered in MC
       - the rendered body is the persona's content, NOT the SOP routing prefix
       - latency, persona, status, and prefix-vs-body byte counts are recorded in STDB

GIVEN  a persona reply is a no-content escalation stub
WHEN   rendered in MC
THEN   status badge = "⤴ escalated" and the escalation reason is shown inline

GIVEN  a persona reply is a failure (tool-cap, provider HTTP error, decode error)
WHEN   rendered in MC
THEN   - status badge = "✗ failed" with the failure reason inline
       - the system auto-retries via the Ollama-fallback path within 30 s
       - if both attempts fail, an operator inbox notification fires (priority 2)

GIVEN  the operator dashboard is bound to 127.0.0.1
WHEN   `hex nexus start` runs without an explicit `--local-only` flag
THEN   the daemon refuses to start; emits a clear error pointing at -b 0.0.0.0
```

---

## Instrumentation

| Component | Where | Field |
|---|---|---|
| `operator_ask` STDB table | hexflo-coordination | id, sent_at, content_hash, routed_to[], thread_id |
| `operator_answer` STDB table | hexflo-coordination | ask_id, persona, server_sent_at, client_rendered_at, status (ok/escalated/failed), body_chars, prefix_chars, reason |
| `record_answer_render` reducer | hexflo-coordination | client posts on first paint of a reply |
| `#/ops-sla` dashboard tile | hex-nexus/assets | today's ask count · p50 / p95 · silent rate · per-persona breakdown · last 50 worst-latency asks |
| daily roll-up | hex-nexus sched_service | 09:00 local → operator inbox notification |

---

## Known blockers (must be green before SLA numbers are credible)

| ID | Defect | Where | Owner (proposed) |
|---|---|---|---|
| B1 | MC renders raw SOP prefix as body — operator sees plumbing, not answer | `hex-nexus/assets/src/components/views/TeamDashboard.tsx` | hex-ux — **code-fixed locally 2026-05-11, awaiting rebuild + restart** |
| B2 | `chief-visionary` returns "Escalated: paradigm/strategy decision queued" to ≈ 90 % of asks regardless of content | persona dispatcher classifier | CTO |
| B3 | Tool round-trip cap (16) silent dead-end — persona burns 16 rounds then returns a status string with no body | SOP executor | CTO |
| B4 | OpenRouter HTTP 402 produces a "reasoning failed" stub indistinguishable from a normal reply; no Ollama fallover on the persona path | inference router | CTO |
| B5 | Nexus daemon may bind to 127.0.0.1 silently — remote operators see nothing | `hex nexus start` | COO |
| B6 | No operator project-board view — MC is chat-first, not plan/milestone-first; operator cannot see in-flight work rolled up against a plan | new dashboard view `#/missions` | CPO + hex-ux |
| B7 | No unstick automation — when a persona stalls / hits tool-cap / hits 402, operator has no system-side verb to "mark this complete and continue" or "re-assess from the top". Recovery is operator-improvised. | `hex steer` subcommands + supervisor reducer | CTO + COO |

Defect-fix PRs are unfrozen if and only if they close one of B1–B7.

---

## Reference: how others do this (2026-05-11)

Comparison against Factory.ai's `droid` CLI + Missions. Three patterns worth stealing:

1. **Plan-first, not chat-first.** Factory's Missions docs: *"The biggest value we have found in Missions is in the planning phase. Getting the upfront plan right is what determines whether the execution succeeds."* Their MC shows features × milestones × progress, not a chat. Implication for us: B6 above; demote the chat panel to secondary, promote a plan/milestone board to primary.
2. **Validator budget formula.** `total runs ≈ #features + 2 × #milestones`. Operator-visible cost estimate. Implication: each milestone in our `#/missions` view should carry a budgeted run count and an actual; over-budget surfaces as a badge.
3. **Verify-not-advocate framing.** Factory's `/verify` returns CONFIRMED / REFUTED / INCONCLUSIVE with anti-fabrication rules. Implication: `chief-visionary`'s "escalation queued" without body is fabrication-by-omission; B2 fix should adopt this contract — every persona reply has explicit verdict + evidence or it doesn't ship.
4. **Unstick playbook is a CLI verb, not a doc.** Factory documents recovery prompts ("The mission seems frozen — re-assess and continue"). We encode those as `hex steer` subcommands so the supervisor can run them autonomously (B7).
5. **Maturity self-grade.** Factory's `/readiness-report` grades a repo against a 5-level Autonomy Maturity Model. Add `hex readiness` that grades the *AIOS itself* — Level 1 = SLA tile renders, Level 5 = 7-day p95 ≤ 60s with zero operator manual interventions.

---

## `hex readiness` — proposed levels

| Level | Name | Pass criteria |
|---|---|---|
| L1 | **Renders** | `#/ops-sla` tile populated; operator can see SLA numbers at all |
| L2 | **Measured** | 7-day rolling p95 + silent rate are recorded in STDB and visible |
| L3 | **Self-healing on known failures** | B3 + B4 fail-overs auto-fire; tool-cap retry with different model; 402 → Ollama; both visible in MC as badges |
| L4 | **Unstuck without operator** | B7 supervisor verbs (`steer mark-complete`, `steer re-assess`) fire autonomously when stall conditions hit, with operator audit log |
| L5 | **Plan-tracked** | B6 milestone board live; every operator ask rolls up to a feature or milestone; readiness score regenerates daily |

Sequencing: L1 → L2 → L3 → L4 → L5. Each level is a freeze gate; cannot skip.

---

## Freeze condition

Until acceptance criteria are green for 7 rolling days, the following are frozen:

- No new persona types
- No new SOP tool primitives
- No new dashboard views beyond `#/ops-sla`
- No new supervisors (resource, persona, worker pool — all keep current scope)
- No new RL / neural-lab features

Exceptions require operator sign-off in this file (append below).

---

## Acceptance criteria

1. All five blockers (B1–B5) have closing commits, each referencing this spec.
2. `#/ops-sla` tile is live, populated from STDB, refreshed in real time.
3. 7-day rolling window shows **p95 ≤ 60 s** and **silent rate < 5 %** on board-broadcast asks.
4. Operator confirms they can answer "how's the team doing" in < 30 s without leaving MC and without touching a CLI.

When all four hold, this spec moves to status **Accepted**, the freeze lifts, and the next mechanism wave can resume.

---

## What this is NOT

- Not a request to slow down. The substrate work that already landed (typed-tool SOP, twin auto-approve, resource supervisor, RL model pick) stays. They just stop accumulating until the operator-facing loop closes.
- Not a re-architecture. The hexagonal substrate is the right call. The defect is that none of the personas owned the operator's experience.
- Not a complaint. The system has been replying to the operator the whole time. The replies just weren't reaching the operator. That's a 50-LOC bug and a measurement gap, not a paradigm failure.

---

## Open questions

1. Who owns the `#/ops-sla` tile end-to-end? COO for instrumentation reads cleaner, but hex-ux has to land the dashboard component.
2. Should silent / failed asks auto-fan-out to a backup persona (e.g. CTO failure → engineering-lead retry) before hitting the operator inbox? Probably yes; defer the policy until B2–B4 data is in.
3. What's the right place to enforce the freeze — a hex CLI gate, a CI check, or trust-by-convention? CI gate preferred; CLI gate is cheap.

---

## Append-only changelog

- 2026-05-11 — drafted in response to MC silent-reply incident.
