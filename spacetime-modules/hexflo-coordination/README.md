# hexflo-coordination

> Swarm coordination + project registry + agent identity + memory + lifecycle (ADR-027 / ADR-058).

The largest module — the coordination backbone for HexFlo swarms, the unified hex-agent identity (ADR-058), workplan/inference task dispatch, and per-project state. Reducers use **CAS optimistic locking** (`version` field on `swarm_task`) to prevent double-assignment across remote nodes without distributed locks (ADR-2026-03-24-1900).

## Tables (grouped by concern)

### Swarms + tasks

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `swarm` | public | `id` (unique) | Named group with `owner_agent_id`, topology (`hierarchical`/`mesh`/`pipeline`/`star`), status |
| `swarm_task` | public | `id` (unique) | Unit of work — `agent_id` (empty=unassigned), `status`, `depends_on` (CSV), `version` (CAS) |
| `inference_task` | public | `id` (PK) | Workplan executor → agent inference dispatch — `workplan_id`, `task_id`, `phase`, `prompt`, `role` |
| `swarm_agent` | public | `id` (unique) | Legacy swarm-participant view |
| `hex_agent` | public | `id` (PK) | **Unified agent identity** (ADR-058) — replaces fragmented registries |

### Project + config sync (ADR-044)

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `project` | public | `id` (unique) | Registered project — root path, name, tags |
| `project_config` | public | composite | Per-project key/value config from `.hex/project.json` |
| `skill_registry` | public | `id` | Per-project skills (slash commands) |
| `agent_definition` | public | `id` | Per-project agent YAMLs |
| `mcp_tool` | public | `id` | Registered MCP tools |
| `architecture_fingerprint` | public | `project_id` | AST-derived fingerprint |

### Remote agents + inference servers

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `remote_agent` | public | `id` | Off-host agent — heartbeats, capabilities |
| `inference_server` | public | `id` | Discoverable inference endpoint (Ollama/vLLM) |
| `compute_node` | public | `id` | GPU node registry — health, agent count |

### Notifications + briefings

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `agent_inbox` | public | composite | Per-agent inbox (priority, ack, expiry) |
| `developer_inbox` | public | composite | Human inbox (decision-needed surfacing) |
| `briefing_buffer` | public | `id` (auto_inc) | Recent events for agent briefings |

### Workplan + lifecycle

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `workplan_event` | public | `id` (auto_inc) | Append-only workplan event log |
| `quality_gate_task` | public | `id` | Architecture/test quality gates |
| `fix_task` | public | `id` | Auto-spawned fix tasks for violations |
| `dev_session` | public | `id` | Dev session lifecycle (phases, paths, quality) |
| `swarm_lifecycle` / `lifecycle_task` / `phase_transition_log` | public | various | Phase-transition state machine |

### Inference logs + enforcement

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `inference_log` | public | `id` (auto_inc) | Per-call latency/cost log |
| `enforcement_rule` | public | `id` | Architecture enforcement rules (toggleable) |

### Memory (key/value with scope)

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `hexflo_memory` | public | `key` (unique) | Cross-agent KV store with scope (`project`/`swarm`/`agent`) |

### Trust + delegation

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `delegation_trust` | public | composite | Per-scope trust score with decay + pinning |

### Hot-swap + shadow testing

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `swap_ticket` | public | `id` | Inference provider hot-swap ticket |
| `shadow_sample` | public | `id` (auto_inc) | Shadow-traffic samples for A/B comparison |

### Hypothesis-driven research (objective/hypothesis/verdict)

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `objective` | public | `id` | Research objective |
| `hypothesis` | public | `id` | Falsifiable hypothesis under an objective |
| `verdict` | public | `id` | Outcome verdict — supports/refutes/inconclusive |

### Worker pool supervisor (ADR-2026-04-28-0000)

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `worker_pool_intent` | public | `id` | Desired worker pool state |
| `worker_process` | public | `id` | Live worker process — heartbeat, exit code |
| `supervisor_event` | public | `id` (auto_inc) | Supervisor decisions (start/restart/exit) |
| `supervisor_tick_schedule` | scheduled | `id` | Periodic `supervisor_tick` |

### Cleanup

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `cleanup_log` | public | `id` (auto_inc) | Audit log for `coordination_cleanup` runs |

## Reducers (107 total — grouped)

Reducer signatures live in `src/lib.rs`. Groups:

- **Agent identity (ADR-058):** `agent_connect`, `agent_disconnect`, `agent_update_capabilities`, `agent_heartbeat_update`, `agent_assign_swarm`, `agent_evict_dead`, `agent_mark_inactive`, plus legacy `agent_register`, `agent_heartbeat`, `agent_mark_stale`, `agent_mark_dead`, `agent_remove`.
- **Project sync:** `register_project`, `remove_project`, `sync_config`, `sync_skill`, `sync_agent_def`, `mcp_tool_sync`, `upsert_fingerprint`, `delete_fingerprint`.
- **Remote agents + inference servers:** `register_remote_agent`, `remote_agent_heartbeat`, `update_remote_heartbeat`, `update_remote_status`, `deregister_remote_agent`, `list_remote_agents_by_host`, `register_inference_server`, `remove_inference_server`.
- **Notifications + briefings:** `notify_agent`, `notify_all_agents`, `acknowledge_notification`, `expire_stale_notifications`, `log_briefing_event`, `mark_briefing_seen`, `archive_old_briefings`.
- **Swarms + tasks:** `swarm_init`, `swarm_complete`, `swarm_fail`, `task_create`, `task_assign` (CAS — pass `expected_version`), `swarm_transfer`, `task_complete`, `task_fail`, `task_reclaim`.
- **Inference tasks:** `inference_task_create`, `inference_task_claim`, `inference_task_promote`, `inference_task_complete`, `inference_task_fail`.
- **Quality gates + fix tasks:** `create_quality_gate`, `complete_quality_gate`, `create_fix_task`, `complete_fix_task`.
- **Workplan events:** `workplan_event_append`.
- **Memory:** `memory_store`, `memory_delete`, `memory_clear_scope`.
- **Sessions:** `session_create`, `session_update_phase`, `session_complete_step`, `session_set_quality`, `session_finalize`, `session_set_paths`.
- **Inference log:** `inference_log_create`.
- **Enforcement rules:** `enforcement_rule_upsert`, `enforcement_rule_toggle`, `enforcement_rule_delete`.
- **Cleanup:** `coordination_cleanup`, `remove_dead_swarm_agent`, `trigger_cleanup`.
- **Compute nodes:** `register_node`, `update_node_health`, `increment_node_agents`, `decrement_node_agents`, `remove_node`.
- **Lifecycle (phase transitions):** `lifecycle_register_swarm`, `lifecycle_register_task`, `lifecycle_on_task_complete`, `lifecycle_on_task_fail`, `lifecycle_check_unblocked`.
- **Decisions (developer inbox):** `surface_decision`, `resolve_decision`, `expire_decisions`.
- **Trust:** `set_trust`, `decay_trust`, `pin_trust`, `init_project_trust`.
- **Hot-swap + shadow:** `swap_ticket_create`, `swap_ticket_transition`, `swap_ticket_set_config`, `swap_ticket_set_shadow_started`, `shadow_sample_record`.
- **Research:** `objective_create`, `objective_update_status`, `hypothesis_create`, `hypothesis_update_status`, `verdict_record`.
- **Worker pool supervisor:** `worker_pool_intent_set`, `worker_pool_intent_set_paused`, `worker_pool_intent_delete`, `worker_process_register`, `worker_process_heartbeat`, `worker_process_record_exit`, `supervisor_event_handle`, `supervisor_init`, `supervisor_tick` (scheduled).

## Key contracts

### Optimistic CAS on `swarm_task` (ADR-2026-03-24-1900)

`task_assign(task_id, agent_id, expected_version)` checks `swarm_task.version == expected_version` and increments on success. On mismatch, returns ConflictError — prevents double-claim across remote nodes without distributed locks.

### Single active swarm per agent

`swarm_init` rejects if `owner_agent_id` already owns an `active` swarm. Use `swarm_transfer` to hand ownership to another agent.

### Memory scope

`memory_store(key, value, scope)` accepts `scope ∈ {project, swarm, agent}`. Use `memory_clear_scope` to wipe everything in one scope.

### Notification priority + override (ADR-060)

`notify_agent` accepts `priority` 0–3. Priority 2+ is treated as override-current-work by the route hook. Acks via `acknowledge_notification`.

## Subscriptions

Common patterns:

```sql
SELECT * FROM hex_agent WHERE status IN ('online', 'idle')
SELECT * FROM swarm WHERE status = 'active'
SELECT * FROM swarm_task WHERE swarm_id = ? ORDER BY created_at
SELECT * FROM inference_task WHERE status = 'Pending' ORDER BY created_at
SELECT * FROM agent_inbox WHERE agent_id = ? AND ack = 0
SELECT * FROM hexflo_memory WHERE scope = ? AND key LIKE ?
SELECT * FROM workplan_event WHERE workplan_id = ? ORDER BY id
```

## Cleanup cadence

- `coordination_cleanup(cutoff)` — global sweep run by hex-nexus on a tick.
- `expire_stale_notifications(now)` — TTL'd inbox cleanup.
- `archive_old_briefings(cutoff)` — briefing buffer roll-off.
- `decay_trust(now, decay_per_day)` — trust decay.
- `expire_decisions(now)` — TTL'd decisions.
- `supervisor_tick` (scheduled) — worker process supervisor.
