# Dashboard Auto-Start Investigation
**Date**: 2026-03-17
**Status**: ✅ Auto-start WORKS, ⚠️ Status reporting BROKEN

---

## Finding: Dashboard IS Auto-Starting

The hex-hub daemon IS automatically starting when hex commands run:

```bash
$ node dist/cli.js analyze .
[hex] Hub running at http://127.0.0.1:5555
[hex] Project registered with dashboard hub
[dashboard-client] command listener connected
```

**Evidence**:
- Process running: PID 84861 at `/Users/gary/.hex/bin/hex-hub --daemon`
- HTTP health check: Returns 200 OK with 3 projects
- WebSocket: Connected and receiving events
- Port 5555: In use (confirmed by "Address already in use" error)

---

## Bug: Status Command Reports "Not Running"

```bash
$ node dist/cli.js daemon status
Dashboard daemon is not running.
Start with: hex daemon start
```

**Root Cause**:
- Hub IS running but **lock file doesn't exist** at `~/.hex/daemon/hub.lock`
- CLI `daemon status` checks lock file existence, not HTTP health
- Lock file is supposed to contain: `{ pid, port, token, startedAt, version }`

---

## Architecture Issue: Two Dashboard Systems

There are TWO separate dashboard implementations:

| System | Location | Status Check | Auto-Start |
|--------|----------|--------------|------------|
| **DaemonManager** | `adapters/primary/daemon-manager.ts` | Checks `~/.hex/daemon/hub.lock` | Spawns Node.js dashboard |
| **HubLauncher** | `adapters/secondary/hub-launcher.ts` | Checks HTTP `http://127.0.0.1:5555/api/projects` | Spawns Rust hex-hub binary |

**Current State**:
- ✅ `composition-root.ts` uses **HubLauncher** (Rust binary)
- ❌ CLI `hex daemon` command uses **DaemonManager** (Node.js, checks wrong lock file)
- 🔀 **Mismatch**: The CLI checks the wrong system!

---

## Why Lock File Doesn't Exist

Two possibilities:

1. **Rust binary doesn't write lock file**
   - Check `hex-hub/src/daemon.rs` for lock file logic
   - May need to add lock file writing to Rust code

2. **Lock file path mismatch**
   - HubLauncher expects: `~/.hex/daemon/hub.lock`
   - Rust binary may write to different location

---

## Fix Options

### Option A: Update CLI to use HubLauncher (✅ Recommended)

Change `cli-adapter.ts` `daemon()` method to use `HubLauncher` instead of `DaemonManager`:

```typescript
private async daemon(args: ParsedArgs): Promise<number> {
  const { HubLauncher } = await import('../secondary/hub-launcher.js');
  const launcher = new HubLauncher();

  switch (subCmd) {
    case 'status':
      const status = await launcher.status();
      // ...
  }
}
```

**Pro**: Aligns CLI with composition-root (both use HubLauncher)
**Con**: Need to update all daemon subcommands

### Option B: Fix Rust binary to write lock file

Add lock file writing logic to `hex-hub/src/daemon.rs`:

```rust
fn write_lock_file(pid: u32, token: &str) -> io::Result<()> {
    let home = std::env::var("HOME")?;
    let lock_path = format!("{}/.hex/daemon/hub.lock", home);
    let lock_data = serde_json::json!({
        "pid": pid,
        "port": 5555,
        "token": token,
        "startedAt": chrono::Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION")
    });
    std::fs::write(lock_path, serde_json::to_string_pretty(&lock_data)?)?;
    Ok(())
}
```

**Pro**: Maintains backward compatibility with existing CLI
**Con**: Duplicates logic between Rust and TypeScript

---

## Recommendation

**Option A** is better because:
1. Single source of truth (HubLauncher)
2. Rust binary uses HTTP health check (more reliable)
3. Removes dead DaemonManager code
4. Aligns CLI with composition-root behavior

---

## Test Plan

After implementing Option A:

```bash
# 1. Stop any running hub
$ pkill hex-hub

# 2. Verify status reports "not running"
$ hex daemon status
# Expected: "Dashboard daemon is not running"

# 3. Start hub
$ hex daemon start
# Expected: "Dashboard daemon started at http://localhost:5555"

# 4. Verify status reports "running"
$ hex daemon status
# Expected: "Dashboard daemon running at http://localhost:5555"

# 5. Verify auto-start still works
$ hex analyze .
# Expected: Hub already running (doesn't spawn second instance)
```

---

## Summary

- ✅ Auto-start works (Rust hex-hub spawns correctly)
- ❌ Status reporting broken (CLI checks wrong lock file)
- 🔧 Fix: Update CLI to use HubLauncher (not DaemonManager)
- 📊 Impact: Low (dashboard works, only status command affected)
