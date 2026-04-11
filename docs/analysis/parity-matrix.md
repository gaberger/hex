# Unified Parity Matrix — CLI / MCP / Dashboard / REST

**Generated**: 2026-03-22
**Sources**: `cli-to-mcp-map.md`, `mcp-to-cli-map.md`, `dashboard-views.md`, `rest-endpoint-map.md`

## Legend

| Symbol | Meaning |
|--------|---------|
| ✓ | Present and functional |
| ✗ | Missing entirely |
| ~ | Partial (exists but incomplete or mismatched) |

---

## Priority 1 — Features Missing from 2+ Surfaces

| # | Feature | CLI | MCP | Dashboard | REST | Gap Count | Notes |
|---|---------|-----|-----|-----------|------|-----------|-------|
| 1 | **Enforcement rule management** | ✓ | ✗ | ✗ | ✓ | 2 | CLI has `hex enforce *`; no MCP or dashboard UI |
| 2 | **Inbox notifications** | ✓ | ✓ | ✗ | ✓ | 1 (dash) | ADR-060; no dashboard inbox panel |
| 3 | **Fleet management** | ✗ | ✗ | ~ | ✓ | 2 | Dashboard register/unregister only; no CLI or MCP |
| 4 | **Project management** | ✓ | ✗ | ~ | ✓ | 1.5 | CLI has full CRUD; no MCP; dashboard has archive/delete but not register/unregister |
| 5 | **Inference provider management** | ✓ | ~ | ~ | ✓ | 1.5 | MCP dispatch exists but not in tools.json; dashboard shows list only |
| 6 | **Agent inspect/kill/restart** | ✗ | ✗ | ✓ | ✓ | 2 | Dashboard-only agent lifecycle controls |
| 7 | **Chat interface** | ✗ | ✗ | ✓ | ✓ | 2 | Dashboard + WS only; no CLI or MCP |
| 8 | **File browser/editor** | ✗ | ✗ | ✓ | ✓ | 2 | Dashboard-only; REST /api/files exists |
| 9 | **Configuration browser** | ~ | ✗ | ✓ | ✓ | 1.5 | Only `hex skill list` covers skills; 6 other sections have no CLI |
| 10 | **Git queries (status/log/diff/branches)** | ✗ | ✗ | ✓ | ✓ | 2 | Dashboard-only via REST git API |
| 11 | **Test management** | ✓ | ✗ | ✗ | ✓ | 2 | CLI has `hex test *`; no MCP or dashboard |
| 12 | **Secrets grant/revoke/set/get** | ✓ | ✗ | ✗ | ✓ | 2 | CLI-only secret lifecycle |
| 13 | **SpacetimeDB management** | ✓ | ✗ | ~ | ✓ | 1.5 | CLI has `hex stdb *`; dashboard has SpacetimeDBView (status only) |
| 14 | **Agent identity (`hex agent id`)** | ✓ | ✗ | ✗ | ~ | 2 | No MCP or dashboard; REST via hex-agents |
| 15 | **Agent spawn-remote** | ✓ | ✗ | ✗ | ~ | 2 | CLI-only; endpoint may not be in router |
| 16 | **README management** | ✓ | ✗ | ✗ | ✗ | 3 | CLI-only; no REST, MCP, or dashboard |
| 17 | **Coordination API** | ✗ | ✗ | ✗ | ✓ | 3 | 13 REST endpoints; zero consumers |
| 18 | **RL engine** | ✗ | ✗ | ✗ | ✓ | 3 | 6 REST endpoints; zero consumers |

---

## Priority 2 — Features by Category

### Analysis

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Analyze path | ✓ | ✓ | ✓ | ✓ |
| Analyze (JSON output) | ✓ | ~ | ✗ | ✓ |
| Analyze (strict mode) | ✓ | ✗ | ✗ | ✓ |
| ADR compliance check | ✓ | ✗ | ✗ | ✓ |
| Dependency graph | ✗ | ✗ | ✓ | ✓ |

### ADR

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| List ADRs | ✓ | ✓ | ✓ | ✓ |
| ADR detail | ✓ | ✓ | ✓ | ✓ |
| ADR search | ✓ | ✓ | ✓ | ✓ |
| ADR abandoned detection | ✓ | ✓ | ✗ | ✓ |
| ADR review | ✓ | ✗ | ✗ | ✗ |
| ADR schema/template | ✓ | ✗ | ✗ | ✗ |
| ADR inline edit/save | ✗ | ✗ | ✓ | ✓ |
| ADR number reservation | ✗ | ✗ | ✗ | ✓ |
| Project-scoped ADRs | ✗ | ✗ | ✓ | ✓ |

### Swarm

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Initialize swarm | ✓ | ✓ | ✓ | ✓ |
| Swarm status | ✓ | ✓ | ✓ | ✓ |
| List all swarms | ✗ | ✗ | ✗ | ✓ |
| Complete swarm | ✗ | ✗ | ✗ | ✓ |
| Task DAG visualization | ✗ | ✗ | ✓ | ✗ |
| Swarm timeline | ✗ | ✗ | ✓ | ✗ |

### Task

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Create task | ✓ | ✓ | ✗ | ✓ |
| List tasks | ✓ | ✓ | ✓ | ✓ |
| Complete task | ✓ | ✓ | ✗ | ✓ |
| Assign task | ✗ | ✓ | ✗ | ✓ |
| Task board (Kanban) | ✗ | ✗ | ✓ | ✗ |

### Agent

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| List agents | ✓ | ✓ | ✓ | ✓ |
| Connect agent | ✓ | ✓ | ✗ | ✓ |
| Disconnect agent | ✓ | ✓ | ✗ | ✓ |
| Agent identity | ✓ | ✗ | ✗ | ~ |
| Agent detail/info | ✓ | ✗ | ✓ | ✓ |
| Agent status (remote) | ✓ | ✗ | ✗ | ✓ |
| Agent fleet capacity | ✓ | ✗ | ~ | ~ |
| Agent audit | ✓ | ✗ | ✗ | ✗ |
| Spawn agent | ✗ | ✗ | ✓ | ✓ |
| Terminate agent | ✗ | ✗ | ✓ | ✓ |
| Agent heartbeat | ✗ | ✗ | ✗ | ✓ |
| Agent inspector | ✗ | ✗ | ✓ | ✗ |
| Evict dead agents | ✗ | ✗ | ✗ | ✓ |

### Memory

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Store memory | ✓ | ✓ | ✗ | ✓ |
| Retrieve memory | ✓ | ✓ | ✗ | ✓ |
| Search memory | ✓ | ✓ | ✗ | ✓ |
| Delete memory | ✗ | ✗ | ✗ | ✓ |
| Memory viewer (read-only) | ✗ | ✗ | ~ | ✗ |

### Inbox (ADR-060)

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Query inbox | ✓ | ✓ | ✗ | ✓ |
| Send notification | ✓ | ✓ | ✗ | ✓ |
| Acknowledge | ✓ | ✓ | ✗ | ✓ |
| Expire stale | ✓ | ✗ | ✗ | ✓ |

### Workplan

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| List workplans | ✓ | ✓ | ✓ | ✓ |
| Execute workplan | ~ | ✓ | ✓ | ✓ |
| Pause workplan | ✗ | ✓ | ✓ | ✓ |
| Resume workplan | ✗ | ✓ | ✓ | ✓ |
| Workplan report | ✓ | ✓ | ✓ | ✓ |
| Workplan history | ✓ | ✓ | ✗ | ✓ |
| Workplan status | ✓ | ✓ | ✗ | ✓ |
| Create workplan | ✗ | ✗ | ✗ | ✗ |
| Workplan schema | ✓ | ✗ | ✗ | ✗ |
| Workplan file defs | ✗ | ✗ | ✓ | ✓ |

### Enforcement

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| List rules | ✓ | ✗ | ✗ | ✓ |
| Sync rules | ✓ | ✗ | ✗ | ✓ |
| Enable/disable rule | ✓ | ✗ | ✗ | ✓ |
| Toggle mode | ✓ | ✗ | ✗ | ✓ |
| Prompt injection | ✓ | ✗ | ✗ | ✗ |

### Secrets

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Secrets status | ✓ | ✓ | ✗ | ✓ |
| Secrets has | ✓ | ✓ | ✗ | ✗ |
| Vault set | ✓ | ✗ | ✗ | ✓ |
| Vault get | ✓ | ✗ | ✗ | ✓ |
| Grant secret | ✗ | ✗ | ✗ | ✓ |
| Revoke secret | ✗ | ✗ | ✗ | ✓ |
| List grants | ✗ | ✗ | ✗ | ✓ |

### Inference

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Add provider | ✓ | ~ | ✗ | ✓ |
| List providers | ✓ | ~ | ✓ | ✓ |
| Test provider | ✓ | ~ | ✗ | ✗ |
| Discover providers | ✓ | ~ | ✗ | ✗ |
| Remove provider | ✓ | ~ | ✗ | ✓ |
| Health check | ✗ | ✗ | ✓ | ✓ |
| Inference completion | ✗ | ✗ | ✗ | ✓ |
| Cost/token monitoring | ✗ | ✗ | ✓ | ✗ |

### Sessions

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Create session | ✗ | ~ | ✓ | ✓ |
| List sessions | ~ | ~ | ✓ | ✓ |
| Session detail | ✗ | ✗ | ✓ | ✓ |
| Session messages | ✗ | ✗ | ✓ | ✓ |
| Fork session | ✗ | ✗ | ✓ | ✓ |
| Delete session | ✗ | ✗ | ✓ | ✓ |
| Compact/revert/archive | ✗ | ✗ | ✗ | ✓ |

### Test

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Run tests (unit/arch/e2e/all) | ✓ | ✗ | ✗ | ✗ |
| Test history | ✓ | ✗ | ✗ | ✓ |
| Test trends | ✓ | ✗ | ✗ | ✓ |
| Record test session | ✓ | ✗ | ✗ | ✓ |
| Flaky test detection | ✗ | ✗ | ✗ | ✓ |
| Test parity | ✓ | ✗ | ✗ | ✗ |

### Git

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Git status | ✗ | ✗ | ✓ | ✓ |
| Git log | ✗ | ✗ | ✓ | ✓ |
| Git diff | ✗ | ✗ | ✓ | ✓ |
| Git branches | ✗ | ✗ | ✓ | ✓ |
| Git worktrees | ✗ | ✗ | ✓ | ✓ |
| Commit detail | ✗ | ✗ | ✗ | ✓ |
| Task commits | ✗ | ✗ | ✗ | ✓ |
| Violation blame | ✗ | ✗ | ✗ | ✓ |
| Git timeline | ✗ | ✗ | ✗ | ✓ |

### Lifecycle (MCP-only)

| Feature | CLI | MCP | Dashboard | REST |
|---------|-----|-----|-----------|------|
| Session start | ~ | ✓ | ✗ | ✗ |
| Session heartbeat | ~ | ✓ | ✗ | ✗ |
| Workplan activate | ✗ | ✓ | ✗ | ✗ |

---

## Summary Statistics

| Metric | Value |
|--------|-------|
| Total unique features/capabilities | ~130 |
| Features with full parity (all 4 surfaces) | 7 |
| Features present in 3 surfaces | 12 |
| Features present in 2 surfaces | 28 |
| Features present in 1 surface only | 40 |
| REST endpoints with zero consumers | 60 |

### Surface Coverage

| Surface | Features Present | Approximate Coverage |
|---------|-----------------|---------------------|
| REST | ~120 | 92% |
| CLI | ~55 | 42% |
| Dashboard | ~45 | 35% |
| MCP | ~30 | 23% |

### Top 10 Gaps to Fix

| # | Gap | Impact | Effort |
|---|-----|--------|--------|
| 1 | **MCP: Inference tools not in mcp-tools.json** | Agents cannot manage inference providers | Low (add tool defs) |
| 2 | **Dashboard: No inbox panel** | Users cannot see/manage agent notifications | Medium (new component) |
| 3 | **MCP: No enforcement tools** | Agents cannot manage enforcement rules | Low (add dispatch + defs) |
| 4 | **MCP: No project management tools** | Agents cannot register/list/manage projects | Low (add dispatch + defs) |
| 5 | **CLI/MCP: No git query commands** | Must use dashboard for git insights | Medium (add CLI subcommands) |
| 6 | **Dashboard: No enforcement rules panel** | Users cannot view/toggle enforcement from dashboard | Medium (new component) |
| 7 | **MCP: No agent identity tool** | Agents cannot discover their own ID via MCP | Low (add 1 tool) |
| 8 | **CLI: No agent spawn/terminate** | Must use dashboard to spawn/kill agents | Low (add CLI subcommands) |
| 9 | **Dashboard: No memory management UI** | Memory is write-only from CLI/MCP; no CRUD in dashboard | Medium (new component) |
| 10 | **REST: Coordination API unused** | 13 endpoints with zero consumers; dead code risk | Audit (may be inter-instance only) |

### Parity by Category

| Category | Full Parity | Partial | Major Gaps |
|----------|------------|---------|------------|
| Analysis | ✓ (core) | Strict/compliance modes | Graph dashboard-only |
| ADR | ✓ (list/detail/search) | Abandoned CLI/MCP only | Edit dashboard-only |
| Swarm | ✓ (init/status) | | Complete/list-all REST-only |
| Task | ✓ (create/list/complete) | Assign MCP-only | Kanban dashboard-only |
| Agent | ~ | Many surface-specific features | Inspector dashboard-only |
| Memory | ✓ (store/get/search) | | Delete REST-only |
| Inbox | ✓ (CLI/MCP) | | Dashboard missing |
| Workplan | ✓ (list/report) | Execute/pause/resume gaps | Create not implemented |
| Enforcement | CLI+REST only | | MCP + dashboard missing |
| Secrets | CLI+REST only | | MCP + dashboard missing |
| Inference | CLI only | MCP dispatch-only | Dashboard partial |
| Git | Dashboard+REST only | | CLI + MCP missing |
| Sessions | Dashboard+REST only | | CLI + MCP missing |
| Test | CLI+REST only | | MCP + dashboard missing |
| Coordination | REST only | | All consumers missing |
| RL | REST only | | All consumers missing |
