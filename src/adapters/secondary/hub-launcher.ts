/**
 * Hub Launcher — Secondary adapter for managing the hex-hub Rust binary lifecycle
 *
 * Finds, starts, stops, and checks the status of the hex-hub daemon process.
 * This replaces the Node.js DashboardHub with the Rust hex-hub binary.
 */

import { spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import type { IHubLauncherPort } from '../../core/ports/hub-launcher.js';

// ─── Constants ───────────────────────────────────────────

export const HUB_PORT = 5555;
const HEALTH_TIMEOUT_MS = 2000;
const SPAWN_TIMEOUT_MS = 5000;
const SPAWN_POLL_MS = 100;

const HUB_BINARY_PATHS = [
  join(homedir(), '.hex', 'bin', 'hex-hub'),
  join(process.cwd(), 'target', 'release', 'hex-hub'),       // workspace target
  join(process.cwd(), 'hex-hub', 'target', 'release', 'hex-hub'),
  join(process.cwd(), 'target', 'debug', 'hex-hub'),
  join(process.cwd(), 'hex-hub', 'target', 'debug', 'hex-hub'),
];

// ─── HubLauncher ─────────────────────────────────────────

export class HubLauncher implements IHubLauncherPort {
  /** Find the hex-hub binary on disk */
  findBinary(): string | null {
    for (const p of HUB_BINARY_PATHS) {
      if (existsSync(p)) return p;
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

  /** Start hub as a background daemon. Returns true if started, false if already running. */
  async start(token?: string): Promise<{ started: boolean; url: string }> {
    if (await this.isRunning()) {
      return { started: false, url: `http://127.0.0.1:${HUB_PORT}` };
    }

    const binary = this.findBinary();
    if (!binary) {
      throw new Error(
        'hex-hub binary not found. Run "hex setup" to install it, or build from hex-hub/ with "cargo build --release".',
      );
    }

    const args = ['--daemon'];
    if (token) args.push('--token', token);

    const child = spawn(binary, args, {
      detached: true,
      stdio: 'ignore',
      env: { ...process.env },
    });
    child.unref();

    // Wait for hub to become healthy
    const deadline = Date.now() + SPAWN_TIMEOUT_MS;
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, SPAWN_POLL_MS));
      if (await this.isRunning()) {
        return { started: true, url: `http://127.0.0.1:${HUB_PORT}` };
      }
    }
    throw new Error('hex-hub started but did not become healthy within 5 seconds');
  }

  /** Stop the running hub by reading the lock file PID and sending SIGTERM */
  async stop(): Promise<boolean> {
    const lockPath = join(homedir(), '.hex', 'daemon', 'hub.lock');
    try {
      const raw: unknown = JSON.parse(readFileSync(lockPath, 'utf-8'));
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
