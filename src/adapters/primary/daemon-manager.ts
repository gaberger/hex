/**
 * Daemon Manager — manages the dashboard hub as a background service
 *
 * Simplified for fixed-port model: always uses port 5555.
 * Lock file at ~/.hex/daemon/hub.lock tracks the running process.
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
const HUB_PORT = 5555;

// ─── Types ───────────────────────────────────────────────

interface HubLockFile {
  pid: number;
  port: number;
  token: string;
  startedAt: string;
  version: string;
}

interface DaemonStatus {
  running: boolean;
  pid?: number;
  port?: number;
  uptime?: number;
}

// ─── Constants ───────────────────────────────────────────

const DAEMON_DIR = join(homedir(), '.hex', 'daemon');
const LOCK_PATH = join(DAEMON_DIR, 'hub.lock');
const LOG_PATH = join(DAEMON_DIR, 'hub.log');
const SPAWN_TIMEOUT_MS = 5000;
const SPAWN_POLL_MS = 200;
const STOP_TIMEOUT_MS = 3000;
const STOP_POLL_MS = 100;
const HEALTH_TIMEOUT_MS = 2000;

// ─── Helpers ─────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function runtimeExecPath(): string {
  if (typeof process.versions.bun === 'string') {
    return process.argv0 || 'bun';
  }
  return process.execPath;
}

// ─── DaemonManager ──────────────────────────────────────

export class DaemonManager {
  private ensureDir(): void {
    mkdirSync(DAEMON_DIR, { recursive: true });
  }

  // ─── Lock File ─────────────────────────────────────────

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

  private writeLock(lock: HubLockFile): void {
    this.ensureDir();
    const tmp = LOCK_PATH + '.tmp';
    writeFileSync(tmp, JSON.stringify(lock, null, 2));
    renameSync(tmp, LOCK_PATH);
  }

  private removeLock(): void {
    try { unlinkSync(LOCK_PATH); } catch { /* already gone */ }
  }

  // ─── Process Checks ───────────────────────────────────

  private isProcessAlive(pid: number): boolean {
    try { process.kill(pid, 0); return true; } catch { return false; }
  }

  private healthCheck(port: number): Promise<boolean> {
    return new Promise((resolve) => {
      const req = request(
        { hostname: '127.0.0.1', port, path: '/api/projects', method: 'GET', timeout: HEALTH_TIMEOUT_MS },
        (res) => { res.resume(); resolve(res.statusCode === 200); },
      );
      req.on('error', () => resolve(false));
      req.on('timeout', () => { req.destroy(); resolve(false); });
      req.end();
    });
  }

  // ─── Public API ────────────────────────────────────────

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

    return {
      running: true,
      pid: lock.pid,
      port: lock.port,
      uptime: Date.now() - new Date(lock.startedAt).getTime(),
    };
  }

  /** Check if hub is running. If not, start it. Returns { port, token }. */
  async findOrStart(daemonEntryPath: string): Promise<{ port: number; token: string }> {
    // Quick check: can we reach the hub?
    const healthy = await this.healthCheck(HUB_PORT);
    if (healthy) {
      const lock = this.readLock();
      return { port: HUB_PORT, token: lock?.token ?? '' };
    }

    // Not running — clean up stale lock and start
    this.removeLock();
    return this.spawnDaemon(daemonEntryPath);
  }

  async stop(): Promise<boolean> {
    const lock = this.readLock();
    if (!lock) return false;

    if (this.isProcessAlive(lock.pid)) {
      try { process.kill(lock.pid, 'SIGTERM'); } catch { /* may have exited */ }
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

  private async spawnDaemon(entryPath: string): Promise<{ port: number; token: string }> {
    this.ensureDir();
    const logFd = openSync(LOG_PATH, 'a');
    const token = randomBytes(16).toString('hex');

    let child: ChildProcess;
    try {
      child = spawn(runtimeExecPath(), [entryPath, 'hub', '--daemon'], {
        detached: true,
        stdio: ['ignore', logFd, logFd],
        env: { ...process.env, HEX_DAEMON: '1', HEX_DAEMON_TOKEN: token },
      });
    } catch (err) {
      throw new Error(`Failed to spawn daemon: ${err instanceof Error ? err.message : String(err)}`);
    }

    const childPid = child.pid;
    child.unref();
    if (childPid === undefined) throw new Error('Daemon PID undefined');

    const deadline = Date.now() + SPAWN_TIMEOUT_MS;
    while (Date.now() < deadline) {
      await sleep(SPAWN_POLL_MS);
      const lock = this.readLock();
      if (lock && lock.pid === childPid) {
        return { port: lock.port, token: lock.token };
      }
    }

    throw new Error(`Daemon failed to start within ${SPAWN_TIMEOUT_MS / 1000}s. Check ${LOG_PATH}`);
  }

  // ─── Self-registration (called by daemon process) ──────

  registerSelf(token: string, version: string): void {
    this.writeLock({
      pid: process.pid,
      port: HUB_PORT,
      token,
      startedAt: new Date().toISOString(),
      version,
    });
  }

  unregisterSelf(): void {
    this.removeLock();
  }

  get paths(): { daemon: string; lock: string; log: string } {
    return { daemon: DAEMON_DIR, lock: LOCK_PATH, log: LOG_PATH };
  }
}
