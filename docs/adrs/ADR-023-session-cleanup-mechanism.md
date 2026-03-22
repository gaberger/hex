# ADR-023: Dashboard Session Cleanup and State Synchronization

**Status:** Accepted
## Date: 2026-03-17
## Deciders: Core Team
## Related: ADR-011 (Coordination), ADR-015 (Hub SQLite), ADR-022 (Coordination Wiring)

## Context

The hex-hub dashboard shows stale coordination sessions that have been inactive for hours. Currently:
- 16+ dead sessions accumulate with PIDs that no longer exist
- All sessions show "0 agents, 0/0 tasks" regardless of actual state
- No automatic or manual cleanup mechanism exists
- Dashboard queries SQLite directly, not actual coordination state

This creates confusion about which sessions are active and wastes dashboard space.

## Decision

Implement a three-part cleanup and synchronization system:

### 1. Automatic Cleanup Cron (Rust)

Add a tokio task to hex-hub that runs every 60 seconds:
- Query all instances from SQLite
- Mark as stale if `last_heartbeat > now() - 60s`
- Remove if `stale_since > now() - 300s` (5 minutes)
- Validate PIDs: mark dead PIDs as stale immediately

### 2. Manual Cleanup API (Rust + UI)

Add endpoint: `POST /api/coordination/cleanup`
- Response: `{ "removed": N }` count of cleaned sessions
- Button in dashboard UI: "Clean Stale Sessions"

### 3. Real-Time State Sync (TypeScript)

Dashboard must query `ICoordinationPort.getInstanceStatus()` instead of SQLite:
- Actual agent count from running processes
- Actual task count from claimed tasks
- Live topology from swarm configuration

## Implementation Details

### Rust (hex-hub)

**New file**: `hex-hub/src/cleanup.rs`
```rust
pub struct CleanupService {
    db: Arc<Database>,
    interval: Duration,
}

impl CleanupService {
    pub fn spawn(db: Arc<Database>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                if let Err(e) = cleanup_stale_sessions(&db).await {
                    error!("Cleanup failed: {}", e);
                }
            }
        })
    }
}

async fn cleanup_stale_sessions(db: &Database) -> Result<usize> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs();

    let stale_threshold = now - 60; // 60s without heartbeat
    let remove_threshold = now - 360; // 6 minutes total (60s + 5min grace)

    // Mark stale
    db.mark_stale_instances(stale_threshold).await?;

    // Validate PIDs
    let instances = db.get_all_instances().await?;
    for inst in instances {
        if !is_pid_alive(inst.pid) {
            db.mark_instance_stale(&inst.id, now).await?;
        }
    }

    // Remove old stale sessions
    let removed = db.remove_stale_instances(remove_threshold).await?;

    if removed > 0 {
        info!("Cleaned up {} stale sessions", removed);
    }

    Ok(removed)
}

fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        kill(Pid::from_raw(pid as i32), Signal::SIGCONT).is_ok()
    }
    #[cfg(not(unix))]
    {
        // Windows: use sysinfo crate or always return true
        true
    }
}
```

**Modified**: `hex-hub/src/persistence.rs`
```rust
// Add fields to schema
pub struct CoordinationInstance {
    pub id: String,
    pub session_id: String,
    pub pid: u32,
    pub last_heartbeat: u64,  // Add this
    pub stale_since: Option<u64>,  // Add this
    pub agents: u32,
    pub tasks: String,
    pub topology: Option<String>,
}

// Add methods
impl Database {
    pub async fn mark_stale_instances(&self, threshold: u64) -> Result<()> {
        sqlx::query!(
            "UPDATE coordination_instances
             SET stale_since = ?
             WHERE last_heartbeat < ? AND stale_since IS NULL",
            threshold, threshold
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_stale_instances(&self, threshold: u64) -> Result<usize> {
        let result = sqlx::query!(
            "DELETE FROM coordination_instances
             WHERE stale_since < ?",
            threshold
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }
}
```

**Modified**: `hex-hub/src/routes/coordination.rs`
```rust
// Add cleanup endpoint
async fn cleanup_stale_sessions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<CleanupResponse>, StatusCode> {
    let removed = crate::cleanup::cleanup_stale_sessions(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(CleanupResponse { removed }))
}

#[derive(Serialize)]
struct CleanupResponse {
    removed: usize,
}

// Register route
Router::new()
    .route("/api/coordination/cleanup", post(cleanup_stale_sessions))
```

**Modified**: `hex-hub/assets/index.html`
```html
<!-- Add button to Instance Status section -->
<div class="cleanup-controls">
  <button id="cleanup-btn" class="cleanup-button">
    Clean Stale Sessions
  </button>
  <span id="cleanup-result" class="cleanup-result"></span>
</div>

<script>
document.getElementById('cleanup-btn').addEventListener('click', async () => {
  const btn = document.getElementById('cleanup-btn');
  const result = document.getElementById('cleanup-result');

  btn.disabled = true;
  btn.textContent = 'Cleaning...';

  try {
    const response = await fetch('/api/coordination/cleanup', {
      method: 'POST'
    });
    const data = await response.json();
    result.textContent = `Removed ${data.removed} stale sessions`;
    result.className = 'cleanup-result success';

    // Refresh dashboard
    setTimeout(() => location.reload(), 1000);
  } catch (error) {
    result.textContent = 'Cleanup failed';
    result.className = 'cleanup-result error';
  } finally {
    btn.disabled = false;
    btn.textContent = 'Clean Stale Sessions';
  }
});
</script>
```

### TypeScript Changes

**Modified**: `src/adapters/primary/dashboard-adapter.ts`
```typescript
// Change from SQLite query to ICoordinationPort query
async getInstanceStatus(): Promise<InstanceStatus[]> {
  // OLD: const instances = await this.hub.query('SELECT * FROM coordination_instances')

  // NEW: Query actual coordination state
  const instances = await this.coordinationPort.getAllInstances();

  return instances.map(inst => ({
    id: inst.id,
    session_id: inst.sessionId,
    pid: inst.pid,
    agents: inst.actualAgentCount,  // From running processes
    tasks: `${inst.claimedTasks}/${inst.totalTasks}`,
    topology: inst.topology || '—',
    last_seen: formatTimestamp(inst.lastHeartbeat),
  }));
}
```

**Verify**: `src/adapters/secondary/coordination-adapter.ts`
- Heartbeat interval is 15s (already correct, spec says 60s stale = 4 missed heartbeats)
- Ensure heartbeat actually updates `last_heartbeat` in hub

## Consequences

### Positive
- Dashboard always shows current state
- Stale sessions auto-cleanup prevents clutter
- Manual cleanup gives users immediate control
- PID validation catches crashed processes
- Real-time state sync eliminates stale data

### Negative
- Adds complexity to hex-hub (cleanup cron, PID validation)
- Cross-platform PID checking requires conditional compilation
- SQLite schema migration needed (add `last_heartbeat`, `stale_since`)

## Migration

Existing hex-hub databases need schema migration:
```sql
ALTER TABLE coordination_instances ADD COLUMN last_heartbeat INTEGER DEFAULT 0;
ALTER TABLE coordination_instances ADD COLUMN stale_since INTEGER DEFAULT NULL;
```

Auto-run on startup if columns don't exist.

## Testing

1. Start hex-hub
2. Register 2 instances with coordination-adapter
3. Kill one instance process (simulate crash)
4. Wait 60s → crashed instance marked stale
5. Wait 5 more minutes → stale instance removed
6. Click "Clean Stale Sessions" → remaining stale removed immediately
7. Verify dashboard shows accurate agent/task counts

## Alternatives Considered

**1. Manual cleanup only (no cron)**
- Rejected: Users would need to remember to click cleanup
- Stale sessions would accumulate between cleanups

**2. Aggressive cleanup (remove after 60s stale)**
- Rejected: Network hiccups or suspended VMs would lose sessions
- 5-minute grace period balances cleanup vs. safety

**3. Keep SQLite as single source of truth**
- Rejected: SQLite becomes stale between heartbeats
- Real-time query from ICoordinationPort is more accurate
