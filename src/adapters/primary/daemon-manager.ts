/**
 * Daemon Manager — persistent background dashboard service
 *
 * Implements lazy-start daemon with lock file discovery,
 * state persistence across restarts, and idle timeout.
 *
 * Lock file: ~/.hex/daemon/hub.lock
 *   { pid, port, token, startedAt, version }
 *
 * State file: ~/.hex/daemon/hub.state
 *   { registeredProjects: [{ rootPath, registeredAt }] }
 */

import { spawn, type ChildProcess } from 'node:child_process';
import {
  readFileSync,
  writeFileSync,
  unlinkSync,
  mkdirSync,
  renameSync,
  openSync,
} from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { randomBytes } from 'node:crypto';
import { request } from 'node:http';

// ─── Types ───────────────────────────────────────────────

export interface HubLockFile {
  pid: number;
  port: number;
  token: string;
  startedAt: string;
  version: string;
}

export interface HubState {
  registeredProjects: Array<{
    rootPath: string;
    registeredAt: number;
  }>;
}

export interface DaemonStatus {
  running: boolean;
  pid?: number;
  port?: number;
  uptime?: number;
  projects?: number;
}

// ─── Constants ───────────────────────────────────────────

const DAEMON_DIR = join(homedir(), '.hex', 'daemon');
const LOCK_PATH = join(DAEMON_DIR, 'hub.lock');
const STATE_PATH = join(DAEMON_DIR, 'hub.state');
const LOG_PATH = join(DAEMON_DIR, 'hub.log');
const IDLE_TIMEOUT_MS = 30 * 60 * 1000; // 30 minutes
const SPAWN_TIMEOUT_MS = 5000;
const SPAWN_POLL_MS = 200;
const STOP_TIMEOUT_MS = 3000;
const STOP_POLL_MS = 100;
const HEALTH_TIMEOUT_MS = 2000;

// ─── Helpers ─────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Resolve the runtime executable — Bun when available, otherwise Node */
function runtimeExecPath(): string {
  // Bun sets process.versions.bun; also check the argv0 path
  if (typeof process.versions.bun === 'string') {
    return process.argv0 || 'bun';
  }
  return process.execPath;
}

// ─── DaemonManager ──────────────────────────────────────

export class DaemonManager {
  /** Ensure daemon directory exists */
  private ensureDir(): void {
    mkdirSync(DAEMON_DIR, { recursive: true });
  }

  // ─── Lock File I/O ─────────────────────────────────────

  /** Read the lock file, or null if missing/corrupt */
  readLock(): HubLockFile | null {
    try {
      const raw: unknown = JSON.parse(readFileSync(LOCK_PATH, 'utf-8'));
      if (typeof raw !== 'object' || raw === null) return null;
      const lock = raw as Record<string, unknown>;
      if (typeof lock.pid !== 'number' || typeof lock.port !== 'number') return null;
      if (typeof lock.token !== 'string' || typeof lock.startedAt !== 'string') return null;
      if (typeof lock.version !== 'string') return null;
      return lock as unknown as HubLockFile;
    } catch {
      return null;
    }
  }

  /** Write lock file atomically via tmp + rename */
  private writeLock(lock: HubLockFile): void {
    this.ensureDir();
    const tmp = LOCK_PATH + '.tmp';
    writeFileSync(tmp, JSON.stringify(lock, null, 2));
    renameSync(tmp, LOCK_PATH);
  }

  /** Remove lock file (idempotent) */
  private removeLock(): void {
    try {
      unlinkSync(LOCK_PATH);
    } catch {
      // File may already be gone — no action needed
    }
  }

  // ─── Process Checks ───────────────────────────────────

  /** Check if a PID is alive via signal 0 */
  private isProcessAlive(pid: number): boolean {
    try {
      process.kill(pid, 0);
      return true;
    } catch {
      return false;
    }
  }

  /** HTTP health check against a port */
  private healthCheck(port: number): Promise<boolean> {
    return new Promise((resolve) => {
      const req = request(
        {
          hostname: '127.0.0.1',
          port,
          path: '/api/projects',
          method: 'GET',
          timeout: HEALTH_TIMEOUT_MS,
        },
        (res) => {
          res.resume();
          resolve(res.statusCode === 200);
        },
      );
      req.on('error', () => resolve(false));
      req.on('timeout', () => {
        req.destroy();
        resolve(false);
      });
      req.end();
    });
  }

  // ─── Public API ────────────────────────────────────────

  /** Get daemon status */
  async status(): Promise<DaemonStatus> {
    const lock = this.readLock();
    if (!lock) return { running: false };

    if (!this.isProcessAlive(lock.pid)) {
      this.removeLock();
      return { running: false };
    }

    const healthy = await this.healthCheck(lock.port);
    if (!healthy) {
      this.removeLock();
      return { running: false };
    }

    const uptime = Date.now() - new Date(lock.startedAt).getTime();
    const state = this.readState();

    return {
      running: true,
      pid: lock.pid,
      port: lock.port,
      uptime,
      projects: state.registeredProjects.length,
    };
  }

  /** Find running daemon or start a new one. Returns { port, token }. */
  async findOrStart(daemonEntryPath: string): Promise<{ port: number; token: string }> {
    const lock = this.readLock();
    if (lock && this.isProcessAlive(lock.pid)) {
      const healthy = await this.healthCheck(lock.port);
      if (healthy) return { port: lock.port, token: lock.token };
      // PID alive but not responding — treat as stale
    }
    this.removeLock();
    return this.spawnDaemon(daemonEntryPath);
  }

  /** Stop a running daemon gracefully */
  async stop(): Promise<boolean> {
    const lock = this.readLock();
    if (!lock) return false;

    if (this.isProcessAlive(lock.pid)) {
      try {
        process.kill(lock.pid, 'SIGTERM');
      } catch {
        // Process may have exited between check and kill
      }

      const deadline = Date.now() + STOP_TIMEOUT_MS;
      while (Date.now() < deadline) {
        await sleep(STOP_POLL_MS);
        if (!this.isProcessAlive(lock.pid)) break;
      }
    }

    this.removeLock();
    return true;
  }

  // ─── Spawn ────────────────────────────────────────────

  /** Spawn a detached daemon process */
  private async spawnDaemon(entryPath: string): Promise<{ port: number; token: string }> {
    this.ensureDir();
    const logFd = openSync(LOG_PATH, 'a');
    const token = randomBytes(16).toString('hex');
    const execPath = runtimeExecPath();

    let child: ChildProcess;
    try {
      child = spawn(execPath, [entryPath], {
        detached: true,
        stdio: ['ignore', logFd, logFd],
        env: {
          ...process.env,
          HEX_DAEMON: '1',
          HEX_DAEMON_TOKEN: token,
        },
      });
    } catch (err) {
      throw new Error(
        `Failed to spawn daemon process: ${err instanceof Error ? err.message : String(err)}`,
      );
    }

    const childPid = child.pid;
    child.unref();

    if (childPid === undefined) {
      throw new Error('Daemon process spawned but PID was undefined');
    }

    // Poll for lock file written by daemon
    const deadline = Date.now() + SPAWN_TIMEOUT_MS;
    while (Date.now() < deadline) {
      await sleep(SPAWN_POLL_MS);
      const lock = this.readLock();
      if (lock && lock.pid === childPid) {
        return { port: lock.port, token: lock.token };
      }
    }

    throw new Error(
      `Daemon failed to start within ${SPAWN_TIMEOUT_MS / 1000} seconds. Check ${LOG_PATH}`,
    );
  }

  // ─── State Persistence ────────────────────────────────

  /** Read persisted state */
  readState(): HubState {
    try {
      const raw: unknown = JSON.parse(readFileSync(STATE_PATH, 'utf-8'));
      if (typeof raw !== 'object' || raw === null) return { registeredProjects: [] };
      const state = raw as Record<string, unknown>;
      if (!Array.isArray(state.registeredProjects)) return { registeredProjects: [] };
      return raw as HubState;
    } catch {
      return { registeredProjects: [] };
    }
  }

  /** Persist current state atomically via tmp + rename */
  persistState(state: HubState): void {
    this.ensureDir();
    const tmp = STATE_PATH + '.tmp';
    writeFileSync(tmp, JSON.stringify(state, null, 2));
    renameSync(tmp, STATE_PATH);
  }

  // ─── Lock File Management (for daemon process itself) ──

  /** Called BY the daemon process on startup to register itself */
  registerSelf(port: number, token: string, version: string): void {
    this.writeLock({
      pid: process.pid,
      port,
      token,
      startedAt: new Date().toISOString(),
      version,
    });
  }

  /** Called BY the daemon process on shutdown */
  unregisterSelf(): void {
    this.removeLock();
  }

  // ─── Idle Timeout ─────────────────────────────────────

  /** Create an idle timer that calls shutdown after IDLE_TIMEOUT_MS of inactivity */
  createIdleTimer(shutdown: () => void): { reset: () => void; clear: () => void } {
    let timer: ReturnType<typeof setTimeout> = setTimeout(shutdown, IDLE_TIMEOUT_MS);
    return {
      reset: () => {
        clearTimeout(timer);
        timer = setTimeout(shutdown, IDLE_TIMEOUT_MS);
      },
      clear: () => clearTimeout(timer),
    };
  }

  /** Get filesystem paths for external use */
  get paths(): {
    daemon: string;
    lock: string;
    state: string;
    log: string;
  } {
    return { daemon: DAEMON_DIR, lock: LOCK_PATH, state: STATE_PATH, log: LOG_PATH };
  }
}
