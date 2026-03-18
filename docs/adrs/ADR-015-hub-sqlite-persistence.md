# ADR-015: SQLite Persistence for Hub Swarm State

## Status: Accepted
## Date

2026-03-17

## Context

hex-hub is a Rust binary (axum + tokio) that serves as the coordination server for multi-agent swarm orchestration. It manages project registration, WebSocket command dispatch, worktree locks, task claims, and activity streams.

All state was stored in in-memory `HashMap`s — lost on hub restart. This created two problems:

1. **Session recovery**: When a Claude Code session crashes mid-swarm, the next session has no way to discover what was in-flight. We used markdown memory files (`MEMORY.md`) as a workaround, but these go stale between sessions and require manual reconciliation.

2. **Swarm lifecycle tracking**: There was no concept of a "swarm" as a first-class entity. Tasks and agents were tracked ephemerally through ruflo's in-memory registry (now superseded by HexFlo, see ADR-027), which also resets on restart.

The hub is the natural home for persistent state because:
- It's already a long-running daemon (started once, serves many sessions)
- All coordination traffic already flows through it (registration, heartbeat, commands)
- It has HTTP endpoints that any session can query

## Decision

Add SQLite persistence to hex-hub using `rusqlite` with the `bundled` feature (compiles SQLite from source, zero system dependencies).

### Storage

Database file: `~/.hex/hub.db` (created automatically on first startup).

### Schema

Three tables with auto-migration on startup (`CREATE TABLE IF NOT EXISTS`):

```sql
-- Swarm orchestration lifecycle
CREATE TABLE swarms (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  name TEXT NOT NULL,
  topology TEXT NOT NULL DEFAULT 'hierarchical',
  status TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- Tasks within a swarm
CREATE TABLE swarm_tasks (
  id TEXT PRIMARY KEY,
  swarm_id TEXT NOT NULL REFERENCES swarms(id),
  title TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  agent_id TEXT,
  result TEXT,
  created_at TEXT NOT NULL,
  completed_at TEXT
);

-- Agents assigned to a swarm
CREATE TABLE swarm_agents (
  id TEXT PRIMARY KEY,
  swarm_id TEXT NOT NULL REFERENCES swarms(id),
  name TEXT NOT NULL,
  role TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'idle',
  worktree_path TEXT
);
```

### API Routes

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/api/swarms` | Create a new swarm |
| GET | `/api/swarms/active` | List all non-completed swarms |
| GET | `/api/swarms/:id` | Get swarm with tasks and agents |
| PATCH | `/api/swarms/:id/tasks/:taskId` | Update task status/result |
| GET | `/api/work-items/incomplete` | Session recovery: all in-flight work |

### Async Safety

SQLite operations use `tokio::task::spawn_blocking` to keep the async runtime non-blocking. The `Connection` is behind `Arc<Mutex<Connection>>` for thread safety.

### Initialization

`SwarmDb::open()` is called on startup in `main.rs`. If it fails (e.g., disk full), the hub logs a warning and continues without persistence — existing in-memory coordination still works.

## Consequences

### Positive
- Swarm state survives hub restarts and session crashes
- New sessions can query `GET /api/work-items/incomplete` to resume where they left off
- Replaces fragile markdown memory files for orchestration state
- SQLite is embedded (no separate server), crash-safe (WAL mode), and well-tested
- `bundled` feature means zero system dependencies

### Negative
- Hub binary size increases (~1.5MB for bundled SQLite)
- Cargo build time increases (~30s for initial rusqlite compilation)
- Need to handle schema migrations as the schema evolves (currently using `CREATE TABLE IF NOT EXISTS`)

### Future Work
- Wire TypeScript `DashboardAdapter` to call new swarm endpoints (`pushSwarmState`, `queryIncompleteWork`)
- Add swarm visualization to hex-hub dashboard UI
- Consider SQLite WAL mode for better concurrent read performance
- Add `hex recover` CLI command that queries the hub for incomplete work
