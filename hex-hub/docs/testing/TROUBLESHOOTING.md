# Hex-Hub Troubleshooting

Common issues and solutions when working with hex-hub.

## Binary Killed Immediately (Exit 137)

**Symptom**: hex-hub starts but is immediately killed with exit code 137 (SIGKILL)

```bash
~/.hex/bin/hex-hub --daemon
# Process exits immediately with no output
echo $?  # Returns 137
```

**Causes**:
1. **macOS Gatekeeper** - Unsigned binary blocked by security policy
2. **Antivirus software** - Binary flagged and killed
3. **System policy** - Corporate/MDM policy blocking execution

**Solutions**:

### 1. Check macOS Gatekeeper

```bash
# Remove quarantine attribute
xattr -d com.apple.quarantine ~/.hex/bin/hex-hub

# OR allow the binary in System Settings
# System Settings → Privacy & Security → Allow hex-hub
```

###  2. Build Locally (Bypasses Code Signing)

```bash
cd hex-hub
cargo build --release
# Use target/release/hex-hub directly instead of installing
target/release/hex-hub --daemon
```

### 3. Check Console Logs

```bash
# View system logs for security events
log show --predicate 'eventMessage contains "hex-hub"' --last 5m
```

### 4. Run in Foreground (Debug Mode)

```bash
# Run without --daemon flag to see errors
~/.hex/bin/hex-hub --port 5555
```

## Daemon "Failed to Start Within 5s"

**Symptom**: `hex daemon start` reports timeout but process is actually running

```bash
hex daemon start
# Error: Daemon failed to start within 5s
ps aux | grep hex-hub  # But process exists!
```

**Cause**: False alarm - the daemon IS running, but the health check times out.

**Solution**: Ignore the error and verify manually:

```bash
ps aux | grep hex-hub
cat ~/.hex/daemon/hub.lock
curl http://localhost:5555/api/version
```

## Coordination Endpoints Return 404

**Symptom**: Tests fail with "POST /api/coordination/instance/register failed: 404"

**Cause**: Running an old hex-hub binary without coordination endpoints.

**Solution**: Rebuild and reinstall hex-hub:

```bash
cd hex-hub
cargo build --release
pkill -9 hex-hub
rm -f ~/.hex/daemon/hub.lock
cp target/release/hex-hub ~/.hex/bin/hex-hub
chmod +x ~/.hex/bin/hex-hub
hex daemon start
```

## Port 5555 Already in Use

**Symptom**: `Failed to bind: Os { code: 48, kind: AddrInUse }`

**Solution**:

```bash
# Find process using port 5555
lsof -i :5555

# Kill it
pkill -9 hex-hub

# Remove lock file
rm -f ~/.hex/daemon/hub.lock

# Restart
hex daemon start
```

## Cleanup Button Not Visible in Dashboard

**Symptom**: Dashboard loads but cleanup button is missing

**Cause**: Browser cache serving old HTML

**Solution**:

```bash
# Hard refresh browser
# Chrome/Firefox: Cmd+Shift+R
# Safari: Cmd+Option+R
```

## Tests Fail: "hex-hub is not running"

**Symptom**: `bun scripts/test-coordination.ts` reports hub not running

**Fix**:

```bash
# 1. Verify daemon is actually running
ps aux | grep hex-hub

# 2. Check it's listening on port 5555
lsof -i :5555

# 3. Test health endpoint
curl http://localhost:5555/api/version

# 4. If all pass, the test script has a bug (check fetch timeout)
```

## Automatic Cleanup Not Running

**Symptom**: Stale sessions remain after 5+ minutes

**Debug**:

```bash
# Check cleanup cron logs (should see logs every 60s)
tail -f ~/.hex/daemon/hub.log | grep -i cleanup

# Manually trigger cleanup
curl -X POST http://localhost:5555/api/coordination/cleanup

# Check if instances are actually stale
curl http://localhost:5555/api/coordination/instances | jq '.[] | {id, last_seen}'
```

## Dead Code Warnings in Rust Build

**Symptom**: Warnings about unused functions (`evict_stale`, `complete_task`)

**Status**: These are intentional - kept for future use or referenced by other parts of the system.

**Ignore**: These warnings don't affect functionality.
