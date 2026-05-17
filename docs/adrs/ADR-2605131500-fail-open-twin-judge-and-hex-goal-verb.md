# ADR-2605131500: Fail-open twin judge + `hex goal` verb (Hermes /goal pattern)

**Status:** Accepted (instance shipped; commit pending)
**Date:** 2026-05-13
**Authors:** Operator (direct authorship — SOP path failed twice to produce a usable draft; the persona that claimed the work hallucinated OWASP-DRP platitudes instead of the requested content, see Provenance section)
**Supersedes:** Partially supersedes the substring/evidence-strict reconciler logic in `hex-cli/src/commands/sched/` (separate companion change tracked below)
**References:**
- [Hermes Agent — Persistent Goals (`/goal`)](https://hermes-agent.nousresearch.com/docs/user-guide/features/goals)
- [Codex CLI 0.128.0 `/goal`](https://github.com/openai/codex) (Eric Traut, OpenAI — the Ralph-loop pattern origin)
- ADR-2605082300 (digital-twin reviewer — what we're patching)
- ADR-2605082500 (typed-tool SOP — the `tool:*` auto-approve fast path)
- Memory: `project_audit_autonomous_dev_2026_05_12` (70% reconciler FP rate measurement)

## Context

Two failure modes were burning ~100% of the SOP path's value on 2026-05-13:

### Failure 1 — Twin judge wedges on prose

`hex-nexus/src/orchestration/twin_reviewer.rs::run_one` polls `proposed_action.pending`, asks `qwen3:4b` to return a strict JSON verdict, and on parse failure logs a warning and leaves the action `pending`. There is no attempt counter and no budget. The next 20-second tick retries the same action. The 4B model reliably emits prose preamble ("We are given a proposed action...") instead of JSON, so the loop retries indefinitely.

Measured cost on a 73-minute nexus-uptime window: **3,734 retries on a single action #33057**, ~4,316 total `no JSON` parse failures across the queue, representing ~99% of the twin's inference budget burned on actions that will never get a verdict. Two stuck actions held the queue head-of-line; downstream work serialized behind them.

This is the same anti-pattern Hermes Agent identifies in their `/goal` design doc: *"a broken judge never wedges progress."* Their answer is fail-open semantics — judge errors are treated as `continue`; a turn budget is the only real backstop.

### Failure 2 — sched reconciler false-positives flip real work to `failed`

Separately, the scheduler's reconciler uses substring matching against expected-evidence patterns to decide whether a workplan task's commit satisfies its acceptance criteria. The autonomy audit on 2026-05-12 measured **~70% false-positive rate**: real work that committed correctly but didn't include the literal expected string was flipped to `failed` and re-enqueued, while bundled commits and re-stagings that *did* contain the substring were marked `done` even when the work was incomplete. Commit `4bb427ad` swapped to an evidence-strict gate but the failure mode is structural — Boolean string matching cannot judge work quality. The right shape is also an LLM judge, with the same fail-open posture as Failure 1.

## Decision

Adopt the **Hermes /goal pattern** uniformly across hex's two judge surfaces:

### 1. Twin reviewer (`hex-nexus/src/orchestration/twin_reviewer.rs`) — SHIPPED

- Introduce `MAX_PARSE_FAILURES: u32 = 5` constant.
- Maintain an in-memory `HashMap<u64, u32>` of action_id → consecutive parse-failure count, owned by the spawned task closure (resets on restart by design — stale wedges from prior runs are already non-pending if they ever escalated).
- After **5** consecutive parse failures on the same action_id, the twin calls `decide(..., "escalate", "<rationale citing budget exhaustion>")` instead of attempting another inference call. The action moves out of `pending` (status = `escalated`) and surfaces to the operator inbox.
- **Only parse-failure error variants count toward the budget** (`no JSON`, `json parse`, `missing verdict`, `invalid verdict`). HTTP / inference transport errors do NOT count — those are transient and retry is correct.
- Counters for actions no longer pending are GC'd at the top of each tick (`retain(|k, _| still_pending.contains(k))`).

### 2. Sched reconciler (`hex-cli/src/commands/sched/`) — TO DO (companion workplan)

- Replace the substring/evidence-strict gate with an LLM-driven verdict over `{commit_diff, workplan_evidence_block}`.
- Same fail-open posture: judge error → treat as `continue` (do not flip the workplan task to `failed`).
- New auxiliary slot `auxiliary.reconciler_judge` in config, defaulting to a T1 local model (`qwen3:4b` or `gemma4:latest` per `project_t2_5_bench_results`).
- Turn budget = `sched.max_judge_turns` (default 20). When exhausted, escalate to operator inbox instead of auto-failing the task.

### 3. New CLI verb: `hex goal "<intent>"` — TO DO (companion workplan)

Sits between `hex chat` (single turn) and `hex plan` (decomposed workplan):

- Persists a standing goal in STDB (`session_goal` table, keyed by session_id).
- After each chat/SOP turn, an aux-model judge returns `{done: bool, reason: string}` over the goal text + last assistant response.
- Fail-open on judge error. Turn budget = 20 continuations (configurable). When budget hits, auto-pause and surface to operator.
- Commands: `hex goal "<text>"` (set), `hex goal status`, `hex goal pause|resume|clear`.
- This is the user-facing surface of the same primitive that fixes the reconciler — one judge implementation, two entry points.

### Decision rules — fail-open invariant

All three surfaces share the same contract: **a judge MUST NOT be able to mark work as failed**. Only the operator (via `escalate` → manual review) or the turn budget (via `pause/escalate`) can terminate a goal/action with a negative outcome. Judge errors of any flavor degrade to "continue / try again," not "fail."

## Consequences

### Positive

- **Wedge avoidance**: empirically validated on the 2026-05-13 dataset. Pre-patch: action #33057 burned 3,734 inference calls in 73 minutes. Post-patch: same action escalated after 5 attempts (~100s) and the queue drained to zero pending. **~747× inference-waste reduction** on this single pattern; ~99.7% across the wedge-prone window.
- **Operator gets a signal instead of silence**: stuck judges now surface as `escalated` rows with rationale, queryable from STDB and visible on the dashboard. Today they were invisible unless the operator scraped `nexus.log` directly.
- **Single primitive, three callsites**: the twin patch, the reconciler rewrite, and `hex goal` all share the same judge-loop shape. Easier to maintain, test, and tune (one aux-model slot to pin).
- **Restart resilience without persistent state**: in-memory counters reset on nexus restart, which is correct — any action that previously escalated is now non-pending and won't be re-tried; any action still pending earns a fresh budget.

### Negative / risks

- **The escalation route depends on operator attentiveness.** If the operator ignores `escalated` rows, work stalls indefinitely. Mitigation: dashboard counter + inbox notification on every escalation (separate small change).
- **Wall-clock latency floor**: an action that genuinely should have approved on attempt #2 (say, a transient OpenRouter content filter) now waits up to 5 ticks × 20s = 100s before that decision. Acceptable — the alternative was infinite waiting.
- **The persona-output quality problem is NOT solved by this ADR.** When the SOP path produces hallucinated content (e.g. CISO's first attempt at this ADR), the twin will dutifully evaluate the hallucinated content against operator memory and either reject (if the path is wrong) or — worse — approve (if the path passes pattern checks but the *content* is junk). Twin's content-vs-ask check is best-effort. Tracked as a separate ADR.
- **In-memory counter is per-process.** If the twin task panics and restarts within the same nexus process (currently not possible — the loop has no internal panic handler), the counter resets. Acceptable: panic-restart of a tokio task should be rare and the worst case is one extra 5-retry round before escalation.

### Neutral

- Filename-substitution bug observed in this session (CISO drafter wrote literal `<turn>` in the path) is unrelated to judge logic. Tracked separately.

## Alternatives Considered

1. **Keep substring reconciler + status-quo twin.** Rejected — 70% FP on reconciler, ∞ retries on twin. Empirically broken; sessions were burning hours of inference on wedged judges.
2. **Boolean LLM judge (no fail-open).** Rejected — same wedge mode in a new wrapper. A model that occasionally returns prose will still occasionally return prose; without a budget the loop wedges.
3. **Persistent retry counter in STDB on `proposed_action`.** Rejected for V1 — needs schema migration and doesn't materially change behavior (the in-memory counter handles within-run wedges, which is the actual failure mode; cross-restart wedges are a non-issue because escalated actions are no longer pending). Revisit if cross-restart counter resilience becomes load-bearing.
4. **Bigger twin model (e.g. claude-haiku, opus, devstral).** Useful but orthogonal. Larger models reduce parse-failure rate but don't eliminate it (any LLM can emit prose on a bad day). Budget+fail-open is the structural fix; model upgrade is a parameter tweak. Both should happen.

## Implementation Notes

### Already shipped in this session (twin reviewer)

- `hex-nexus/src/orchestration/twin_reviewer.rs`: imports, constants, run_one signature, budget check, error-class match, GC of completed counters.
- `cargo check -p hex-nexus`: passes.
- `cargo build --release -p hex-nexus`: passes (2m 54s).
- Binary installed at `~/.local/bin/hex-nexus` (note: `cargo build` writes to `target/x86_64-unknown-linux-gnu/release/`, NOT the workspace `target/release/` — `hex nexus start` reads `~/.local/bin/hex-nexus`, so an install-copy step is required after every cargo build).
- Validation: action #33057 escalated at 19:03:11Z after exactly 5 attempts; STDB pending count went 2 → 0.

### To do (companion workplans, separate ADRs / patches)

1. Sched reconciler rewrite with aux-model judge + `auxiliary.reconciler_judge` config slot.
2. `hex goal` CLI verb with STDB-persisted standing goals, judge after each turn, pause/resume/clear/status subcommands.
3. Dashboard surface for `escalated` rows (one-click "approve/reject" from the UI).
4. Inbox notification hook on every escalation so operators don't need to poll the dashboard.

### Migration Plan

- **Twin patch**: no migration; counters are in-memory and start empty. Existing `pending` rows continue to be reviewed; ones that have been retrying forever will now escalate within 100s of nexus restart.
- **Sched reconciler**: behind a feature flag `sched.judge_mode: substring | aux_model` (default `substring` until the new judge is validated on a shadow workplan run; flip to `aux_model` after a week of dual-run telemetry).
- **`hex goal`**: greenfield verb, no migration.

## Provenance (why this ADR is operator-authored, not SOP-emitted)

This ADR was originally routed through SOP via `/api/org/send-message` to CTO at 2026-05-13 17:51:44Z (message_id `db1dd318`). The thread atomic-claim landed on the CISO persona instead. CISO's drafter emitted a 2,304-byte file at `docs/adrs/ADR-260512-<turn>-fail-open-goal-judge.md` (note the unresolved `<turn>` placeholder — a separate drafter bug). The file's content was wholly hallucinated: it described "an OWASP Dependency Ranking Project goal judge" dated 2023-04-28, with no reference to the fail-open semantics, the Hermes pattern, the twin wedge data, or any of the eight numbered scope points the operator supplied. Deleted at 14:59 local.

Per memory `feedback_homeostasis.md` and `feedback_no_asking_for_permission.md`: the SOP failed to produce a usable artifact on a domain it should own; operator-authoring is the correct fallback while the SOP-content-quality and drafter-filename-substitution bugs are tracked as separate work.
