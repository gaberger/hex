# ADR-2606071651: ReAct edit-loop progress guard — edit-nudge + single-shot fallback

**Status:** Proposed
**Date:** 2026-06-07
**Epoch:** single-agent
**Drivers:** A live `hex do` run on a local 14B model (`qwen2.5-coder:14b`) wandered the
ReAct edit-loop for the full step budget calling only read/verify tools, never emitted
the terminal `propose_edit`, and exited with `loop ended with no edit` — shipping nothing.
The identical task on the `--fast` single-shot path passed the evidence gate on the first
attempt. The loop's *reasoning* surface is the bottleneck, not its tools or its gate.
**Supersedes:**
**Superseded-By:**

## Context

The canonical execution path (`hex do`, ADR-2026-06-04-1740 Path A) is the evidence-gated
ReAct loop in `hex-exec/src/direct_react.rs`: reason → call read/verify tools → eventually
call the terminal `propose_edit`, which applies the edit, runs the evidence command, and
commits iff it exits 0. The loop already has three guards: `max_steps` (clamped 1..40),
duplicate-call suppression, and a `NO_PROGRESS_LIMIT` churn breaker.

None of those guards address the dominant observed failure mode on smaller local models:
the model gathers context indefinitely — `repo_read`, `repo_grep`, `cargo_check` — and
**never commits to a `propose_edit`**. It either exhausts `max_steps` or emits a bare
text turn (no tool call), and `direct_react.rs:172-181` terminates with
`loop ended with no edit`. The edit was never even *attempted*, so the evidence gate —
the part that actually works — never engages.

This is the same conclusion the repo reached repeatedly under the retired org-sim epoch
(the ebay-clone runs, the 2026-06-04 factory live-test): *the substrate and the gate are
sound; the doer-loop execution is unreliable.* The difference now is that the single-agent
epoch makes the failure isolable and cheaply fixable — there is exactly one loop to harden.

Two facts make the fix tractable:

1. **The failure is detectable from inside the loop.** "K steps elapsed with zero
   `propose_edit` calls" is a trivial counter; so is "the loop is about to return with
   `edit_applied == false`."
2. **A more constrained path already succeeds on the same input.** The single-shot path
   (`direct_exec` / `--fast`: read window → one edit → evidence, N attempts) committed the
   exact task the ReAct loop abandoned, on attempt 1. We do not need a better model — we
   need to *stop wandering* and, failing that, *route to the path that works.*

## Decision

Add a two-stage progress guard to the ReAct edit-loop, and an automatic fallback to the
single-shot path. No change to the evidence gate, the tool allowlist, or commit semantics —
a commit still requires `propose_edit` + evidence exit 0.

1. **Edit-nudge (in-loop steering).** Track `read_only_streak` — consecutive dispatched
   steps that produced no `propose_edit`. When it crosses `EDIT_NUDGE_AFTER` (default 4),
   inject a single high-salience `user`-role nudge into the transcript:
   *"You have gathered enough context. Call `propose_edit` now with your best edit; you can
   iterate after seeing the evidence result. Do not call another read tool."* The nudge
   fires at most once per run (it is steering, not nagging) and resets if an edit is
   attempted.

2. **Single-shot fallback (cross-path recovery).** If the ReAct loop returns without a
   committed edit (`!result.committed`) — whether by no-edit, max-steps, or no-progress —
   `hex do run` automatically re-runs the task once on the single-shot path before
   reporting failure, unless the caller passed `--no-fallback`. The fallback's outcome is
   authoritative; its provenance is recorded so `hex do runs` shows the path that landed it.

3. **Failure-reason telemetry (prerequisite for measurement).** The run record gains a
   coarse `failure_reason` enum — `no_edit` | `evidence_fail` | `max_steps` | `no_progress`
   | `inference_error` — so wander is distinguishable from a genuine evidence failure in
   `hex do runs` and in any downstream evaluation harness (e.g. the context-ablation
   harness, which cannot interpret pass-rate without knowing *why* a run failed).

`EDIT_NUDGE_AFTER` and the fallback toggle are overridable so the ablation harness can
disable both arms to measure the *raw* loop when that is the variable under test.

## Consequences

**Positive**
- The exact live failure that motivated this ADR becomes a self-healing success: nudge
  first, single-shot fallback second.
- The gate's authority is untouched — fallback edits pass the same evidence command, so
  this cannot launder a vacuous commit.
- `failure_reason` makes the loop *measurable*, which is a hard dependency of the
  context-ablation harness (the separate "prove the thesis" workstream).

**Negative / risks**
- A run that would have failed fast now also pays one single-shot attempt — bounded extra
  latency/tokens on the failure path only (success path is unchanged).
- The nudge is a prompt-level heuristic; on a model that genuinely needs more context it
  could push a premature edit. Mitigated: the edit still faces the evidence gate and the
  loop continues iterating after a failed `propose_edit`, so a premature edit costs one
  reverted attempt, not a bad commit.
- Two code paths (ReAct + single-shot) are now coupled at the orchestration layer; the
  fallback wiring must live in the `hex do run` command handler, not inside either path.

## Implementation

- `hex-exec/src/direct_react.rs` — add `read_only_streak`, the one-shot nudge injection,
  and populate `failure_reason` on each terminal/break path.
- `hex-exec/src/direct_exec.rs` (or the single-shot entry) — expose a callable the command
  handler can invoke as the fallback with the same task/evidence.
- `hex-cli/src/commands/` (the `hex do run` handler) — on a no-commit ReAct result, invoke
  the single-shot fallback unless `--no-fallback`; record which path committed.
- Run record / `hex do runs` — surface `failure_reason` and the landing path.
- Tests: unit-test the `read_only_streak` → nudge trigger; unit-test `failure_reason`
  classification for each break path; an integration smoke that a stubbed loop returning
  no edit triggers exactly one fallback.

Tracking workplan: `docs/workplans/drafts/` (created via `hex plan draft` alongside this ADR).

## References

- ADR-2026-06-04-1740 — direct executor (Path A), the loop this hardens.
- ADR-2606061359 — single-agent epoch (why there is one loop to fix).
- ADR-2606071340 — `hex-exec` crate extraction (where the loop lives).
- ADR-2026-05-19-0720 / ADR-2026-06-04-1740 — the evidence gate (unchanged by this ADR).
- Live evidence: `hex do runs` — ReAct run `loop ended with no edit · 18 steps`;
  single-shot run `evidence pass · 1 attempt · commit 49fc3b09`.
