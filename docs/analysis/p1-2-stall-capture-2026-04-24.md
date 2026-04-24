# P1-2: Instrumented workplan stall capture (2026-04-24)

Workplan: wp-fix-workplan-inference-stalling, task P1-2
Target: wp-bazzite-e2e-arch-validation.json, execution b026500d-70df-48ec-822f-c42489a3a1bb
Raw log: `docs/analysis/p1-2-debug-stall-2026-04-24.log.txt` (635 lines; `.txt` suffix because `.gitignore` excludes `*.log`)

## Command

```
hex plan execute --verbose docs/workplans/wp-bazzite-e2e-arch-validation.json 2>&1 | tee /tmp/debug-stall.log
```

Host: claude-bazzite.lan (run ON the target, not via ssh).
Start 15:25:17Z · end 15:35:17Z · exit 0.

## Where did output stop?

It didn't. The CLI streamed output steadily for the full 10 minutes:

| Sample | bytes added in prior 60s |
|---|---|
| 15:26:13Z | 6862 |
| 15:27:13Z → 15:35:13Z | 6030–6238 each minute (effectively constant) |

The hypothesis behind this workplan — "workplan processes hang silently during inference" — is **not** what happens from the CLI's point of view. The CLI exits on its own 600s global ceiling:

```
Workplan execution timed out after 600s
```

## What the CLI is actually doing for 10 min

Two interleaved loops, both healthy:

1. HTTP GET to `http://127.0.0.1:5555/` every ~2s (`reuse idle connection` / `pooling idle connection` pairs) — nexus poll.
2. `♡ [Xs] status: running` heartbeat every 30s. The status value is always `running`; the heartbeat never reports a task id, phase, or current step.

The heartbeat line is the only signal the operator gets, and it carries no resolution below "the execution is still alive."

## Where the real stall is

The stall is server-side, upstream of inference. Evidence from nexus after the CLI gave up:

- `hex plan active` lists 4 concurrent workplan executions, **all at progress 0/6 on their first phase**, including b026500d for wp-bazzite-e2e-arch-validation and two separate runs of wp-fix-workplan-inference-stalling itself.
- `hex task list` shows 6 `brain-lease` tasks, every one `pending`, `agent` column empty.

So: nexus accepted the workplan, enqueued its phase-1 tasks on the `brain-lease` swarm, and then nothing assigned those tasks to an agent. No task ever transitioned to `in_progress`. The dispatch step that should emit a subagent-spawn event to the attached Claude Code session (Path B, per `CLAUDE_SESSION_ID`) never fired — or fired into a void. Inference was never attempted, so a per-task inference timeout would not have helped.

## Implication for the rest of this workplan

- **P2-1 TimeoutGuard (per-tier 30s/120s/300s/600s):** insufficient on its own. The guard would only fire if tasks actually started running. Today they sit `pending` forever and the CLI's 600s global timeout is the only thing that eventually unsticks the caller.
- **P3 Heartbeat:** the existing `♡ [Xs] status: running` is too coarse to detect this class of failure. It only proves the CLI's poll loop is alive. The heartbeat payload needs to surface `task_id`, `phase`, and `current_step` from the nexus side, not the CLI side — otherwise an operator watching the log sees steady output while nothing is advancing.
- **P4 Orphan cleanup / P5 state sync:** directly applicable. A task that is `pending` with no agent and no claim for >N seconds should be reaped or re-dispatched by nexus.
- **Missing rung:** a P1.5 task to verify nexus dispatcher → agent-claim path. Currently `pending` tasks in `brain-lease` never attract an agent. Suspect sites: `hex-nexus/src/coordination/` (swarm claim), the subagent-spawn hook on UserPromptSubmit, `agent-registry` WASM module heartbeats.

## Instrumentation note on `hex-cli/src/commands/plan/executor.rs`

P1-1 added a fn-main scaffold with `execute_task`, `execute_inference`, `receive_ollama_response`, `check_compile_gate`, and state-transition logs. That file is a standalone binary demo — it is not on the live `hex plan execute` call path. The live path is `CLI → POST nexus → nexus brain-lease queue → (agent claim) → inference`, and this run never reached the agent-claim step. The scaffold's instrumentation pattern is still useful as a target shape once the real executor is wired, but P1-1's instrumentation didn't and couldn't fire in this run.

## Artifact

Full captured log preserved at `docs/analysis/p1-2-debug-stall-2026-04-24.log.txt`.
