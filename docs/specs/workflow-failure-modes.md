# Workflow Failure Modes — Why the SOP Pipeline Doesn't Close Commitments

**Status:** Diagnosis (operator-authored, 2026-05-17)
**Scope:** Operator → `hex ops send` → `org_responder` → classifier → commitment → `drafter` → `twin_reviewer` → `action_executor` → commit
**Evidence:** `/home/gary/.hex/nexus.log` (9-day span, 2026-05-08 → 2026-05-17)

## Headline Numbers

| Metric | Count | Note |
|---|---:|---|
| twin_reviewer verdicts | 1253 | over 9 days |
| `approve` verdicts | 429 (34%) | **all** are `auto-approved: SOP-emitted action` — the typed-tool path |
| `reject` verdicts | 700 (56%) | drafter / free-form path |
| `escalate` verdicts | 124 (10%) | mostly content-grounding gate |
| drafter queue events | 802 | across 47 distinct commitments |
| commitments **satisfied** | 57 of 75 (76%) | per STDB `commitment` table at 2026-05-17T20:50Z |
| commitments abandoned | 17 of 75 (23%) | per STDB; 1 still open |
| worst single loop | 323 retries | `commitment 12293` → `resilience_thought_experiments.md` |
| Silent verdicts | 15 | cto 7, eng-lead 4, ciso 2, cpo 1, cvo 1 |
| off-contract responder drops | 30 | persona produced reply ≠ `Confirm:`/`Silent` |
| content-grounding rejects | 101 | cto 94, ciso 7 |
| twin parse-failure-budget exhaustions | 23 | twin's own LLM produced no valid JSON in 5 tries |

**The headline is efficiency, not closure rate.** Direct STDB query at 2026-05-17T20:50Z showed 75 commitments — 57 satisfied, 17 abandoned, 1 open. So the floor *is* 76% satisfied. But the **cost** of getting there is the bug: the 6 worst-offender commitments accounted for ~75% of all drafter work (commitment 12293 alone retried 323 times against the same rejection). The system eventually closed most things, but at 5–300× the necessary LLM budget. The 429 `approve` verdicts are entirely typed-tool actions; persona-free-form actions reject at 100% on source paths and require many retries elsewhere before any random draft happens to ground.

(An earlier draft of this section claimed 0/47 closures based on a log-grep that missed the `commitment_satisfy` reducer's log format. The STDB query is authoritative; the satisfaction *floor* was higher than first reported, but the **structural problems are real** — efficiency loops, silent drops, stub-clobber, rejection-loops without back-pressure.)

## Pipeline Map + Lossy Edges

```
operator
  │  hex ops send / API POST /api/org/send-message
  ▼
org_responder (poll inbox every 4s)
  │  ──▶ classifier LLM (free-form persona prompt, strict Confirm:/Silent output contract)
  │     ✗ 30 off-contract drops (persona produced prose, not Confirm/Silent → dropped)
  │     ✗ 15 Silent verdicts (persona judged "not mine" but operator-direct never should be)
  │
  ▼  if Confirm:
commitment opens in STDB
  │
  ▼  drafter poll (every 30s)
drafter LLM (free-form, prompt: "produce the artifact at this path")
  │  ✗ no artifact-kind dispatch (.rs treated same as .md treated same as .toml)
  │  ✗ no shared schema with twin_reviewer (each crate has its own allowlist)
  │  ✗ proposed_by = persona name, not tool name → mismatch with twin's rule
  │
  ▼
proposed_action(file_write, role=<persona>, path=<artifact>)
  │
  ▼
twin_reviewer LLM (auto-approves tool:* actions, judges everything else)
  │  ✓ 429 auto-approve when proposed_by starts with "tool:" (correct path)
  │  ✗ 700 reject on persona-direct actions
  │     - 671 "path outside allowed prefixes" / "only code_patch may write source files"
  │     - 101 "content-grounding gate: no repo paths / commit SHAs / hex verbs"
  │     - 23 "twin parse-failure budget exhausted"
  │  ✗ NO BACK-PRESSURE: every reject increments a counter, but the next drafter
  │    attempt uses identical prompt, identical model, identical persona. No learning.
  │
  ▼  if approve:
action_executor consumes proposed_action
  │  → file_write to disk, optional inline cargo_check, commit step
  │
  ▼
commitment_satisfied  ← reaches here 0/47 times for persona-direct paths
```

## Root Causes (Ranked by Damage Caused)

### 1. Free-form drafter path is structurally incompatible with twin_reviewer
**Evidence:** 700 rejects in 9 days, 0 closures. `tool:code_patch` actions auto-approve (429/429); persona-direct actions reject 100% of the time on source paths.

The twin's rule is "only `tool:code_patch` may write source files" (twin_reviewer.rs:413). The drafter unconditionally emits `proposed_by=<persona-role>`. The drafter and the twin share no schema — they have parallel hardcoded allowlists that disagree. When the artifact path is in twin's deny set, the persona has no escape: the drafter cannot produce `tool:code_patch` invocations, only `file_write` payloads.

**Fix shape:** Either (a) the drafter dispatches to the right tool by artifact-kind (`.rs`/`.ts`/`.go` → `code_patch`, `.md`/`.json` → `file_write`), or (b) remove the free-form drafter entirely and require all persona commitments to be satisfied by tool invocations from the SOP loop. Two parallel commits today (`a66bb412`, `f02952e9`) added the source-path abstain — that stops the loop but doesn't solve the underlying gap.

### 2. No back-pressure or learning loop between twin and drafter
**Evidence:** Commitment 12293 retried **323 times** against the same path with the same rejection ("path outside allowed prefixes"). Commitment 12292 retried **256 times**. Neither escalates, neither switches strategy, neither stops.

The drafter's failure counter (`STUB_AFTER_FAILURES = 2`) is meant to be the back-stop, but it didn't fire because each twin rejection isn't being counted as a draft failure. The drafter sees "I queued an action" → success. The poller sees "commitment still open" → retry. No-one closes the loop.

**Fix shape:** Twin rejections must feed back into the same per-commitment failure counter the drafter uses for INSUFFICIENT_CONTEXT abstains. After N rejections, escalate (different model, different persona, or operator). Or: convert the counter to a token-budget — abandon when N tokens have been burned on the same commitment.

### 3. Binary Confirm/Silent contract loses real signal
**Evidence:** 30 off-contract drops (persona produced thoughtful prose, classifier discarded it) + 15 Silent verdicts (5 of them from operator-direct asks where Silent should be unreachable).

The classifier prompt (`org_responder.rs:817-830`) gives the persona two outputs: `Confirm: <thing>` or `Silent`. Real persona responses include: "this isn't mine, route to X", "I need <Y> first", "I can do this but only the first half", "this is impossible because <Z>". All of those get binned as off-contract and dropped, or coerced into Silent. The system has no place for legitimate rejection or clarification.

**Fix shape:** Expand the contract to `Confirm:` / `Defer: <reason>` / `Route: <other-persona>` / `Clarify: <question>` / `Reject: <reason>`. For operator-direct asks (`from=operator`), `Silent` is illegal — escalate to operator inbox instead.

### 4. Recovery primitives bypass safety primitives
**Evidence:** The stub-writer ran for 9 days writing markdown stubs that bypass twin review "because stubs are an operator-triage signal." On 2026-05-17 it cheerfully overwrote `hex-nexus/src/orchestration/drafter.rs` (the source file holding the abstain logic that had just been committed) with a markdown stub.

When the system has a "safe path" (twin review) and an "emergency path" (stub-writer), the emergency path needs to be **strictly safer** than the safe path — not less safe. The stub-writer had less protection than the thing it was rescuing.

**Fix shape:** Commit `f02952e9` added the source-path refusal to the stub-writer. The general principle needs to propagate: any pipeline that bypasses a safety gate must restate every guarantee the gate provided. Test: can the recovery path overwrite a source file? If yes, it's wrong.

### 5. Persona identity is conflated with tool authorization
**Evidence:** `proposed_by="cto"` rejected, `proposed_by="tool:code_patch"` approved — even though `cto` is the persona that *invoked* `code_patch`. The twin can't tell "CTO via code_patch tool" from "CTO via free-form drafter" because the proposed_by string is single-valued.

**Fix shape:** Split `proposed_by` into `(persona, tool)` — twin authorizes on tool, attribution on persona. Drafter emits `(cto, drafter:free-form)`; SOP loop emits `(cto, tool:code_patch)`. Twin rules become "tool=code_patch can write source" without losing persona attribution.

### 6. No idempotency on commit retries
**Evidence:** Commitment 24578 has 5 different proposed_actions for `docs/adrs/ADR-2026-05-12-structural-smell.md` in 2 minutes, each with different content (2946 bytes, 3099, 2606, 2735, …). Whichever wins, the other 4 drafts are silently lost. Personas have no way to see "my previous attempt was rejected for X, here's an improved version."

**Fix shape:** Drafter prompt should fetch the most recent rejected action for this commitment and include its content + rejection rationale. Then "produce a v6" is a real iteration instead of a blind reroll.

### 7. Operator-passthrough is the universal escape hatch — and is invisible
**Evidence:** Most artifacts that actually landed on disk in the last 9 days were written via `hex ops write` (operator-passthrough). The SOP path produced ~0 of them. The system *appears* autonomous because the dashboard shows traffic; in fact the operator is the load-bearing component.

**Fix shape:** Surface "% of artifacts landed via SOP vs operator-passthrough" on the dashboard. If the ratio inverts (operator > SOP), that's a system-health alarm.

### 8. Stuck commitments aren't surfaced
**Evidence:** Commitment 12293 looped 323 times over days. No alert, no operator notification, no inbox entry. Discovered only by reading the log.

**Fix shape:** Per-commitment retry-budget alarm. When retries > N or token-burn > $X, fire `hex inbox notify` to operator. Currently the cost watchdog exists but doesn't tie to per-commitment runaway.

## What's Already Shipped Today

- `a66bb412` — drafter abstains on source-file paths instead of looping (root cause #1, partial)
- `f02952e9` — stub-writer refuses source-file paths instead of clobbering (root cause #4)
- 5 commitments abandoned manually to stop the bleed

These are tactical patches. The structural gaps (#2 back-pressure, #3 classifier contract, #5 persona-vs-tool split, #6 iteration-aware drafts, #7 SOP-vs-operator metric, #8 stuck-commitment alarms) remain open.

## Next Workplan Candidates

1. **Twin → drafter back-pressure** — single highest ROI; would eliminate the 323-retry loops alone. Wire reject events into the same per-commitment counter that triggers stub/abandon.
2. **Classifier contract expansion** — `Defer/Route/Clarify/Reject` verbs; bans `Silent` for `from=operator`.
3. **Persona+tool split in proposed_by** — schema change in STDB `proposed_action` row + twin_reviewer rule rewrite; unlocks free-form drafter to safely produce source-file changes via persona-invoked tools.
4. **Iteration-aware drafter prompts** — feed prior-rejection content + rationale into the next attempt.
5. **Dashboard metric: SOP-vs-operator artifact share** + per-commitment retry alarm.
