# ADR-046: SpacetimeDB Single Authority for State Mutations

**Status:** Accepted
**Date:** 2026-03-21
**Drivers:** Ghost re-registration bug — ControlPlane was re-creating deleted projects via REST

## Context

The dashboard connects directly to SpacetimeDB via WebSocket subscriptions. When a project was deleted via the `removeProject` reducer, the ControlPlane component was also re-registering projects via REST `POST /api/projects/register` on every page load. This created a dual-authority conflict where deleted projects would immediately reappear.

## Decision

**SpacetimeDB reducers are the ONLY authority for state mutations in the dashboard.**

### Rules

1. **Dashboard → SpacetimeDB**: All state reads and writes go through WebSocket (reducers + subscriptions)
2. **Dashboard → REST (hex-nexus)**: ONLY for filesystem side-effects that WASM cannot perform:
   - Scaffolding files (`POST /api/projects/init`)
   - Deleting files from disk (`POST /api/projects/:id/delete`)
   - Architecture analysis (`POST /api/analyze`)
   - Git operations (`GET /api/:id/git/*`)
3. **REST must NEVER mutate SpacetimeDB state**: No REST endpoint should register, unregister, or update project records. If it does, it must be a pass-through to a SpacetimeDB reducer, not a parallel write.
4. **No background re-sync from REST to SpacetimeDB**: If a project exists in SpacetimeDB, it exists. Period. No "ensure registered" patterns.

### Allowed Patterns

```typescript
// CORRECT: reducer for state, REST for filesystem
conn.reducers.registerProject(id, name, path, timestamp);  // state
await fetch("/api/projects/init", { body: { path } });      // filesystem

// CORRECT: read from subscription, not REST
const projects = registeredProjects();  // SpacetimeDB subscription

// WRONG: mutating state via REST
await fetch("/api/projects/register", { body: { rootPath } });  // NO!

// WRONG: re-syncing state on page load
onMount(() => projects().forEach(p => fetch("/api/projects/register")));  // NO!
```

## Consequences

**Positive:**
- Single source of truth — no ghost records
- Instant UI updates via WebSocket (no polling, no REST round-trips)
- Deletes are permanent and consistent

**Negative:**
- Developers must understand which operations need REST (filesystem) vs reducers (state)
- If SpacetimeDB is down, state mutations fail (no REST fallback for state)
