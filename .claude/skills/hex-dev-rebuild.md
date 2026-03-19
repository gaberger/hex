---
name: hex-dev-rebuild
description: Rebuild and deploy hex-nexus binary after Rust code or asset changes
triggers:
  - rebuild hex-nexus
  - rebuild hex
  - deploy nexus
  - redeploy dashboard
---

# hex-dev-rebuild — Rebuild and Deploy hex-nexus

**Use this skill when**: You've made changes to hex-nexus Rust code or dashboard assets and need to rebuild and deploy.

## What This Does

1. Stops running hex-nexus daemon
2. Rebuilds hex-nexus in release mode
3. Copies new binary to `~/.hex/bin/hex-nexus`
4. Removes stale lock files
5. Starts daemon with new binary
6. Verifies deployment

## Workflow

### Step 1: Stop Daemon

```bash
pkill -9 hex-nexus
sleep 2
rm -f /Users/gary/.hex/daemon/hub.lock
rm -f /Users/gary/.hex/daemon/hub.state
```

### Step 2: Rebuild hex-nexus

```bash
cd hex-nexus
cargo build --release 2>&1 | grep -E "(Compiling|Finished|error|warning)"
```

If build fails, STOP and report error.

### Step 3: Copy Binary

```bash
cp hex-nexus/target/release/hex-nexus ~/.hex/bin/hex-nexus
chmod +x ~/.hex/bin/hex-nexus
```

### Step 4: Verify Binary

```bash
ls -lh ~/.hex/bin/hex-nexus
strings ~/.hex/bin/hex-nexus | grep -c "cleanup" || echo "Binary verification failed"
```

### Step 5: Start Daemon

```bash
bun run hex daemon start
```

If this times out with "Daemon failed to start within 5s", IGNORE IT and check if process is running:

```bash
ps aux | grep hex-nexus | grep -v grep
cat ~/.hex/daemon/hub.lock | jq '.'
```

If process exists and lock file has a PID, daemon IS running despite the error message.

### Step 6: Verify Deployment

```bash
sleep 2
lsof -i :5555 | grep hex-nexus
cat ~/.hex/daemon/hub.lock | jq -r '.version, .startedAt'
```

Dashboard should be available at: http://localhost:5555

## Success Criteria

- ✅ hex-nexus process running (check with `ps`)
- ✅ Lock file exists with valid PID
- ✅ Port 5555 is bound to hex-nexus
- ✅ Dashboard loads in browser

## Common Issues

### Issue: "Address already in use"
**Fix**: Go back to Step 1, ensure all processes killed

### Issue: "Daemon failed to start within 5s"
**Fix**: This is a false alarm if the process is actually running. Check `ps aux | grep hex-nexus`

### Issue: Binary file is "No such file or directory"
**Fix**: Build output is in `hex-nexus/target/release/hex-nexus` (relative to project root)

### Issue: Dashboard shows old version
**Fix**: Hard refresh browser (Cmd+Shift+R) — assets are compile-time embedded

## Why This is Needed

hex-nexus uses `rust-embed` to bake `hex-nexus/assets/*` (HTML, CSS, JS) into the Rust binary at **compile time**. This means:

1. Editing `hex-nexus/assets/index.html` requires **rebuilding the Rust binary**
2. The new binary must be **copied** to `~/.hex/bin/hex-nexus` (not automatic)
3. The daemon must be **restarted** to load the new binary
4. Browser must **hard-refresh** to bypass cache

This is different from typical web dev where HTML changes are live-reloaded.

## ARGUMENTS

No arguments required. Run with: `/hex-dev-rebuild`
