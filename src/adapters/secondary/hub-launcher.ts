/**
 * Hub Launcher — Secondary adapter for managing the hex-hub Rust binary lifecycle
 *
 * Finds, starts, stops, and checks the status of the hex-hub daemon process.
 * This replaces the Node.js DashboardHub with the Rust hex-hub binary.
 */

import { execFileSync, spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

// ─── Constants ───────────────────────────────────────────

export const HUB_PORT = 5555;
const HEALTH_TIMEOUT_MS = 2000;
const SPAWN_TIMEOUT_MS = 5000;
const SPAWN_POLL_MS = 100;

// ─── Dependency injection for testability ────────────────

/** Injectable dependencies — production defaults come from Node built-ins. */
export interface HubLauncherDeps {
  existsSync: (path: string) => boolean;
  readFileSync: (path: string, encoding: string) => string;
  spawn: (cmd: string, args: string[], opts: Record<string, unknown>) => { unref: () => void; pid?: number };
  homedir: () => string;
  join: (...parts: string[]) => string;
}

const defaultDeps: HubLauncherDeps = {
  existsSync,
  readFileSync: readFileSync as (path: string, encoding: string) => string,
  spawn: spawn as HubLauncherDeps['spawn'],
  homedir,
  join,
};

// ─── HubLauncher ─────────────────────────────────────────

export class HubLauncher {
  private readonly deps: HubLauncherDeps;

  constructor(deps?: HubLauncherDeps) {
    this.deps = deps ?? defaultDeps;
  }
  /** Find the hex-hub binary on disk */
  findBinary(): string | null {
    const binaryPaths = [
      this.deps.join(this.deps.homedir(), '.hex', 'bin', 'hex-hub'),
      this.deps.join(process.cwd(), 'target', 'release', 'hex-hub'),       // workspace root target
      this.deps.join(process.cwd(), 'hex-hub', 'target', 'release', 'hex-hub'),
      this.deps.join(process.cwd(), 'target', 'debug', 'hex-hub'),
      this.deps.join(process.cwd(), 'hex-hub', 'target', 'debug', 'hex-hub'),
    ];
    for (const p of binaryPaths) {
      if (this.deps.existsSync(p)) return p;
    }
    return null;
  }

  /** Check if hub is already listening on the expected port */
  async isRunning(): Promise<boolean> {
    try {
      const res = await fetch(`http://127.0.0.1:${HUB_PORT}/api/projects`, {
        signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
      });
      return res.ok;
    } catch {
      return false;
    }
  }

  /** Get the build hash of the running hub via /api/version */
  async getRunningBuildHash(): Promise<string | null> {
    try {
      const res = await fetch(`http://127.0.0.1:${HUB_PORT}/api/version`, {
        signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
      });
      if (!res.ok) return null;
      const data = (await res.json()) as { buildHash?: string };
      return data.buildHash ?? null;
    } catch {
      return null;
    }
  }

  /** Get the build hash baked into the installed binary via --build-hash flag */
  getInstalledBuildHash(): string | null {
    const binary = this.findBinary();
    if (!binary) return null;
    try {
      const out = execFileSync(binary, ['--build-hash'], { timeout: 3000, encoding: 'utf-8' });
      return out.trim() || null;
    } catch {
      return null;
    }
  }

  /**
   * Start hub as a background daemon.
   * If a hub is already running but its build hash doesn't match the installed
   * binary, it is stopped and replaced with the correct version.
   */
  async start(token?: string): Promise<{ started: boolean; url: string }> {
    if (await this.isRunning()) {
      const runningHash = await this.getRunningBuildHash();
      const installedHash = this.getInstalledBuildHash();

      if (runningHash && installedHash && runningHash !== installedHash) {
        process.stderr.write(
          `[hex] Hub version mismatch: running=${runningHash}, installed=${installedHash}. Restarting…\n`,
        );
        await this.stop();
        const stopDeadline = Date.now() + SPAWN_TIMEOUT_MS;
        while (Date.now() < stopDeadline) {
          await new Promise((r) => setTimeout(r, SPAWN_POLL_MS));
          if (!(await this.isRunning())) break;
        }
      } else {
        return { started: false, url: `http://127.0.0.1:${HUB_PORT}` };
      }
    }

    const binary = this.findBinary();
    if (!binary) {
      throw new Error(
        'hex-hub binary not found. Run "hex setup" to install it, or build from hex-hub/ with "cargo build --release".',
      );
    }

    const args = ['--daemon'];
    if (token) args.push('--token', token);

    const child = this.deps.spawn(binary, args, {
      detached: true,
      stdio: 'ignore',
      env: { ...process.env },
    });
    child.unref();

    // Wait for hub to become healthy AND lock file to exist.
    // The lock file is critical — without it, clients can't read the auth token
    // and all POST requests will fail with 401.
    const lockPath = this.deps.join(this.deps.homedir(), '.hex', 'daemon', 'hub.lock');
    const deadline = Date.now() + SPAWN_TIMEOUT_MS;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, SPAWN_POLL_MS));
      if (await this.isRunning() && this.deps.existsSync(lockPath)) {
        return { started: true, url: `http://127.0.0.1:${HUB_PORT}` };
      }
    }
    // Hub may be healthy but lock file missing (race condition from ADR-016 restart)
    if (await this.isRunning()) {
      process.stderr.write(
        '[hex] Warning: hub is running but lock file is missing — clients may fail auth\n',
      );
      return { started: true, url: `http://127.0.0.1:${HUB_PORT}` };
    }
    throw new Error('hex-hub started but did not become healthy within 5 seconds');
  }

  /** Stop the running hub by reading the lock file PID and sending SIGTERM */
  async stop(): Promise<boolean> {
    const lockPath = this.deps.join(this.deps.homedir(), '.hex', 'daemon', 'hub.lock');
    try {
      const raw: unknown = JSON.parse(this.deps.readFileSync(lockPath, 'utf-8'));
      if (typeof raw === 'object' && raw !== null && 'pid' in raw) {
        const lock = raw as Record<string, unknown>;
        if (typeof lock.pid === 'number') {
          process.kill(lock.pid, 'SIGTERM');
          return true;
        }
      }
    } catch { /* no lock file or invalid */ }

    return false;
  }

  /** Get hub status */
  async status(): Promise<{ running: boolean; url: string | null; projects: number }> {
    if (!(await this.isRunning())) {
      return { running: false, url: null, projects: 0 };
    }
    try {
      const res = await fetch(`http://127.0.0.1:${HUB_PORT}/api/projects`, {
        signal: AbortSignal.timeout(HEALTH_TIMEOUT_MS),
      });
      const data = (await res.json()) as { projects: unknown[] };
      return {
        running: true,
        url: `http://127.0.0.1:${HUB_PORT}`,
        projects: data.projects?.length ?? 0,
      };
    } catch {
      return { running: false, url: null, projects: 0 };
    }
  }
}

/** Ensure hub is running — start if not. Used by composition-root and CLI commands. */
export async function ensureHubRunning(): Promise<string> {
  const launcher = new HubLauncher();
  const { url } = await launcher.start();
  return url;
}
