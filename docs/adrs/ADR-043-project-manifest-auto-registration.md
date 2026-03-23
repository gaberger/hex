# ADR-043: Project Manifest + Auto-Registration via SpacetimeDB

**Status:** Accepted
**Accepted Date:** 2026-03-22
## Date: 2026-03-21

> **Implementation Evidence:** `ProjectManifest` parser and `auto_register_project()` in `hex-nexus/src/config_sync.rs`. Wired into nexus startup in `lib.rs`. `.hex/project.yaml` scaffolded by `hex init` (`hex-cli/src/commands/init.rs`). SpacetimeDB `register_project` reducer exists in `hexflo-coordination` module. REST register route already delegates to SpacetimeDB.
- **Informed by**: ADR-025 (SpacetimeDB), ADR-037 (agent lifecycle), ADR-040 (remote agents)
- **Authors**: Gary (architect), Claude (analysis)

## Context

The hex-nexus dashboard shows a blank project list because projects registered via REST (`POST /api/projects/register`) are stored in nexus's in-memory HashMap, NOT in SpacetimeDB. The dashboard reads from SpacetimeDB's `project` table subscription, which is empty.

### Current State

| Operation | Where State Lives | Dashboard Reads From |
|-----------|------------------|---------------------|
| Register project | nexus in-memory HashMap | SpacetimeDB `project` table |
| Register agent | nexus in-memory + SpacetimeDB | SpacetimeDB `agent` table |
| Create swarm/task | SpacetimeDB (via HexFlo) | SpacetimeDB `swarm`/`swarm_task` |

The mismatch: projects go into REST memory, everything else goes into SpacetimeDB. Result: dashboard sidebar is blank.

### Rules (established this session)

1. **NO REST for state** — SpacetimeDB is the single source of truth
2. **REST is only for stateless file operations** — git, tree-sitter analysis, file upload
3. **If SpacetimeDB is down, the dashboard waits** — no workarounds

## Decision

### 1. Project Manifest File: `.hex/project.yaml`

Every hex project has a `.hex/project.yaml` that defines the project:

```yaml
---
name: hex-intf
description: Hexagonal Architecture Framework for LLM-Driven Development
version: "26.3.1"
created: "2024-09-15"

auto_register: true

agent:
  provider: auto
  model: claude-sonnet-4-20250514
  project_dir: .

inference:
  - name: ollama-bazzite
    host: bazzite.local
    port: 11434
    models:
      - qwen3.5:27b
      - qwen3.5:9b
```

This file is committed to the repo (force-add past `.gitignore` on `.hex/`). It defines project identity, default agent configuration, and known inference servers.

### 2. Auto-Registration on Nexus Startup

When `hex nexus start` runs, the nexus:

1. Reads `.hex/project.yaml` from the current working directory
2. If `auto_register: true`, calls the SpacetimeDB `registerProject` reducer via the hexflo-coordination module
3. The project immediately appears in the dashboard sidebar
4. If SpacetimeDB is not connected, queues the registration and retries on connection

```rust
// In start_server(), after SpacetimeDB connection is established:
if let Some(manifest) = read_project_manifest(&cwd) {
    if manifest.auto_register {
        hexflo_conn.reducers.register_project(
            manifest.name,
            manifest.description,
            cwd.to_string_lossy(),
            chrono::Utc::now().to_rfc3339(),
        );
    }
}
```

### 3. Agent Auto-Registration in SpacetimeDB

When `hex-agent` starts with `--hub-url`, it registers itself in SpacetimeDB (not REST):

1. Connect to SpacetimeDB hexflo-coordination module
2. Call `registerProject` reducer with its `--project-dir`
3. Call agent registry reducers for agent registration

The current REST-based `POST /api/projects/register` auto-register in the agent's `main.rs` must be replaced with a SpacetimeDB reducer call.

### 4. Remove REST State Routes (Migration Path)

Phase 1 (immediate): Nexus REST register route calls SpacetimeDB reducer internally
Phase 2 (next): Dashboard removes all REST state calls, uses SpacetimeDB only
Phase 3 (final): Remove REST state routes entirely, keep only file operation routes

### 5. Nexus SharedState Becomes a Cache

The `state.projects` HashMap becomes a read-through cache of SpacetimeDB:
- Populated from SpacetimeDB subscription on startup
- Updated reactively when SpacetimeDB events arrive
- Used for quick lookups in git routes (`resolve_project_path`)
- Never written to directly by REST routes

## Implementation Progress

| Component | Status | Notes |
|-----------|--------|-------|
| SpacetimeDB project table | Done | `hexflo-coordination` module has project table with reducers |
| REST register route | Done | `POST /api/projects/register` calls SpacetimeDB reducer |
| `.hex/project.yaml` manifest | Done | Parser in `config_sync.rs`, `hex init` scaffolds it |
| Auto-registration on startup | Done | Two paths: server-side (`config_sync.rs`) + CLI-side (`nexus.rs`) with retry |
| REST → SpacetimeDB migration | Done | Register route already calls SpacetimeDB reducer |
| Dashboard project subscription | Done | Dashboard subscribes to SpacetimeDB project table |
| SpacetimeDB port migration | Done | Default port changed from 3000 → 3033 to avoid Next.js/Rails conflicts |
| Ping path v2.0.5 compat | Done | Updated `SPACETIMEDB_PING_PATH` from `/database/ping` → `/v1/ping` |
| CLI retry resilience | Done | 4 attempts with [2,3,4,3]s delays to handle slow module replay |

## Lessons Learned (2026-03-23)

1. **Port 3000 conflicts**: SpacetimeDB's default port 3000 conflicts with Next.js, Rails, and other common dev servers. Changed hex default to **3033**. All defaults in code AND config files (`.hex/state.json`, `~/.hex/state.json`) must agree.
2. **Ping endpoint drift**: SpacetimeDB v2.0.5 moved `/database/ping` → `/v1/ping`. The `SPACETIMEDB_PING_PATH` constant (ADR-039 enforcement) prevented partial migration — one-line fix propagated everywhere.
3. **Status false positives**: `hex nexus status` was using a raw HTTP 200 check for SpacetimeDB connectivity. Any web server (Next.js) returns 200 for unknown paths. Now uses `is_spacetimedb_reachable()` which verifies Content-Type is not HTML.
4. **Config file overrides**: Compiled defaults are overridden by `.hex/state.json` at runtime. Changing code defaults alone is insufficient — config files must also be updated.
5. **Module replay latency**: SpacetimeDB can take >7.5s to replay WASM modules on cold start. Auto-registration must tolerate this with retry logic, not assume SpacetimeDB is ready immediately.

## Consequences

### Positive
- Dashboard shows projects immediately on nexus start
- Single source of truth (SpacetimeDB) for all state
- `.hex/project.yaml` is version-controlled — project identity travels with the repo
- Remote agents auto-register their projects consistently

### Negative
- Requires SpacetimeDB to be running for any project to appear
- `.hex/project.yaml` needs manual creation for existing projects (one-time migration)
- Nexus startup depends on SpacetimeDB connection for auto-register

### Risks
- SpacetimeDB down = no projects visible (by design — no workarounds)
- Stale `.hex/project.yaml` if project is renamed/moved
- Multiple nexus instances registering the same project (idempotent — SpacetimeDB deduplicates by ID)
