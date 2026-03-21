# ADR-043: Project Manifest + Auto-Registration via SpacetimeDB

- **Status**: Proposed
- **Date**: 2026-03-21
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
