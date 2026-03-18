# Multi-Instance Coordination Testing

How to test the coordination session cleanup feature (ADR-023).

## Quick Test (2 minutes)

```bash
# 1. Start hex-hub daemon
hex daemon start

# 2. Run quick test suite
bun scripts/test-coordination.ts
```

This tests:
- ✅ Normal heartbeat (instance stays alive)
- ✅ Manual cleanup endpoint
- ✅ Coordination state (locks, claims, activities)

**Skips slow tests** (stale detection, dead PID) which require 60s+ waits.

## Full Test Suite (5+ minutes)

To test stale session detection and dead PID cleanup, uncomment the slow tests in `scripts/test-coordination.ts`:

```typescript
await testStaleSession();  // Waits 70s
await testDeadPID();        // Waits 65s
```

## Manual Browser Testing

1. **Start daemon and open dashboard**:
   ```bash
   hex daemon start
   # Open http://localhost:5555
   ```

2. **Register multiple instances**:
   ```bash
   # Terminal 1
   cd /path/to/project1
   hex analyze .  # Registers instance

   # Terminal 2
   cd /path/to/project2
   hex analyze .  # Registers another instance
   ```

3. **View coordination state**:
   - Dashboard → Instance Status section
   - Should see multiple registered instances
   - Each shows PID, agent count, task count

4. **Test manual cleanup**:
   - Click "🗑 Clean Stale Sessions" button
   - Should see "✓ Removed N stale sessions" message
   - Dashboard auto-refreshes

5. **Test stale detection**:
   - Kill a terminal process (Ctrl+C)
   - Wait 60 seconds (stale threshold)
   - Wait 5 more minutes (removal threshold)
   - Instance should disappear from dashboard

6. **Test dead PID**:
   - Kill a process: `kill -9 <PID>`
   - Wait for cleanup cron (runs every 60s)
   - Dead PID instance should be removed immediately

## Test Scenarios

### Scenario 1: Normal Operation

```typescript
// Register instance
POST /api/coordination/instance/register
{
  "project_id": "my-project",
  "pid": 12345,
  "session_label": "main-session"
}

// Heartbeat every 30s
POST /api/coordination/instance/heartbeat
{
  "instance_id": "uuid",
  "project_id": "my-project",
  "agent_count": 3,
  "active_task_count": 5
}

// Instance stays registered ✅
```

### Scenario 2: Stale Session

```typescript
// Register instance
POST /api/coordination/instance/register { ... }

// NO heartbeat for 60+ seconds ⏳

// After 5 minutes total:
// Automatic cleanup removes instance ✅
```

### Scenario 3: Dead PID

```typescript
// Register instance with PID 99999
POST /api/coordination/instance/register
{
  "pid": 99999,  // This process doesn't exist
  ...
}

// Cleanup cron detects dead PID (libc::kill returns error)
// Instance removed immediately ✅
```

### Scenario 4: Manual Cleanup

```typescript
// Register 3 instances, only heartbeat 1

// Click cleanup button in UI
POST /api/coordination/cleanup

// Response: { "removed": 2 }
// Active instance kept, stale removed ✅
```

## Verification

Check coordination state via REST API:

```bash
# List all instances
curl http://localhost:5555/api/coordination/instances?projectId=my-project

# List locks
curl http://localhost:5555/api/coordination/worktree/locks?projectId=my-project

# List task claims
curl http://localhost:5555/api/coordination/tasks?projectId=my-project

# List activities
curl http://localhost:5555/api/coordination/activities?projectId=my-project&limit=10
```

## Expected Behavior

### Automatic Cleanup (Cron)

- **Runs every**: 60 seconds
- **Stale threshold**: 60 seconds (no heartbeat)
- **Removal threshold**: 5 minutes (total)
- **Dead PID**: Immediate removal
- **Logs**: Check `hex-hub/logs/` for cleanup events

### Manual Cleanup (Button)

- **Endpoint**: `POST /api/coordination/cleanup`
- **Returns**: `{ "removed": N }`
- **Effect**: Removes all stale sessions immediately
- **Active sessions**: Kept (last heartbeat < 60s ago)

### PID Validation

- **Unix**: Uses `libc::kill(pid, 0)` - returns 0 if alive, error if dead
- **Windows**: Always returns true (heartbeat timeout catches it)
- **Dead PID**: Marked stale immediately on next cron run

## Troubleshooting

### Instances not showing in dashboard

- Check daemon is running: `hex daemon status`
- Check logs: `tail -f ~/.hex/daemon/hub.log`
- Verify registration: `curl http://localhost:5555/api/coordination/instances`

### Cleanup not working

- Check cleanup cron is running (should see logs every 60s)
- Verify stale threshold: last_seen must be > 60s ago
- Verify removal threshold: registered_at must be > 6 minutes ago
- Check PID validation: `ps -p <PID>` should fail for dead processes

### Manual cleanup button not visible

- Hard refresh browser (Cmd+Shift+R) — assets are compile-time embedded
- Verify hex-hub build includes button: `strings ~/.hex/bin/hex-hub | grep cleanup`
- Check commit: Button added in commit 64d421d

## Behavioral Specs

See `docs/specs/dashboard-session-cleanup.json` for full behavioral specifications.

Key specs:
- **spec-1**: Sessions marked stale after 60s no heartbeat
- **spec-2**: Stale sessions removed after 5 minutes
- **spec-3**: Dead PIDs marked stale immediately
- **spec-5**: Manual cleanup button works
- **spec-7**: Active sessions NOT cleaned up

## Performance

- **Cleanup cron overhead**: ~10ms per run (in-memory scan)
- **PID validation overhead**: ~0.1ms per instance (libc call)
- **Max instances**: No hard limit (tested with 100+)
- **Memory**: ~1KB per instance (in SharedState)
