# ADR-2604131945: Brain as Self-Consistency Daemon

**Status:** Proposed
**Date:** 2026-04-13
**Drivers:** Every bug caught this session was a deterministic invariant violation that hex already had tools to detect — but nothing ran those tools automatically. Brain should be hex's continuous self-consistency layer, not just an RL model selector.

## Context

Today's session exposed 5 classes of bug that hex's existing tools could have caught automatically:

| Bug | Tool that catches it | Why it wasn't caught |
|-----|---------------------|---------------------|
| Unwired `brief.rs` (no mod.rs + main.rs entry) | File scan: every `commands/*.rs` must have mod.rs + Commands enum entry | No automated scan |
| Stale workplan status (3/4 tasks "todo" but code exists) | `hex plan reconcile` | Never run automatically |
| Stale release binary (debug has fixes, release doesn't) | Timestamp comparison: binary mtime vs HEAD commit time | No automated check |
| Dropped code in worktree merge | `hex worktree merge --verify` integrity check | Manual merge bypassed hex |
| Brief events duplicated across projects | Test: `hex brief` response size < threshold | No regression test |

None of these require inference. They are all **deterministic checks** that should run continuously.

## Decision

### 1. Brain gains a `validate` subcommand

```bash
hex brain validate          # Run all consistency checks
hex brain validate --watch  # Run on every commit (hook-driven)
```

### 2. Consistency checks (code-first, no inference)

| Check | Command | When |
|-------|---------|------|
| **CLI module wiring** | Scan `commands/*.rs`, verify each has `pub mod` in `mod.rs` + variant in `Commands` enum | Post-commit, post-merge |
| **MCP-CLI parity** | Compare MCP tool list with CLI subcommands (ADR-019) | Post-commit |
| **Workplan reconciliation** | `hex plan reconcile` all active workplans | Session start, post-merge |
| **Binary freshness** | Compare `target/release/hex` mtime with HEAD commit time | Post-commit, pre-`hex` invocation |
| **Worktree integrity** | For any pending worktree merges, verify all agent lines present | Post-merge |
| **Architecture grade** | `hex analyze .` must return A+ | Post-commit gate |
| **Test suite** | `cargo test --workspace` must pass | Post-merge gate |

### 3. Hook integration

Brain validate runs via hooks:
- **`post-commit`** → binary freshness + CLI wiring + architecture grade
- **`session-start`** → workplan reconciliation + worktree status
- **`post-merge`** (new) → full validation suite

### 4. Brain validate output

```
⬡ hex brain validate

  CLI wiring:      ✓ 40/40 modules registered
  MCP-CLI parity:  ✓ 28/28 tools have CLI equivalents
  Workplans:       ✓ 3 active, all reconciled
  Binary:          ✗ STALE — release binary 2h behind HEAD (rebuilding...)
  Worktrees:       ✓ 0 pending merges
  Architecture:    ✓ A+ (100/100)
  Tests:           ✓ 271/271 passing

  1 issue found — auto-fixing: binary rebuild
```

### 5. Auto-fix for safe operations

Brain SHALL auto-fix issues that are safe and deterministic:
- Stale binary → `cargo build --release` in background
- Stale workplan status → `hex plan reconcile`
- Stale worktrees → `hex worktree cleanup`

Brain SHALL NOT auto-fix issues that require judgment:
- Unwired modules → report, don't guess the enum variant
- Dropped merge code → report, don't auto-merge
- Failing tests → report, don't modify code

## Consequences

**Positive:**
- Every deterministic bug from this session would have been caught automatically
- Brain becomes genuinely useful beyond RL model selection
- Developers get a single command to verify system health
- Auto-fix eliminates the "stale binary" class of bug entirely

**Negative:**
- Post-commit hook adds ~5s (architecture check is the bottleneck)
- Brain validate on session-start adds ~10s

**Mitigations:**
- Architecture check runs in background (non-blocking)
- Binary freshness check is instant (stat comparison)
- CLI wiring check is instant (file scan)

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | CLI wiring check: scan commands/*.rs vs mod.rs + main.rs | Pending |
| P2 | Binary freshness check with auto-rebuild | Pending |
| P3 | Hook integration: post-commit + session-start | Pending |
| P4 | Workplan auto-reconciliation on session start | Pending |
| P5 | MCP-CLI parity check (ADR-019) | Pending |
| P6 | `hex brain validate` unified command | Pending |
| P7 | Auto-fix for safe operations | Pending |

## References

- ADR-031: RL-Driven Model Selection (existing brain functionality)
- ADR-019: CLI-MCP Parity
- ADR-2604131800: Last-Mile Self-Hosting Gaps
- ADR-2604131930: First-Class Worktree Lifecycle
- Session 2026-04-13: 5 classes of deterministic bugs caught manually
