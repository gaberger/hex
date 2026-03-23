# ADR-049: Embedded Settings Template — Single Source of Truth

**Status:** Accepted
## Date: 2026-03-21

## Context

When `hex init` bootstraps a project, it writes `.claude/settings.json` with hooks, permissions, statusline, and announcements. Prior to this change, **two independent sources defined the same configuration**:

1. **`hex-setup/mcp/hex-claude-settings.json`** — the versioned template in the hex-setup package, intended to be the canonical definition
2. **`hex-cli/src/commands/init.rs` → `create_claude_settings()`** — a hardcoded inline `serde_json::json!()` block that duplicated every hook, permission, and announcement

These two sources had already drifted: the hex-setup template included `hex swarm status` in its `companyAnnouncements` string, while `init.rs` did not. Any future change to hooks (new events, timeout adjustments, new matchers) required updating **both** files and hoping a reviewer caught the discrepancy.

Additionally, the `session_start` hook handler (`hex hook session-start`) only printed a status banner — it did not register the Claude Code session as an agent with hex-nexus, despite ADR-048 specifying this behavior. The `session_end` handler was a no-op.

## Decision

### 1. Embed the template at compile time

Replace the inline JSON in `init.rs` with `include_str!()` pointing to the hex-setup template:

```rust
const SETTINGS_TEMPLATE: &str =
    include_str!("../../../hex-setup/mcp/hex-claude-settings.json");
```

`create_claude_settings()` now parses this embedded template and merges its fields (`hooks`, `statusline`, `companyAnnouncements`) into the target project's settings. User-customized `permissions` are preserved (only set if absent).

This makes `hex-setup/mcp/hex-claude-settings.json` the **single source of truth** — editing it automatically updates the next `cargo build` of hex-cli.

### 2. Implement session agent lifecycle (ADR-048)

The `hex hook session-start` handler now:

1. Checks nexus health and reports **SpacetimeDB connectivity** status in the banner
2. Calls `POST /api/agents/connect` with session metadata (`hostname`, `model`, `session_id`, `project_dir`)
3. Persists the returned `agentId` to `~/.hex/sessions/agent-{sessionId}.json`
4. Prints registration confirmation in the banner

The `hex hook session-end` handler now:

1. Reads the persisted `agentId` from `~/.hex/sessions/`
2. Calls `POST /api/agents/disconnect` to deregister
3. Deletes the state file

Both operations are **fire-and-forget** — failures are silently ignored to never block session lifecycle.

### Session Banner (connected state)

```
⬡  hex — my-project
  ───────────────────────────────────────
  Project: my-project (026df98f)
  Nexus:   connected
  StDB:    connected
  Agent:   registered (claude-a1b2c3d4)
  Arch:    run `hex analyze .` to check health
```

## Consequences

### Positive

- **No more drift** — hook definitions, timeouts, matchers, and announcements are defined once in `hex-setup/` and automatically embedded into the CLI binary
- **Full fleet visibility** — Claude Code sessions now appear on the hex dashboard as registered agents (completing ADR-048 implementation)
- **SpacetimeDB awareness** — developers see at session start whether SpacetimeDB is connected or the nexus is falling back to SQLite
- **Clean session lifecycle** — sessions register on start, deregister on end, with disk-persisted state bridging the gap between hook invocations
- **Backwards-compatible** — `hex init` still merges into existing settings without clobbering user customizations

### Negative

- **Compile-time coupling** — `hex-cli` now has a `include_str!` path dependency on `hex-setup/mcp/hex-claude-settings.json`. Moving or renaming that file will break the build (which is the desired behavior — it forces the rename to be intentional)
- **No heartbeat** — sessions register once but don't send periodic heartbeats (same limitation noted in ADR-048). Crashed sessions rely on the stale-agent cleanup timer (120s)

## Files Changed

| File | Change |
|------|--------|
| `hex-cli/src/commands/init.rs` | Replaced inline JSON with `include_str!()` from hex-setup template |
| `hex-cli/src/commands/hook.rs` | Implemented `register_session_agent()` and `deregister_session_agent()` per ADR-048; enhanced `session_start` with SpacetimeDB status reporting |
| `hex-setup/mcp/hex-claude-settings.json` | No change — already canonical (this ADR makes it official) |
| `.claude/settings.json` | Synced `companyAnnouncements` to match template |
