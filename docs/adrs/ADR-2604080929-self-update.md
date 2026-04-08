# ADR-2604080929: hex Self-Update

**Status:** Proposed
**Date:** 2026-04-08
**Drivers:** Gary Berger

## Context

The current hex upgrade path is:

```bash
curl -fsSL https://raw.githubusercontent.com/gaberger/hex/main/scripts/install.sh | bash
```

This works but has three problems:

1. **Friction**: requires knowing the install URL and running it manually
2. **Process kill**: `install.sh` kills all running hex processes (nexus, CLI) before
   replacing the binary — this disrupts active agent sessions
3. **No in-context path**: when Claude Code detects a new version is available, it
   can't trigger an update without the user running an external script

The goal is `hex self-update` — a single command that downloads, verifies, and
replaces the hex binary without killing nexus if possible.

**Decision type:** `add`

## Decision

### Command

```
hex self-update [--check] [--version <tag>] [--yes]
```

| Flag | Behavior |
|------|----------|
| *(none)* | Check, prompt, then update to latest |
| `--check` | Print available version and exit (no update) |
| `--version <tag>` | Pin to a specific release tag |
| `--yes` | Skip confirmation prompt (for scripted use) |

### Version Check

Call the GitHub releases API (no auth required for public repos):

```
GET https://api.github.com/repos/gaberger/hex/releases/latest
```

Extract `tag_name` (e.g., `v26.4.30`). Compare against `hex --version` output.
If already up to date, print confirmation and exit 0.

### Download + Verify

1. Determine platform: `{arch}-{os}` (e.g., `x86_64-unknown-linux-gnu`,
   `aarch64-apple-darwin`)
2. Download asset: `hex-{version}-{platform}.tar.gz` from the release
3. Download `SHA256SUMS.txt` from the same release
4. Verify SHA256 of the downloaded tarball against `SHA256SUMS.txt`
5. Extract binary to a temp path alongside the current binary

### Atomic Replace

On macOS/Linux, the running binary can be replaced while it is executing because
the OS holds an open file descriptor to the original inode (unlink-safe):

```
mv ~/.local/bin/hex.new ~/.local/bin/hex
```

This is an atomic rename on the same filesystem. The running process is unaffected.
nexus does NOT need to be killed — it loaded its code at startup.

**Exception**: if `hex-nexus` is the binary being replaced (same binary as `hex-cli`
on some builds), nexus should be gracefully restarted post-update rather than killed.

### Post-Update

After the binary is replaced:
1. Run `hex --version` with the new binary to confirm the update succeeded
2. If nexus is running and the update includes a nexus version change, print:
   `"Restart nexus to activate: hex nexus start"`
3. Do NOT auto-restart nexus — let the user decide

### MCP Tool

Add `hex_self_update` MCP tool:
- `--check` maps to `{ "check_only": true }` — returns `{ current, latest, up_to_date }`
- Full update maps to `{ "yes": true }` — runs update non-interactively

### Nexus Version Notification (ADR-060)

When nexus starts, it can compare its own version against the latest GitHub release
(non-blocking, best-effort HTTP call on startup). If a newer version exists, send
an inbox notification (priority 1 = low) to all connected agents:

```json
{
  "type": "update_available",
  "current": "26.4.28",
  "latest": "26.4.30",
  "message": "hex v26.4.30 available — run hex self-update"
}
```

This surfaces the update suggestion in the Claude Code statusline without interrupting work.

## Impact Analysis

### Affected Files

| File | Change | Impact |
|------|--------|--------|
| `hex-cli/src/commands/` | New `update.rs` subcommand | LOW — additive |
| `hex-cli/src/main.rs` | Register `self-update` subcommand | LOW |
| `hex-cli/src/commands/mcp.rs` | Add `hex_self_update` MCP tool | LOW |
| `hex-nexus/src/lib.rs` | Optional startup version check → inbox notify | LOW |
| `scripts/install.sh` | Can simplify — delegate to `hex self-update --yes` after first install | LOW |

### Build Verification Gates

| Gate | Command |
|------|---------|
| Workspace compile | `cargo check --workspace` |
| CLI subcommand present | `hex self-update --help` |
| MCP tool present | `hex mcp list \| grep hex_self_update` |

## Consequences

**Positive:**
- Single command upgrade path from terminal and Claude Code
- Atomic replace — nexus session survives the binary swap
- Version check surfaced in agent inbox on startup (non-blocking)
- `--check` flag enables CI/CD scripts to test for new versions

**Negative:**
- Adds a GitHub API dependency (rate-limited at 60 req/hr unauthenticated)
- SHA256 verification adds complexity but is required for security

**Mitigations:**
- GitHub API call is best-effort; if it fails, self-update prints an error and exits 0
- Rate limit is never a problem in practice (one check per install, not per request)

## Implementation

| Phase | Description | Validation Gate | Status |
|-------|-------------|-----------------|--------|
| P0 | Add `hex-cli/src/commands/update.rs` with version check + download + atomic replace | `cargo check -p hex-cli` + `hex self-update --check` | Proposed |
| P1 | Register `self-update` subcommand in `main.rs` | `hex self-update --help` | Proposed |
| P2 | Add `hex_self_update` MCP tool to `mcp.rs` | `hex mcp list \| grep hex_self_update` | Proposed |
| P3 | Add startup version check in `hex-nexus/src/lib.rs` → inbox notify | `cargo check -p hex-nexus` | Proposed |

## References

- `scripts/install.sh` — current manual install path (kills processes, replaces binary)
- ADR-017: macOS inode/sigkill cache — related to binary replacement on macOS
- ADR-060: Agent notification inbox — used for version-available notifications
- ADR-2604081320: Claude Code context detection — `hex_self_update` MCP tool follows same pattern
- GitHub releases API: `https://api.github.com/repos/gaberger/hex/releases/latest`
