# OTP Supervisor in SpacetimeDB + Agent Thought Memory — design

**Status:** draft. NOT a workplan — do not auto-execute.
**Origin:** 2026-05-07 incident (CPU pinning + chat collapse). CEO directive: "the team is ALWAYS ready to work; workers may sleep, never disappear; the supervisor must be in SpacetimeDB, not a Rust background task or shell loop."

## What already exists in STDB (agent-registry + hex modules)

The OTP primitives for **process-backed workers** are live:

| Table (db) | Purpose |
|---|---|
| `worker_pool_intent` (hex) | declarative desired state: `id` (pk), `role`, `desired_count`, `restart_strategy ∈ {permanent, transient, temporary}`, `max_restarts`, `max_restart_window_secs`, `paused`, `in_crash_loop`, `owner_agent_id` |
| `worker_process` (hex) | actual fleet: `id`, `pool_id`, `role`, `host`, `pid`, `started_at`, `last_heartbeat`, `restart_count`, `in_crash_loop`, `exited_at`, `exit_reason` |
| `supervisor_event` (hex) | command/audit stream: `id`, `ts`, `kind ∈ {spawn_request, ...}`, `pool_id`, `worker_id`, `payload` (JSON), `handled`, `handled_at`, `handled_by` |
| `supervisor_tick_schedule` (hex) | STDB scheduled reducer firing every 10s |
| `agent` + `agent_heartbeat` (agent-registry) | conversation/persona registry (separate from worker_process) |

A STDB scheduled reducer (the "supervisor tick") inspects `worker_pool_intent` vs `worker_process`, emits `supervisor_event`s. The nexus-side `supervisor_subscriber` consumes events and executes side-effects (start a process, mark a worker exited). `handled_by="nexus-supervisor"` confirms the round-trip works.

**This is the OTP topology already in place — declarative intent in STDB, deterministic decision, side-effects on the nexus side because WASM can't fork processes.** Same as Erlang/OTP except processes live outside the BEAM.

## Gap 1 — fleet state is under-seeded

`worker_pool_intent` has 1 row (`pm-agent-default`). The other ~30 role pools the supervisor spawns at startup have no declarative intent. Net: the supervisor has no opinion on them, they get spawned imperatively each restart, and ceiling enforcement / restart_intensity are inactive for everyone except pm-agent.

**Fix (STDB-side):**

1. Seed `worker_pool_intent` for every role found in `hex-cli/assets/agents/hex/hex/*.yml` on first boot (idempotent: `INSERT ... ON CONFLICT DO NOTHING` equivalent — STDB pattern is `if !pool_exists(id) { insert }`).
2. The seeding runs as a STDB **`__init__` reducer** in agent-registry or a new `supervisor` module. Not on the nexus side.
3. Sane defaults per role tier — execs `desired_count=1, restart_strategy=permanent, max_restarts=5/300s`; ICs `desired_count=1, max_restarts=10/60s`.

## Gap 2 — personas have no supervisor primitive

Executive personas (cto, cpo, coo, ciso, chief-visionary, engineering-lead, product-lead, sre-lead) aren't process-backed. They live as conversation entities answered by `org_responder`. Today they only "exist" when they reply to a DM — not in the registry, so the dashboard shows the team offline and there's no durable "always ready" guarantee.

**Fix (STDB-side, two options):**

### Option A — extend `worker_pool_intent` with a `kind` discriminator
Add `kind ∈ {process, persona}` column. The supervisor tick treats `kind=persona` pools differently:
- No spawn_request emitted (no process).
- Instead, ensure an `agent` row exists for the role and refresh `agent_heartbeat.last_seen=now` on each tick.
- Sleep state: `paused=true` → skip heartbeat refresh; persona goes stale; `org_responder` skips it.

### Option B — separate `persona_pool` table
Cleaner schema separation. Same fields as `worker_pool_intent` but no `restart_strategy` (no restart since no process). Sibling scheduled reducer.

**Recommendation: Option A.** One supervisor for both kinds, one tick, one event log. The discriminator is cheap.

### Acceptance for Gap 2

- After STDB module republish, query `agent_heartbeat WHERE agent_id IN (persona ids)` — `last_seen` is within the last tick interval.
- `hex agent list` shows all 8 personas as online.
- Dashboard org chart shows green dots for the executive tier.
- `hex pool pause cto` (new CLI) flips `worker_pool_intent.paused=true`; on next tick, cto's heartbeat stops refreshing; within ~45s `last_seen` is stale and dashboard shows cto offline. `hex pool resume cto` restores.
- No shell loops, no nexus tokio tasks for heartbeat. The reducer + scheduler do it.

## Gap 3 — inference-failure budget

The 2026-05-07 chat collapse was caused by `org_responder` retrying `claude-haiku-4-5-20251001` indefinitely against OpenRouter (no such model). No backoff, no benching.

**Fix:** new STDB table `persona_health { role, recent_failures (u32), last_failure_at, banned_until }`. `org_responder` writes `record_failure(role, model_id, status_code)` reducer on each 4xx; the reducer:
- increments recent_failures within a rolling 60s window
- if ≥3, sets `banned_until = now + 5min`
- emits a `supervisor_event` with kind=`inference_bench` and payload `{role, model_id, count}`
- `org_responder` checks `banned_until > now` before each inference call and skips

Writes the `record_failure` *to STDB* — not to nexus state, not to a file, not in-memory. Survives nexus crashes.

## Phase 2 — agent thought memory (unchanged from before)

| Table | Module | Purpose |
|---|---|---|
| `agent_thought` | chat-relay (or new `cognition` module) | `thought_id` (pk), `agent_role`, `kind ∈ {decision, observation, plan, frustration, learning, commitment}`, `content` (≤4 KB), `related_task_id`, `related_msg_id`, `confidence`, `created_at` |

Reducer `journal_thought(role, kind, content, related_*)`. After every successful `org_responder` reply, prompt persona for a 1-line reasoning summary (≤64 tokens, capped) and write a thought row linked to the reply's msg_id. `improver::Source::ThoughtPattern` detects ≥3 same-kind on same scope within 24h → workplan suggestion.

## Behavioral specs

1. **BS-1 idempotent restart.** worker_pool_intent has 1 row for cto-default. Restart nexus + STDB cleanly. After convergence: agent_heartbeat for cto is fresh, exactly one agent row for cto in registry. No duplicate spam (current state has 25 dupes — that's the bug).
2. **BS-2 inference backoff.** 3 sustained HTTP 400 for `model=claude-haiku-4-5-20251001` on role=cto → persona_health.banned_until in future, supervisor_event of kind=inference_bench, no further inference for cto until banned_until passes.
3. **BS-3 sleep / resume.** `hex pool pause cto` flips paused=true → next tick, no heartbeat refresh; last_seen goes stale; dashboard shows cto offline; `hex pool resume cto` flips paused=false → next tick refreshes; cto online.
4. **BS-4 thought journal.** After 3 successful CTO replies, `agent_thought` has 3 rows for role=cto with kind=decision, each linked to a reply msg_id.
5. **BS-5 thought pattern.** CTO journals 3× kind=frustration with related_task_id LIKE 'auth-%' within 24h → improver discover emits ThoughtPattern hypothesis.

## Validation gates

- `cargo check --workspace`
- `spacetime build` for each module (agent-registry, chat-relay/cognition, hex)
- `cargo test -p hex-nexus --tests supervisor_subscriber` — verify nexus consumes new event kinds correctly
- 3× nexus + STDB restart sequence — exactly desired_n alive after each, no duplicate persona rows
- Inject a known-bad model_id → ban fires within 3 attempts, lifts after 5min

## Where the work lives

| Concern | Module / file |
|---|---|
| Schema changes (worker_pool_intent + persona_health + agent_thought + kind discriminator) | `spacetime-modules/agent-registry/src/lib.rs`, `spacetime-modules/chat-relay/src/lib.rs` (or new `spacetime-modules/cognition/`) |
| Scheduled reducer (supervisor tick) — read intent, refresh heartbeats, emit events | extend the existing tick reducer in agent-registry |
| `record_failure` reducer | new in agent-registry |
| `journal_thought` reducer | chat-relay/cognition |
| Nexus subscriber for new event kinds (inference_bench, persona heartbeat done — though heartbeat is in-STDB so no subscriber needed) | `hex-nexus/src/orchestration/supervisor_subscriber.rs` |
| `hex pool` CLI (list / set / pause / resume / unban) | `hex-cli/src/commands/pool/mod.rs` |

## What NOT to do

- No tokio background tasks in hex-nexus for heartbeating personas. STDB reducers do this.
- No shell loops anywhere.
- No `docs/workplans/wp-*.json` drafts of this spec — the workplans dir is a live execution surface (see `feedback_workplan_overwrite_hazard.md`). Convert to a real workplan only on user request, with the live schema (id=UUID, name, version, steps[]).
