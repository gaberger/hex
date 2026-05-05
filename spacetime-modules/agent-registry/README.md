# agent-registry

> Agent lifecycle + heartbeat registry (ADR-027 / ADR-048).

WASM module that tracks live AI dev agents — registration, heartbeat, status transitions (`registered` → `active` → `stale` → `dead`), and cleanup-run audit. Cleanup is invoked by hex-nexus on a periodic schedule (WASM cannot keep wall-clock state, so the cutoffs are passed in by the caller).

## Tables

| Table | Visibility | Key | Purpose |
|---|---|---|---|
| `agent` | public | `id` (unique) | One row per dev agent — name, project, model, status, started_at, ended_at, metrics_json |
| `agent_heartbeat` | public | `agent_id` (unique) | Per-agent last_seen + counters (turn_count, token_usage) |
| `agent_cleanup_log` | public | `id` (auto_inc) | Audit trail — every run that flipped at least one agent to stale/dead |

## Status values

`registered` · `active` · `stale` · `dead` · `disconnected` (validated by `is_valid_status`).

## Thresholds

- `STALE_THRESHOLD_SECS = 45` — no heartbeat for 45 s → `stale`
- `DEAD_THRESHOLD_SECS = 120` — no heartbeat for 120 s → `dead` (slot reclaimable)

## Reducers

| Reducer | Args | Effect |
|---|---|---|
| `register_agent` | `id, name, project_id, project_dir, model, started_at` | Insert `agent` (status=`registered`) + empty `agent_heartbeat` |
| `update_status` | `id, status, metrics_json` | Update an existing agent — errors if not found |
| `heartbeat` | `agent_id, turn_count, token_usage` | Bump heartbeat — errors if agent not registered |
| `remove_agent` | `id` | Delete agent + heartbeat — errors if not found |
| `run_agent_cleanup` | `now, stale_cutoff, dead_cutoff` (RFC3339) | Flip agents past cutoff → stale/dead; append to `agent_cleanup_log` if any work happened |
| `trigger_agent_cleanup` | same as above | Manual cleanup wrapper (dashboard button / ad-hoc) |

`stale_cutoff = now − 45 s` and `dead_cutoff = now − 120 s` are computed by the caller (hex-nexus) — the WASM compares strings lexicographically (Z is normalized to `+00:00`).

## Subscriptions

Clients typically subscribe to:

```sql
SELECT * FROM agent WHERE status IN ('registered', 'active', 'stale')
SELECT * FROM agent_heartbeat
SELECT * FROM agent_cleanup_log ORDER BY id DESC LIMIT 50
```

## Example flow

```
register_agent("uuid-...", "coder-1", "proj-1", "/tmp/proj", "claude-opus-4-7", "2026-05-04T12:00:00Z")
heartbeat("uuid-...", 5, 12_345)         // every UserPromptSubmit via `hex hook route`
update_status("uuid-...", "active", "{}")
run_agent_cleanup(now, now-45s, now-120s) // every ~30 s from hex-nexus
remove_agent("uuid-...")                  // explicit teardown
```

## ID format

`is_valid_agent_id` requires UUID-v4 shape (36 chars, 4 hyphens). Caller is responsible for generating IDs.
