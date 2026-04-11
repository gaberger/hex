# Background Daemon Patterns for Developer Tools

Research into how popular developer tools implement always-running background services, with recommendations for hex's dashboard hub.

---

## 1. Tool-by-Tool Analysis

### 1.1 Docker Daemon (dockerd)

| Aspect | Pattern |
|--------|---------|
| **Startup** | Explicit — managed by OS init system (systemd on Linux, launchd on macOS via Docker Desktop). Not lazy-started. |
| **Persistence** | Unix socket at `/var/run/docker.sock` (Linux). Config in `/etc/docker/daemon.json` or `~/.config/docker/daemon.json` (rootless). |
| **Discovery** | Clients connect via well-known Unix socket path. `DOCKER_HOST` env var overrides. TCP socket optional. |
| **Crash recovery** | Delegated to init system (`Restart=always` in systemd unit). Docker Desktop has its own supervisor. |
| **Cleanup** | Socket file removed on clean shutdown. Stale sockets detected by attempting connection. |
| **Key insight** | Production-grade, OS-integrated approach. Overkill for dev tools — requires root/admin setup. |

### 1.2 eslint_d

| Aspect | Pattern |
|--------|---------|
| **Startup** | **Lazy on first use** — running `eslint_d lint file.js` auto-starts the daemon if not running. Explicit `eslint_d start` also available. |
| **Persistence** | Writes a `.eslint_d` file containing `{ token, port, pid, hash }` into the resolved eslint installation directory. |
| **Discovery** | Client reads the `.eslint_d` file to find port + auth token. Connects over TCP to `127.0.0.1:<port>`. |
| **Crash recovery** | Client detects stale `.eslint_d` file (PID not running), deletes it, spawns new daemon. |
| **Cleanup** | Daemon watches its own config file — if deleted, it exits. Also exits on SIGTERM. Removes config on clean shutdown. Idle timeout (default 15 min). Optional parent-PID monitoring (`ESLINT_D_PPID`). |
| **Key insight** | Best model for Node.js dev tools. Token file = discovery + auth + liveness in one file. Idle timeout prevents zombie processes. |

### 1.3 TypeScript tsserver

| Aspect | Pattern |
|--------|---------|
| **Startup** | Editor-managed — VS Code / Vim spawns `tsserver` as a child process via stdio. Not a standalone daemon. |
| **Persistence** | No persistence — lifecycle tied to editor session. Each editor window gets its own tsserver. |
| **Discovery** | stdin/stdout pipes between editor and tsserver. No socket or port. |
| **Crash recovery** | Editor detects process exit, shows notification, offers restart. Automatic restart with backoff in VS Code. |
| **Cleanup** | Process exits when editor closes or sends shutdown command. |
| **Key insight** | Parent-child model. Not suitable for hex since dashboard must survive terminal close. |

### 1.4 Turborepo Daemon

| Aspect | Pattern |
|--------|---------|
| **Startup** | **Lazy on first use** — `turbo run` auto-starts the daemon for file hashing/caching. Runs per-repository. |
| **Persistence** | gRPC server on a Unix domain socket. Socket path derived from repo root hash, stored in `~/.turbo/daemon/`. |
| **Discovery** | Socket path is deterministic from repo root. Client computes expected path and connects. |
| **Crash recovery** | Client detects dead socket, cleans up, restarts daemon. `turbo daemon clean` for manual reset. |
| **Cleanup** | Idle timeout (repo-specific). Clean shutdown removes socket file. |
| **Key insight** | Per-repo daemon with deterministic socket path is elegant. gRPC is heavier than needed for hex. |

### 1.5 Watchman (Facebook)

| Aspect | Pattern |
|--------|---------|
| **Startup** | **Lazy on first use** — any `watchman` command auto-spawns the server if not running. One daemon per user. |
| **Persistence** | Unix socket at `<STATEDIR>/<USER>` (typically `/usr/local/var/run/watchman/<USER>-state`). State file preserves watches across restarts. |
| **Discovery** | `watchman get-sockname` returns socket path. Clients should use this command for discovery rather than hardcoding paths. |
| **Crash recovery** | State file (`<USER>.state`) preserves all watches and triggers. Daemon restores them on restart. Supports systemd socket activation (`--inetd` flag). |
| **Cleanup** | `watchman shutdown-server` for clean stop. Socket and log files in STATEDIR. |
| **Key insight** | State file for durable subscriptions is valuable — hex could persist registered projects similarly. |

### 1.6 Claude Code / Cursor (IDE Tools)

| Aspect | Pattern |
|--------|---------|
| **Startup** | Embedded — runs within the editor process or as a managed child process. |
| **Persistence** | Tied to editor lifecycle. MCP servers spawned as child processes of the IDE. |
| **Discovery** | Internal IPC or stdio. MCP uses stdin/stdout transport or SSE over HTTP. |
| **Crash recovery** | IDE respawns crashed child processes. User-visible error states with retry. |
| **Cleanup** | Children terminated when IDE exits (process group kill). |
| **Key insight** | MCP's SSE transport pattern is relevant — hex already uses SSE for dashboard updates. |

### 1.7 pm2

| Aspect | Pattern |
|--------|---------|
| **Startup** | `pm2 start app.js` launches a God Daemon that supervises all managed processes. Daemon auto-starts on first pm2 command. |
| **Persistence** | PID file + dump file (`~/.pm2/dump.pm2`) stores process list. `pm2 startup` generates OS-specific init scripts (systemd, launchd, upstart). |
| **Discovery** | pm2 CLI connects to God Daemon via RPC over Unix socket (`~/.pm2/rpc.sock` and `~/.pm2/pub.sock`). |
| **Crash recovery** | God Daemon auto-restarts crashed apps (configurable). `pm2 resurrect` restores from dump file. OS init script restarts God Daemon itself. |
| **Cleanup** | `pm2 kill` stops everything. `pm2 unstartup` removes init scripts. |
| **Key insight** | `pm2 startup` pattern for generating platform-specific init scripts is the gold standard for surviving reboots. |

---

## 2. Pattern Comparison Matrix

| Pattern | Startup | Discovery | Survives Terminal | Survives Reboot | Complexity |
|---------|---------|-----------|-------------------|-----------------|------------|
| **OS init (Docker)** | Explicit systemd/launchd | Well-known socket | Yes | Yes | High (root needed) |
| **Token file (eslint_d)** | Lazy on first use | Read token file | Yes (detached) | No | Low |
| **Parent-child (tsserver)** | Editor spawns | stdio pipes | No | No | Minimal |
| **Deterministic socket (Turbo)** | Lazy on first use | Computed path | Yes (detached) | No | Medium |
| **State file + socket (Watchman)** | Lazy on first use | get-sockname cmd | Yes | With init system | Medium |
| **Supervisor (pm2)** | Explicit + lazy | RPC socket | Yes | Yes (with startup) | High |

---

## 3. Recommendation for hex Dashboard Hub

### Recommended: Hybrid eslint_d + Watchman Pattern

The best fit for hex is a **lazy-start daemon with a lock file and optional OS integration**, combining the simplicity of eslint_d with Watchman's state persistence.

### 3.1 Architecture

```
~/.hex/
  daemon/
    hub.lock          # { pid, port, token, startedAt }
    hub.state         # { registeredProjects: [...] }
    hub.log           # Daemon log output (rotated)
```

### 3.2 Lifecycle

#### Starting (Lazy on First Use)

```
hex dashboard
  |
  +-- Read ~/.hex/daemon/hub.lock
  |     |
  |     +-- File exists? Check if PID alive (process.kill(pid, 0))
  |     |     |
  |     |     +-- Alive + port responds? --> Print URL, exit
  |     |     +-- Stale? --> Delete lock, continue to spawn
  |     |
  |     +-- No file? --> Continue to spawn
  |
  +-- Spawn daemon:
        child_process.spawn(process.execPath, ['daemon-entry.js'], {
          detached: true,
          stdio: ['ignore', logFd, logFd],
          env: { ...process.env, HEX_DAEMON: '1' }
        });
        child.unref();
```

Key details:
- `detached: true` + `child.unref()` lets the daemon survive the parent terminal session
- Daemon listens on port 0 (OS-assigned), writes actual port to lock file
- Security token generated with `crypto.randomBytes(8).toString('hex')`
- Lock file written atomically (write to temp, rename)

#### Discovery

```typescript
interface HubLockFile {
  pid: number;
  port: number;
  token: string;
  startedAt: string;  // ISO timestamp
  version: string;    // hex version for compatibility check
}
```

Client reads `~/.hex/daemon/hub.lock`, validates PID is alive, connects to `http://localhost:<port>`. Token passed as `Authorization: Bearer <token>` for mutations.

#### Crash Recovery

```typescript
// In CLI client, before connecting:
function findOrStartHub(): { port: number; token: string } {
  const lock = readLockFile();
  if (lock && isProcessAlive(lock.pid)) {
    // Verify HTTP health endpoint responds
    if (await healthCheck(lock.port)) return lock;
  }
  // Stale or missing — clean up and restart
  removeLockFile();
  return spawnDaemon();
}
```

#### Idle Shutdown

```typescript
// In daemon process:
const IDLE_TIMEOUT = 30 * 60 * 1000; // 30 minutes, longer than eslint_d's 15
let idleTimer = setTimeout(shutdown, IDLE_TIMEOUT);

// Reset on any API request
function resetIdle() {
  clearTimeout(idleTimer);
  idleTimer = setTimeout(shutdown, IDLE_TIMEOUT);
}
```

#### State Persistence (from Watchman)

```typescript
// On project register/unregister, persist to hub.state
function persistState() {
  const state = {
    registeredProjects: Array.from(projects.values()).map(p => ({
      rootPath: p.ctx.rootPath,
      registeredAt: p.registeredAt,
    })),
  };
  writeFileSync(STATE_PATH + '.tmp', JSON.stringify(state));
  renameSync(STATE_PATH + '.tmp', STATE_PATH);
}

// On daemon startup, restore previous registrations
function restoreState() {
  try {
    const state = JSON.parse(readFileSync(STATE_PATH, 'utf-8'));
    for (const proj of state.registeredProjects) {
      if (existsSync(proj.rootPath)) {
        registerProject(proj.rootPath);
      }
    }
  } catch { /* fresh start */ }
}
```

#### Clean Shutdown

```typescript
function shutdown() {
  // 1. Close all file watchers
  for (const slot of projects.values()) {
    for (const w of slot.watchers) w.close();
    for (const t of slot.debounceTimers.values()) clearTimeout(t);
  }
  // 2. End all SSE connections
  for (const client of sseClients) {
    clearInterval(client.heartbeat);
    client.res.end();
  }
  // 3. Close HTTP server
  server.close(() => {
    // 4. Remove lock file (but keep state file for next start)
    unlinkSync(LOCK_PATH);
    process.exit(0);
  });
}

// Signal handlers
process.on('SIGTERM', shutdown);
process.on('SIGINT', shutdown);

// Watch lock file for deletion (eslint_d pattern)
fs.watch(LOCK_PATH, { persistent: false }).on('change', (type) => {
  if (type === 'rename') shutdown();  // lock file deleted = stop
});
```

### 3.3 Optional: Surviving Reboots (pm2 Pattern)

For users who want the dashboard to auto-start on login:

```bash
# macOS (launchd)
hex daemon install
# Generates ~/Library/LaunchAgents/com.hex.dashboard.plist

# Linux (systemd user unit)
hex daemon install
# Generates ~/.config/systemd/user/hex-dashboard.service

# Remove
hex daemon uninstall
```

This should be opt-in, not default. Most developers do not want dev tools starting on boot.

### 3.4 CLI Commands

```bash
hex dashboard          # Open dashboard (lazy-starts daemon)
hex daemon status      # Show PID, port, uptime, registered projects
hex daemon stop        # Graceful shutdown
hex daemon restart     # Stop + start
hex daemon logs        # Tail ~/.hex/daemon/hub.log
hex daemon install     # Install OS startup script (opt-in)
hex daemon uninstall   # Remove OS startup script
```

### 3.5 Why This Pattern

| Requirement | How It's Met |
|-------------|--------------|
| "Just works" without manual management | Lazy start on `hex dashboard` |
| Multiple projects register/unregister | Multi-project hub with state persistence |
| Survives terminal close | `detached: true` + `child.unref()` |
| macOS + Linux support | Node.js detached process works everywhere; optional launchd/systemd for reboot survival |
| No zombie processes | Idle timeout (30 min) + lock file watch + signal handlers |
| Crash recovery | Client validates PID + health check, auto-restarts if stale |
| Security | Auth token in lock file, localhost-only binding, CORS validation |

### 3.6 What NOT to Do

1. **Do not use pm2 as a runtime dependency** — it is 20MB+ and overkill for a single daemon.
2. **Do not use Unix domain sockets** — HTTP on localhost is simpler, browser-accessible for the dashboard UI, and works on Windows.
3. **Do not require explicit `daemon start`** — lazy start is the developer-friendly pattern every modern tool uses.
4. **Do not bind to a fixed port** — use port 0 and write the assigned port to the lock file. Fixed ports cause conflicts across tools.
5. **Do not store lock files in the project directory** — use `~/.hex/daemon/` so one daemon serves all projects.

---

## 4. Implementation Priority

1. **Phase 1**: Lock file + lazy start + idle timeout (covers 90% of use cases)
2. **Phase 2**: State persistence for registered projects across daemon restarts
3. **Phase 3**: Optional `daemon install` for launchd/systemd integration
4. **Phase 4**: Log rotation and `daemon logs` command
