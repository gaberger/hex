# ADR-016: Hub Binary Version Verification

## Status: Accepted
## Date

2026-03-17

## Context

hex-hub is a compiled Rust binary that serves the dashboard UI and coordination API. It is launched by the TypeScript `HubLauncher` adapter, which searches for the binary on disk and spawns it as a background daemon on port 5555.

Three compounding issues caused repeated "wrong version" incidents where the running hub served stale UI or missed new API endpoints:

1. **Workspace target mismatch**: hex-hub lives in a Cargo workspace. `cargo build --release` outputs to `<workspace-root>/target/release/`, not `hex-hub/target/release/`. The launcher's search paths listed the local member target first, consistently finding a stale binary from a previous non-workspace build.

2. **No build identity**: The only version identifier was `CARGO_PKG_VERSION` (e.g., `26.3.1`), which stays constant across rebuilds. Two binaries compiled from different source states were indistinguishable.

3. **Blind connection**: `ensureHubRunning()` checked `isRunning()` via a health ping to `/api/projects`. If any hub was listening on port 5555 — even one from a previous session with completely different code — the launcher connected to it without question.

## Decision

### Compile-time build hash

A `build.rs` script runs `git rev-parse --short HEAD` and sets `HEX_HUB_BUILD_HASH` as a compile-time environment variable. This hash is:

- Returned by `/api/version` as `{ version, buildHash, name }`
- Written to the daemon lock file (`~/.hex/daemon/hub.lock`) as `build_hash`
- Printed by `hex-hub --build-hash` (no daemon startup) and `hex-hub --version`

### Version verification on connect

`HubLauncher.start()` no longer blindly reuses a running hub. The sequence is:

1. Check if hub is running on port 5555
2. If running, fetch `/api/version` to get `runningBuildHash`
3. Query the installed binary via `execFileSync(binary, ['--build-hash'])` to get `installedBuildHash`
4. If hashes differ: stop the stale hub, wait for port release, start the correct binary
5. If hashes match (or either is unavailable): connect as before

### Canonical install location

`~/.hex/bin/hex-hub` is the intended canonical binary path. The search order in `findBinary()` is:

1. `~/.hex/bin/hex-hub` (canonical install)
2. `<cwd>/target/release/hex-hub` (workspace root)
3. `<cwd>/hex-hub/target/release/hex-hub` (member local)
4. Debug variants of the above

## Consequences

### Positive

- **Correct hub guaranteed**: Stale binaries are automatically replaced — no manual `kill` + restart cycle
- **Transparent diagnostics**: `hex-hub --version` shows `hex-hub 26.3.1 (65f466e)` — version + commit in one line
- **CI-friendly**: The build hash can be checked in CI to verify deployed binaries match expected commits
- **Lock file forensics**: `~/.hex/daemon/hub.lock` now records which exact build is running

### Negative

- **Startup latency**: Version check adds one HTTP request (`/api/version`) and one `execFileSync` call (~50ms) to every `start()`
- **Git dependency in build**: `build.rs` requires git in the build environment. Falls back to `"unknown"` if unavailable
- **Rebuild triggers**: `build.rs` watches `.git/HEAD` and `.git/index`, which may cause unnecessary recompilations when switching branches

### Risks

- If the hub binary doesn't support `--build-hash` (pre-ADR-016 binary), `getInstalledBuildHash()` returns null and the version check is skipped — graceful degradation
- If `/api/version` doesn't return `buildHash` (pre-ADR-016 hub running), same graceful degradation

## Alternatives Considered

1. **File mtime comparison**: Compare binary modification time against source. Rejected — unreliable across file systems and doesn't survive `cp`.
2. **Always restart**: Kill any running hub on every `hex dashboard`. Rejected — disruptive to other connected sessions.
3. **Semantic versioning only**: Bump `CARGO_PKG_VERSION` on every change. Rejected — requires manual discipline and doesn't catch uncommitted builds.
