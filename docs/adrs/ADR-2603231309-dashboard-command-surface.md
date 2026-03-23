# ADR-2603231309: Map All hex CLI Commands Into Dashboard UI

**Status:** Accepted
**Date:** 2026-03-23
**Drivers:** Developer workflow friction — switching between terminal windows and the dashboard breaks flow. The dashboard should be the single pane of glass for all hex operations.
**Supersedes:** None

## Context

The hex dashboard currently has 11 pages and a command palette (Ctrl+P) with only **15 entries**, almost all navigation shortcuts. Meanwhile, the hex CLI exposes **20+ command groups** (swarm, task, memory, inbox, adr, project, agent, analyze, plan, inference, git, skill, enforce, secrets, status, readme, test) — most of which already have REST API endpoints in hex-nexus but no dashboard UI surface.

Forces at play:

- **Developer expects single pane of glass**: The dashboard is meant to be the nexus of control (ADR-039). If a developer must open a terminal to run `hex task list` or `hex memory search`, the dashboard fails its purpose.
- **REST endpoints already exist**: ~140 routes are registered in hex-nexus. The dashboard only calls a fraction of them.
- **Command dispatch infrastructure exists**: `POST /api/{project}/command` with WebSocket broadcast and result tracking is fully implemented but underutilized.
- **Command palette is the fast path**: Ctrl+P fuzzy search is the quickest way to invoke operations. Expanding it from 15 to 60+ commands gives power users immediate access.

Alternatives considered:

1. **Embedded terminal** — Too heavy, security concerns with shell access from browser, duplicates CLI without adding value.
2. **Command palette only** — Fast for power users but no discoverability for new users; some operations need forms (e.g., creating tasks, storing memory).
3. **Full page per command group** — Already partially done (agents, swarms, ADRs have pages). Extend this pattern plus expand the palette.

## Decision

We will implement a **two-tier command surface** in the dashboard:

### Tier 1: Command Palette Expansion (Ctrl+P)

Expand `stores/commands.ts` to include **all hex CLI operations** that have REST endpoints. Each command entry calls the REST client directly and shows results via toast notifications. New categories added: `"analysis"`, `"memory"`, `"task"`, `"git"`.

Commands are grouped by category matching the CLI subcommand structure:

| Category | Commands | Transport | Why |
|----------|----------|-----------|-----|
| project | register, unregister, archive, status | SpacetimeDB reducers + signals | State read/write (ADR-046) |
| agent | list, connect, disconnect, audit | SpacetimeDB signals + reducers | State read/write |
| swarm | init, status, list | SpacetimeDB signals + reducers | State read/write |
| task | create, list, complete, assign | SpacetimeDB signals + reducers | State read/write |
| memory | store, get, search | SpacetimeDB signals + reducers | State read/write |
| inbox | list, notify, ack | SpacetimeDB reducers | State write |
| analysis | analyze, adr-compliance | REST (`/api/analyze*`) | Filesystem op |
| git | status, log, diff, branches | REST (`/api/{project}/git/*`) | Filesystem op |
| settings | refresh config, sync skills | REST (`/api/config/*`, `/api/skills/*`) | Filesystem→SpacetimeDB bridge |

**Transport rule (ADR-046):** State reads use SpacetimeDB subscription signals. State writes use SpacetimeDB reducers. REST is ONLY for filesystem operations (analysis, git, config sync from disk).

### Tier 2: Action Panels on Existing Pages

Commands that need **input forms** (task create, memory store, inbox notify) get inline action panels on their respective pages. No new pages — extend existing ones:

- **Swarms page** → Task create/complete forms, task list table
- **Agents page** → Connect/disconnect actions, fleet audit button
- **Config page** → Skill sync, enforce apply buttons
- **Control Plane** → Project register/unregister, global status
- **Health page** → Re-analyze button (already exists), ADR compliance toggle

### Tier 3: Command Output Panel (new component)

A collapsible bottom panel (`CommandOutputPanel.tsx`) that shows:
- Recent command history (last 20)
- Live command results (streamed via WebSocket `project:{id}:result` topic)
- Error details with retry buttons

This replaces the need for a terminal — structured output is better than raw text.

### REST Endpoint Gaps to Fill

These CLI commands lack REST endpoints and need new routes in hex-nexus:

| Command | New Endpoint | Handler |
|---------|-------------|---------|
| `hex git status` | `GET /api/{project}/git/status` | Shell out to git |
| `hex git log` | `GET /api/{project}/git/log` | Shell out to git |
| `hex git diff` | `GET /api/{project}/git/diff` | Shell out to git |
| `hex git branches` | `GET /api/{project}/git/branches` | Shell out to git |
| `hex skill list` | `GET /api/skills` | Read skills dir |
| `hex skill sync` | `POST /api/skills/sync` | Copy skills |
| `hex enforce list` | `GET /api/enforce/rules` | Read enforce config |
| `hex enforce apply` | `POST /api/enforce/apply` | Apply rules |
| `hex readme validate` | `POST /api/readme/validate` | Validate README |
| `hex status` | `GET /api/{project}/status` | Aggregate status |
| `hex inbox list` | `GET /api/inbox` | Query inbox |
| `hex inbox notify` | `POST /api/inbox` | Send notification |
| `hex inbox ack` | `POST /api/inbox/{id}/ack` | Acknowledge |

## Consequences

**Positive:**
- Developer can perform ALL hex operations from the dashboard without opening a terminal
- Command palette becomes a true power-user tool (60+ commands via Ctrl+P)
- Command output panel provides structured, searchable history (better than scrollback)
- No new pages to maintain — extends existing pages with action panels
- REST endpoints added for gap commands benefit MCP tools and external integrations too

**Negative:**
- 12 new REST endpoints to implement and maintain
- Command palette could feel overwhelming with 60+ entries (mitigated by fuzzy search + categories)
- Git operations from the dashboard require careful security review (path traversal, command injection)

**Mitigations:**
- Git endpoints use the existing `safePath()` validation and only accept predefined operations (no arbitrary git commands)
- Command palette already has category-based fuzzy search which scales well
- New endpoints follow the exact same patterns as existing ones — no new abstractions needed

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Expand `commands.ts` with 40+ commands using SpacetimeDB signals/reducers | Done |
| P2 | Add `CommandOutputPanel.tsx` bottom panel with tracked command history | Done |
| P3 | Add task drill-down chain (Task→Agent→Worktree→Commit) in SwarmDetail | Done |
| P3b | Add InboxPanel with SpacetimeDB agent_inbox subscription | Done |
| P4 | Implement REST endpoints for filesystem-only gap commands (git, enforce) | Pending |
| P5 | Add remaining commands to palette that depend on P4 endpoints | Pending |
| P6 | Add new `CommandCategory` types and update category colors | Pending |

## References

- ADR-039: Command Palette (Ctrl+P access methods)
- ADR-056: REST client singleton pattern
- ADR-060: Inbox notification system
- `hex-nexus/src/routes/commands.rs` — existing command dispatch infrastructure
- `hex-nexus/assets/src/stores/commands.ts` — current 15-entry command registry
