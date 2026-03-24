# Workplan Priorities — 2026-03-22

> **UPDATE 2026-03-22**: Workplan reconciliation complete.
> - Tiers 1-4 of feat-adr-gap-closure verified as done (2026-03-21)
> - feat-agent-notification-inbox steps 1-4, 6-7 delivered in 480c7a5; steps 5, 8-10 remain
> - feat-remote-agent-transport + feat-remote-agent-spawn marked superseded
> - 3 markdown workplans converted to JSON (aiide-remaining, aiide-phase2, pencil-implementation)
> - CI/CD cross-compilation identified as upcoming need

Prioritized from the workplan review. Ordered by: unblocks-other-work first, then impact, then effort.

## Tier 1: Active In-Progress Work (finish first)

### 1. Agent notification inbox remaining (feat-agent-notification-inbox.json)
- **Status:** in_progress — steps 1-4, 6-7 done (480c7a5)
- **Remaining:** step-5 (notification producers — binary update detector, config sync hook), step-8 (state preservation on restart), step-9 (idle agent escalation), step-10 (integration tests)
- **Effort:** Medium (4 steps, ~2000 LOC)
- **Unblocks:** Reliable agent restart coordination, binary update propagation

### 2. ADR gap closure tier 5 (feat-adr-gap-closure.json)
- **Status:** in_progress — tiers 1-4 complete
- **Remaining:** Integration tests, documentation (glossary, system-architecture)
- **Effort:** Small (test suites + 2 docs)
- **Unblocks:** Confidence for future refactors

## Tier 2: Dashboard & UX (high visibility)

### 3. Pencil design implementation (feat-pencil-implementation.json)
- **Status:** in_progress — Control Plane partial, 5 of 6 phases not started
- **Why:** Dashboard is the primary developer surface. 14 of 15 tasks remain.
- **Effort:** Large (6 phases, all tier-2 primary adapters — fully parallelizable by swarm)
- **Focus first:** Project Detail (P1) and ADR Browser (P2) — highest user-facing impact
- **Design tokens:** bg #0a0e14, cards #111827, accent #22d3ee, layer colors documented in workplan

### 4. AIIDE Phase 2 config sync (feat-aiide-phase2.json)
- **Status:** in_progress — P0 partially done (hex init, config_sync.rs, production build)
- **Why:** Dashboard doesn't read from SpacetimeDB subscriptions yet. This bridges "edit in repo, see in dashboard."
- **Effort:** Large (22 tasks across 5 phases)
- **Note:** Overlaps with gap-closure P6 (skill lifecycle). P0.2-P0.4 (SpacetimeDB tables + bindings) should come first.

### 5. AIIDE remaining tasks (feat-aiide-remaining.json)
- **Status:** in_progress — P0 complete, P1 largely done
- **Remaining:** P1.1 (real worktree data), P1.4 (health auto-fetch), P2 (discovery layer), P3 (polish)
- **Effort:** Medium (10 tasks)
- **Note:** Some overlap with aiide-phase2 (file tree, worktree API). Do together.

## Tier 3: Agent System (enables remote/distributed work)

### 6. Remote agent remaining work (feat-remote-agent-remaining.json)
- **Status:** planned — transport (a4f2799) and spawn (225a710) landed but remaining work not started
- **Why:** Full remote agent lifecycle: monitoring, reconnection, multi-host fleet management
- **Effort:** Large
- **Depends on:** Tier 1-2 stability

### 7. SpacetimeDB hydration (feat-stdb-hydrate.json)
- **Status:** planned
- **Why:** New hex installs need automated WASM module deployment. Currently manual.
- **Effort:** Medium
- **Unblocks:** Onboarding new developers/machines

## Tier 4: Can Defer

### 8. Secure secret distribution (feat-secure-secret-distribution.json)
- **Status:** planned — complex 4-tier plan, current secrets work fine
- **When:** After remote agent transport is stable

### 9. hex-desktop (feat-hex-desktop.json)
- **Status:** planned — Tauri wrapper, dashboard works fine in browser
- **When:** After pencil designs implemented and dashboard stable

### 10. ADR-035: Architecture V2 — Rust-first migration
- **Status:** planned — ambitious 32-step plan to retire TypeScript
- **When:** After SpacetimeDB fully authoritative and agent system stable

### 11. CI/CD cross-compilation
- **Status:** upcoming — need GitHub Actions for macOS ARM64 + Linux x86_64
- **When:** Before first external release

## Consolidation Status

| Action | Status |
|--------|--------|
| ~~outstanding-adrs-workplan.json → feat-adr-gap-closure.json~~ | Superset exists |
| ~~feat-remote-agent-transport.json → superseded~~ | Done 2026-03-22 |
| ~~feat-remote-agent-spawn.json → superseded~~ | Done 2026-03-22 |
| ~~aiide-remaining.md → JSON~~ | Done 2026-03-22 → feat-aiide-remaining.json |
| ~~aiide-phase2.md → JSON~~ | Done 2026-03-22 → feat-aiide-phase2.json |
| ~~pencil-implementation.md → JSON~~ | Done 2026-03-22 → feat-pencil-implementation.json |
| feat-mcp-tools-sync-and-workplan-dashboard.json → feat-adr-gap-closure P7 | Pending |

## Quick Wins (< 1 hour each)

- [ ] Mark outstanding-adrs-workplan.json as superseded by feat-adr-gap-closure.json
- [ ] Mark feat-mcp-tools-sync-and-workplan-dashboard.json as superseded by feat-adr-gap-closure P7
- [ ] Create docs/reference/glossary.md (gap-closure P9.1 — standalone, no deps)
- [ ] Create docs/reference/system-architecture.md (gap-closure P9.2 — standalone)
- [x] Mark feat-remote-agent-transport.json as superseded (done 2026-03-22)
- [x] Mark feat-remote-agent-spawn.json as superseded (done 2026-03-22)
- [x] Convert aiide-remaining.md to JSON (done 2026-03-22)
- [x] Convert aiide-phase2.md to JSON (done 2026-03-22)
- [x] Convert pencil-implementation.md to JSON (done 2026-03-22)
- [x] Reconcile feat-agent-notification-inbox status (done 2026-03-22)
