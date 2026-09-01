---
name: hex-close-loop
description: Close the SDLC loop — a deterministic trigger invokes the agent with no person in the path, and what it finds re-enters the pipeline as a new intent.md. Use when the user says "close the loop", "monitor production", "run this on a schedule", "autonomous", "control bands", "alert to fix", "recurring scan", or asks how maintenance feeds back into planning.
---

# Hex Close Loop — Stage 6, Maintain

**What changes:** maintenance stops being reactive. A trigger — a breached
control band, a ticket, a channel message, a schedule — invokes the agent with
nobody in the invocation path. It diagnoses, acts only through gated routes, and
writes what it finds as an intent file, which goes through Stages 1–5 like any
other change. People triage and review that work; they no longer have to start it.

Traditional: an alert fires at 3 a.m. and can be missed, a ticket sits in the
backlog until someone picks it up, and post-mortem actions may never reach the
codebase because another fire started first.

## Prerequisites

This play comes last. It needs the intent format (Stage 1) to have something
structured to restart the loop with, review passes and approval-gate hooks
(Stage 5) so its output cannot bypass a human, and a rehearsed rollback path,
because the highest autonomy tier invokes it.

## How to execute it

1. **Pick one metric with a stable rolling baseline.** CI test failure rate,
   post-deploy 5xx rate, PR cycle time. One, not ten.
2. **Write the detection script.** Mean and standard deviation over a rolling
   window, with rules (Western Electric or similar) so the bands catch slow drift
   as well as spikes. Version-controlled and unit-tested. **Detection stays
   entirely deterministic — no model in the detection path.**
3. **Define response tiers in version-controlled config.** At 1σ log only. At 2σ
   invoke the agent read-only to diagnose. At 3σ let it act, but only by opening
   a PR into the review gate or triggering a pre-approved runbook.
4. **Pick the trigger layer:** a scheduled CI workflow, a webhook from the
   existing monitoring stack, or a cron job inside the network. The agent runs
   stateless and non-interactive, so a loop can begin and end without anyone
   starting it.
5. **The agent writes its diagnosis as an intent file in the Stage 1 format** —
   the anomaly and its evidence, a proposed outcome, affected systems, open
   questions. From there it goes through the pipeline like anything else.
6. **Someone triages the queue.** Fix now, schedule, or dismiss. Dismissals tune
   the bands and cut the noise; a dismissal without a reason teaches nothing.
7. **When a fix ships, add an eval for the incident** (`hex-feedback-loop`) so
   that class of failure is regression-tested from then on.

```yaml
metric: ci_test_failure_rate
baseline: rolling_30d
rules: western_electric
tiers:
  1sigma: { action: log }
  2sigma: { action: diagnose, tools: "Read,Grep,Bash(gh run view *)" }
  3sigma: { action: propose, routes: [pull_request, runbook:rollback-deploy] }
```

Worked examples:

- CI test failure rate breaches 3σ → quarantine the flaky test or open a revert
  PR; the review gate decides.
- Post-deploy 5xx rate breaches 3σ with a deployment in the window → trigger the
  existing rollback pipeline.
- PR cycle time trips a drift rule → write a report for engineering leadership.
  The harness works for process metrics, not just production ones.

## Recurring scans

A security scan is a point-in-time statement about a codebase under a particular
model, and both halves go stale: the code changes every week, and each model
generation finds what the previous one missed. So run scans on a schedule with no
human in the invocation path, and send findings through the same gates as any
other change.

- Treat the first scan of a repository as the baseline — expect findings in code
  that was considered clean.
- Weekly is a sensible default for actively developed services.
- Triage with the confidence rating in hand, and dismiss with a reason so the
  same finding does not return as new.
- A finding that fits in one PR goes through the review gate. Anything wider — an
  architectural weakness, a pattern repeated across services — becomes an intent
  file and starts at Stage 1.
- Model-driven scans augment the deterministic checks; they do not replace the
  static analysis and dependency scanning already in CI.

## Work arriving through a channel

Incidents also arrive as a 10 p.m. message in an incident channel. When the agent
is a member of that channel under its own identity, each incident gets a first
responder and the response becomes part of the record: hypotheses tested in
thread, the metric confirmed back at baseline, and the post-mortem written to a
version-controlled lessons file that future investigations read. The channel is
the audit trail — request, diagnosis, human authorization and fix all stay where
the incident was handled.

In a hex project, that lessons file has a queryable equivalent:
`hex memory store lesson:<topic> "<text>"`, which any later session can search.
Notifications arrive through `hex inbox`, and a priority-2 notification preempts
current work.

## Confidence gates between stages

Running headless means each stage needs an independent gate deciding whether the
previous stage's output continues or escalates to a human — a deterministic check
where one exists, an adversarial reviewing agent where one does not. Without that
gate, an autonomous loop compounds its own mistakes at machine speed.

## Governance

Tier boundaries are enforced from version-controlled config, with permissions and
managed settings denying production access outright. Invocations, findings and
triage decisions are logged with timestamps. A service owner triages and
approves; resulting changes go through the normal PR review gate; the runbooks
the agent may trigger were approved in advance.

## Measure it

- **Leading:** time from band breach to an intent file in the triage queue,
  against the old time from incident to post-mortem action; share of connected
  repositories on a scan schedule.
- **Lagging:** share of findings that become merged fixes; repeat incidents of the
  same class, which should fall as fixes add cases to the eval suite; issues found
  by the scheduled scan versus those found in production.

## Next

Stage 1. The loop keeps running; human judgment stays above it.
