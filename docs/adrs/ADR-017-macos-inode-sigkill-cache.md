# ADR-017: Unlink Binary Before Copy to Avoid macOS Inode-Based SIGKILL Cache

## Status

Accepted

## Date

2026-03-17

## Context

When hex installs or updates the `hex-hub` binary via `hex setup`, it uses `copyFileSync()` to overwrite the existing binary at `~/.hex/bin/hex-hub`. On Linux this works reliably. On macOS, it causes the newly copied binary to be killed immediately on launch with no error message.

### Root cause

macOS kernel maintains a per-inode cache of process termination decisions. When a running `hex-hub` process is killed with `SIGKILL` (e.g., during daemon restart or `hex setup` replacing a stale instance), the kernel records that inode as "should be killed."

`copyFileSync()` overwrites the file **in place**, preserving the original inode number. When the new binary is launched, the kernel sees the same inode that was previously SIGKILL'd and immediately terminates the new process — even though the file contents are completely different.

### Symptoms observed

- `hex-hub` daemon fails to start after `hex setup` with no error output
- Process exits with signal 9 (SIGKILL) before executing any user code
- Works correctly on first install (no prior inode history)
- Works correctly on Linux (no inode-based kill cache)
- Extremely difficult to debug — no logs, no crash report, no codesign error

### Discovery path

The issue was traced by:
1. Observing that a freshly compiled binary worked when copied to a **new** path
2. Observing that the same binary failed when copied to the **existing** path
3. Confirming the inode number was preserved across `copyFileSync()`
4. Confirming `unlinkSync()` + `copyFileSync()` produced a new inode and resolved the issue

## Decision

**Always `unlinkSync()` the destination binary before `copyFileSync()`** in all code paths that install or update `hex-hub`.

```typescript
// CORRECT — new inode, avoids macOS kill cache
try { unlinkSync(hexBinDest); } catch { /* not present */ }
copyFileSync(prebuilt, hexBinDest);
chmodSync(hexBinDest, 0o755);

// WRONG — preserves inode, macOS may kill the new binary
copyFileSync(prebuilt, hexBinDest);
```

### Affected code paths

All three install paths in `cli-adapter.ts` (`hex setup`):
1. Pre-built binary found in package — copy to `~/.hex/bin/`
2. Cargo build from source — copy release binary to `~/.hex/bin/`
3. Binary found via CWD search paths — copy to `~/.hex/bin/`

## Consequences

### Positive

- Hub binary reliably starts after update on macOS
- Fix is minimal (one `unlinkSync` + try/catch per code path)
- No behavioral change on Linux — `unlinkSync` of a non-running binary is harmless
- Eliminates a class of "impossible" bugs that produce no diagnostic output

### Negative

- Brief window where no binary exists at the destination path (between unlink and copy)
- If the process crashes between `unlinkSync` and `copyFileSync`, the binary is gone (user must re-run `hex setup`)

### Platform notes

- **macOS**: This is the only platform where the issue is observed. The kernel's inode-based kill cache is undocumented but reproducible across macOS 13–15.
- **Linux**: `copyFileSync` overwrite works fine — Linux does not cache kill decisions by inode.
- **Windows**: Not applicable — hex-hub does not support Windows.

## Alternatives Considered

1. **`renameSync()` old binary before copy** — Avoids the brief gap but leaves orphan files that need cleanup
2. **Copy to temp path, then `renameSync()` into place** — Atomic but adds complexity; `renameSync` across filesystems falls back to copy anyway
3. **Use `execve()` from a wrapper** — Would bypass the cache but adds a launcher binary
4. **Codesign the binary** — Does not help; the kill cache is orthogonal to code signing
