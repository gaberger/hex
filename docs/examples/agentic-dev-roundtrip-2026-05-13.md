# Agentic-Dev Round-Trip — 2026-05-13 trajectory

This is a captured end-to-end run of the hex autonomous loop on
2026-05-13. The operator sent a single board message; the system
ran the SOP pipeline against the message; two complete trajectories
landed, each exercising a different failure-mode of the loop's
safety nets. The point of capturing it is to show **both what
agentic development looks like when it works** and **how the
system refuses to ship bad output** — not to claim happy-path.

## The ask

```bash
curl -s -X POST http://127.0.0.1:5555/api/org/send-message \
  -H 'Content-Type: application/json' \
  -d '{
    "from": "operator",
    "to": "cto",
    "content": "@cto please draft docs/specs/agentic-dev-roundtrip.md — a developer-facing overview (~300 words) of how an operator drives the autonomous loop: board ask → persona claim → SOP GROUND/REASON/ACT/EXECUTE → drafter typed-tool emit → digital-twin approval → executor file write → commit. Include a concrete board-message example and what to watch on the dashboard. Use spec_draft."
  }'
```

The same ask was sent twice, against two different configurations
of the drafter, with the stub from the first run removed in between.

## Trajectory A — drafter model too small (commit `f336930a`)

Defaults at the time: `pick_drafter_model` returned `nemotron-mini`
(2.5 GB) for every path, long-form or not. `docs/specs/*` paths
require ≥ 800 bytes per `min_content_bytes_for_path` in
`hex-nexus/src/orchestration/drafter.rs:790`.

Watch log:

| Local time | Event |
|---|---|
| `20:37:51` | board ask routed → `cto` |
| `20:38:01` | `cto` reply landed: *"Confirm: I (cto) will draft docs/specs/agentic-dev-roundtrip.md"* |
| `20:38:01` | commitment `#16415` opened with `success_artifact = docs/specs/agentic-dev-roundtrip.md` |
| `20:38:01` | proposed_action `#33071` (file_write) → twin verdict `escalated` |
| `20:38:37` → `20:40:03` | proposed_action `#33081`, `#33082`, `#33083` — all twin-escalated |
| `20:40:59` | drafter circuit-breaker fired: stub written to disk, commitment `#16415` marked `abandoned` |

**Outcome — file landed but as a STUB.** Top of the file:

```
# docs/specs/agentic-dev-roundtrip.md — STUB (operator triage required)
Status: stub — auto-generated after 2 drafter attempts
Committed by: cto
Why this is a stub: persona returned INSUFFICIENT_CONTEXT … or content
was too short for the long-form artifact type.
```

The drafter recognized that nemotron-mini couldn't reliably hit the
800-byte spec threshold and *intentionally* abandoned the commitment
rather than write garbage. The stub is a triage signal, not a
deliverable.

### Fix

Patched `pick_drafter_model` to pin long-form paths
(`docs/adrs/**`, `docs/specs/**`, `docs/analysis/**`) to
`qwen2.5-coder:14b` by default (the current T2/T2.5 bench winner —
see memory `project_t2_5_bench_results`). Short-form (commit-mode
replies, code patches) still defaults to `nemotron-mini`. The
existing `HEX_DRAFTER_MODEL_LONGFORM` env override is preserved.

## Trajectory B — drafter content ungrounded (commit `<this PR>`)

Same ask, against the patched drafter. The first stub was deleted so
we could observe the new path cleanly.

Watch log:

| Local time | Event |
|---|---|
| `20:45:14` | board ask routed → `cto` |
| `20:45:14` | commitment `#16416` opened |
| `20:45:14` → `20:50:54` | proposed_actions `#33083`–`#33089` — **7 drafter attempts** |
| every attempt | twin verdict: `escalate`, reason: `no-grounding` |
| `20:50:54` | watch deadline (`+6m`), no file written, commitment still `open` |

Sample twin rationale (proposed_action `#33089`, 3116 bytes
generated):

```
content-grounding gate: persona 'cto' produced 3116 bytes to
'docs/specs/agentic-dev-roundtrip.md' with no repo paths, ADR IDs,
commit SHAs, or hex verbs.
```

The drafter generated substantive prose (`# Autonomous Development
Loop Overview\n\nThis document provides a developer-facing overview
of how an operator drives the autonomous loop within our system…`)
but cited nothing concrete. The twin's grounding gate (see
`feedback_no_persona_fabrication` memory + the
`orchestration::repo_grounding` system-prompt patch) caught the
hallucination risk and refused to write — for **seven** attempts in a
row.

**Outcome — no file landed, no stub written, commitment open.**

This is the system's *anti-fabrication* safety net working. The
operator gets a visible `pending_decisions.commitments[]` entry on
Mission Control instead of a polished-but-fictional spec on disk.

## What an operator should take away

1. **Pick the model for the path.** Long-form needs ≥14B; the
   2026-05-13 patch makes that the default. The override knob is
   `HEX_DRAFTER_MODEL_LONGFORM`. Set it if your hardware can't
   run `qwen2.5-coder:14b`.

2. **Ask in citations, not concepts.** The grounding gate measures
   anchor density (repo paths, ADR IDs, commit SHAs, hex verbs).
   A prompt that names the files you want discussed —
   *"reference `hex-nexus/src/orchestration/drafter.rs:790` and
   `hex-cli/src/commands/sched/improver/thought_patterns.rs`"* —
   gives the persona the anchors and lets the gate pass.

3. **Watch Mission Control, not the CLI.**

   * Pulse strip top-left: last commit + age tells you the executor
     is alive.
   * "Active thought patterns": if a commitment is sitting open for
     minutes, this is where you'll see what the system thinks is
     unresolved.
   * "Live events": `improver_tick` every 30s = sched daemon
     healthy. `loop_notification` = severity-error finding pushed
     to inbox.

4. **An open commitment without a written file is the system
   asking for help.** Don't manually write the artifact and clear
   the commitment — instead re-ask the persona with the missing
   context, or `commitment_abandon` the row if the ask was wrong.

## Reproducing this run

```bash
# 1. ensure nexus + sched daemon + STDB are up
hex nexus start
hex sched daemon --interval 30 --max-failures 3 &

# 2. fire the ask
curl -s -X POST http://127.0.0.1:5555/api/org/send-message \
  -H 'Content-Type: application/json' \
  -d '{"from":"operator","to":"cto","content":"@cto please draft …"}'

# 3. watch Mission Control http://<host>:5555/dashboard
#    or poll /api/mission-control for cto's commitments + proposed_actions
```

## Pointers

* Drafter model selection: `hex-nexus/src/orchestration/drafter.rs:766`
  (`pick_drafter_model`).
* Content-grounding gate (twin): the verdict surfaces in
  `proposed_action.twin_rationale`; the rule is enforced by the
  twin's evaluator before the executor sees the action.
* Stub circuit-breaker: `hex-nexus/src/orchestration/drafter.rs`
  emits the structured stub format documented in the example file.
* Commitment lifecycle: `hexflo-coordination::commitment` —
  `open` → `satisfied` | `abandoned` | `overdue`.
