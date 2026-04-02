# ADR-2604012110: Hooks-First Architecture Enforcement

**Status:** Accepted
**Date:** 2026-04-01
**Supersedes:** (none — extends ADR-002, ADR-018)

---

## Context

A 3-agent investigation (docs/analysis/hooks-investigation-*.md) evaluated whether hex's architecture enforcement layer could be migrated from the hex-agent Rust daemon + hex-nexus REST calls to Claude Code lifecycle hooks.

**Finding:** Architecture enforcement is session-local and event-driven — it maps perfectly to Claude Code hooks. SpacetimeDB is only genuinely required for distributed state (swarm coordination, cross-host agent visibility, inference routing, secret grants).

**Current state:** Enforcement runs inside hex-agent via `ConstraintEnforcer` (163 lines) that calls hex-nexus at port 5555. This requires a running daemon for basic arch checking, which is heavy for solo development workflows.

---

## Decision

Implement architecture enforcement as **Claude Code lifecycle hooks** that invoke the existing `hex analyze` and `hex enforce` CLI binaries directly — no daemon required for local enforcement.

SpacetimeDB is retained for its irreducible multi-agent/multi-host responsibilities (hexflo-coordination, agent-registry, inference-gateway, secret-grant).

---

## Implementation: Three Hooks

### 1. PreToolUse — Path + Layer Validation
```json
{
  "matcher": "Write|Edit|MultiEdit",
  "hooks": [{
    "type": "command",
    "command": "hex enforce check-file \"$TOOL_INPUT_PATH\"",
    "blocking": true,
    "timeout": 5000
  }]
}
```
Blocks writes to forbidden paths and wrong hex layers. Reads `.hex/adr-rules.toml` locally — no daemon.

### 2. PostToolUse — Treesitter Boundary Check
```json
{
  "matcher": "Write|Edit|MultiEdit",
  "hooks": [{
    "type": "command",
    "command": "hex analyze --file \"$TOOL_INPUT_PATH\" --quiet",
    "blocking": false,
    "timeout": 10000
  }]
}
```
Runs hex's treesitter analysis after every edit. Non-blocking (warns, doesn't stop).

### 3. Stop — Session-Exit Gate
```json
{
  "hooks": [{
    "type": "command",
    "command": "hex analyze --violations-only --exit-code",
    "blocking": true,
    "timeout": 30000
  }]
}
```
Fails the session if architecture violations exist at session end.

---

## New CLI Command: `hex enforce check-file <path>`

A thin subcommand added to `hex-cli/src/commands/enforce.rs` that:
1. Reads `.hex/adr-rules.toml` (or `~/.hex/adr-rules.toml` as fallback)
2. Checks the given path against `forbidden_paths` and `hex_layer` rules
3. Exits 0 (allow) or 1 (block) — Claude Code interprets exit code for blocking hooks

No network calls, no daemon dependency.

---

## What This Replaces

| Replaced | By |
|---|---|
| `ConstraintEnforcer` (163 lines) called from hex-nexus REST | `hex enforce check-file` (~50 lines, local) |
| `spacetime_hook.rs` PreToolUse path checking | Hook in `settings.json` |
| Daemon required for any enforcement | Zero-daemon for local solo workflows |

## What Is NOT Replaced

- `hex swarm`, `hex task`, `hex agent worker` — remain SpacetimeDB-backed
- `stdb_inference.rs` — no non-STDB fallback exists yet; do not touch
- Secret grant distribution — network service required

---

## Consequences

**Positive:**
- Architecture enforcement works without a running daemon
- ~80% of enforcement value with 3 hooks + ~50-line CLI addition
- Composable: hooks call existing CLI binaries; no new logic to test
- Rollback is operational (re-enable nexus), not architectural

**Negative:**
- `hex enforce check-file` is a new CLI subcommand to maintain
- Remote multi-agent enforcement still requires nexus (hooks are session-local)
- Hook performance adds ~20–100ms per Write tool call (shell startup)

---

## Related

- `docs/analysis/hooks-investigation-recommendation.md` — full investigation synthesis
- `docs/analysis/hooks-investigation-risk.md` — risk assessment + rollback paths
- `docs/analysis/hooks-prototype/` — working prototype scripts
- ADR-002: Hexagonal Architecture Boundaries
- ADR-018: Architecture Enforcement Runtime
