/**
 * Coordination Adapter
 *
 * Implements ICoordinationPort via HTTP calls to hex-hub's coordination endpoints.
 * Manages instance registration, heartbeat with unstaged file tracking,
 * worktree locking, task claiming, and activity publishing.
 */

import { request } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';
import type {
  ICoordinationPort,
  InstanceSwarmState,
  UnstagedFile,
  WorktreeLock,
  LockResult,
  TaskClaim,
  ClaimResult,
  ActivityEntry,
  InstanceInfo,
  UnstagedState,
} from '../../core/ports/coordination.js';

const execFileP = promisify(execFile);

/**
 * Injectable dependencies for CoordinationAdapter.
 * Production code uses defaults; tests can inject fakes to avoid mock.module().
 */
export interface CoordinationAdapterDeps {
  httpRequest?: typeof request;
  execFileAsync?: (cmd: string, args: string[], opts: { cwd: string; timeout: number }) => Promise<{ stdout: string; stderr: string }>;
  authToken?: string;
}

/** Read auth token from hub lock file. */
function readHubToken(): string {
  try {
    const lockPath = join(homedir(), '.hex', 'daemon', 'hub.lock');
    const lock = JSON.parse(readFileSync(lockPath, 'utf-8'));
    return lock.token ?? '';
  } catch {
    return '';
  }
}

/** Classify a file path into a hex layer. */
function classifyLayer(filePath: string): string {
  if (filePath.includes('/core/domain/') || filePath.includes('core/domain/')) return 'domain';
  if (filePath.includes('/core/ports/') || filePath.includes('core/ports/')) return 'port';
  if (filePath.includes('/core/usecases/') || filePath.includes('core/usecases/')) return 'usecase';
  if (filePath.includes('/adapters/primary/') || filePath.includes('adapters/primary/')) return 'primary-adapter';
  if (filePath.includes('/adapters/secondary/') || filePath.includes('adapters/secondary/')) return 'secondary-adapter';
  return 'other';
}

export class CoordinationAdapter implements ICoordinationPort {
  private instanceId: string | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private readonly authToken: string;
  private readonly _httpRequest: typeof request;
  private readonly _execFileAsync: (cmd: string, args: string[], opts: { cwd: string; timeout: number }) => Promise<{ stdout: string; stderr: string }>;

  constructor(
    private readonly projectId: string,
    private readonly projectPath: string,
    private readonly hubPort: number = 5555,
    deps?: CoordinationAdapterDeps,
  ) {
    this.authToken = deps?.authToken ?? readHubToken();
    this._httpRequest = deps?.httpRequest ?? request;
    this._execFileAsync = deps?.execFileAsync ?? ((cmd, args, opts) => execFileP(cmd, args, opts));
  }

  // ── Instance lifecycle ────────────────────────────────

  async registerInstance(sessionLabel?: string): Promise<string> {
    const result = await this.post('/api/coordination/instance/register', {
      projectId: this.projectId,
      pid: process.pid,
      sessionLabel: sessionLabel ?? `session-${Date.now()}`,
    });
    if (!result?.instanceId) throw new Error('Failed to register coordination instance');
    this.instanceId = result.instanceId as string;

    // Start heartbeat every 15s
    this.startHeartbeat();

    return this.instanceId;
  }

  async heartbeat(unstagedFiles?: UnstagedFile[], swarmState?: InstanceSwarmState): Promise<void> {
    if (!this.instanceId) return;

    // If no files provided, capture current git state
    const files = unstagedFiles ?? await this.captureUnstagedFiles();

    await this.post('/api/coordination/instance/heartbeat', {
      instanceId: this.instanceId,
      projectId: this.projectId,
      unstagedFiles: files,
      ...(swarmState ? {
        agentCount: swarmState.agentCount,
        activeTaskCount: swarmState.activeTaskCount,
        completedTaskCount: swarmState.completedTaskCount,
        topology: swarmState.topology,
      } : {}),
    });
  }

  // ── Worktree locks ────────────────────────────────────

  async acquireLock(feature: string, layer: string): Promise<LockResult> {
    if (!this.instanceId) throw new Error('Instance not registered');
    const result = await this.post('/api/coordination/worktree/lock', {
      instanceId: this.instanceId,
      projectId: this.projectId,
      feature,
      layer,
    });
    return (result ?? { acquired: false, lock: null, conflict: null }) as unknown as LockResult;
  }

  async releaseLock(feature: string, layer: string): Promise<void> {
    const key = encodeURIComponent(`${this.projectId}:${feature}:${layer}`);
    await this.del(`/api/coordination/worktree/lock/${key}`);
  }

  async listLocks(): Promise<WorktreeLock[]> {
    const result = await this.get(`/api/coordination/worktree/locks?projectId=${encodeURIComponent(this.projectId)}`);
    return (result ?? []) as unknown as WorktreeLock[];
  }

  // ── Task ownership ────────────────────────────────────

  async claimTask(taskId: string): Promise<ClaimResult> {
    if (!this.instanceId) throw new Error('Instance not registered');
    const result = await this.post('/api/coordination/task/claim', {
      instanceId: this.instanceId,
      projectId: this.projectId,
      taskId,
    });
    return (result ?? { claimed: false, claim: null, conflict: null }) as unknown as ClaimResult;
  }

  async releaseTask(taskId: string): Promise<void> {
    await this.del(`/api/coordination/task/claim/${encodeURIComponent(taskId)}`);
  }

  async listClaims(): Promise<TaskClaim[]> {
    const result = await this.get(`/api/coordination/task/claims?projectId=${encodeURIComponent(this.projectId)}`);
    return (result ?? []) as unknown as TaskClaim[];
  }

  // ── Activity stream ───────────────────────────────────

  async publishActivity(action: string, details?: Record<string, unknown>): Promise<void> {
    if (!this.instanceId) return;
    await this.post('/api/coordination/activity', {
      instanceId: this.instanceId,
      projectId: this.projectId,
      action,
      details: details ?? {},
    });
  }

  async getActivities(limit?: number): Promise<ActivityEntry[]> {
    const params = new URLSearchParams({ projectId: this.projectId });
    if (limit) params.set('limit', String(limit));
    const result = await this.get(`/api/coordination/activities?${params}`);
    return (result ?? []) as unknown as ActivityEntry[];
  }

  // ── Unstaged tracking ─────────────────────────────────

  async getUnstagedAcrossInstances(): Promise<UnstagedState[]> {
    const result = await this.get(`/api/coordination/unstaged?projectId=${encodeURIComponent(this.projectId)}`);
    return (result ?? []) as unknown as UnstagedState[];
  }

  // ── Lifecycle ─────────────────────────────────────────

  stop(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  // ── Private helpers ───────────────────────────────────

  private startHeartbeat(): void {
    if (this.heartbeatTimer) clearInterval(this.heartbeatTimer);
    this.heartbeatTimer = setInterval(() => {
      void this.heartbeat().catch(() => {}); // non-critical
    }, 15_000);
    this.heartbeatTimer.unref(); // don't keep process alive
  }

  private async captureUnstagedFiles(): Promise<UnstagedFile[]> {
    try {
      const { stdout } = await this._execFileAsync('git', ['status', '--porcelain'], {
        cwd: this.projectPath,
        timeout: 5000,
      });
      return stdout
        .split('\n')
        .filter((line) => line.length >= 4)
        .map((line) => {
          const code = line.substring(0, 2);
          const path = line.substring(3).trim();
          let status: UnstagedFile['status'] = 'modified';
          if (code.includes('?') || code.includes('A')) status = 'added';
          if (code.includes('D')) status = 'deleted';
          return { path, status, layer: classifyLayer(path) };
        });
    } catch {
      return [];
    }
  }

  private post(path: string, body: unknown): Promise<Record<string, unknown> | null> {
    return new Promise((resolve) => {
      const payload = JSON.stringify(body);
      const headers: Record<string, string | number> = {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(payload),
      };
      if (this.authToken) headers['Authorization'] = `Bearer ${this.authToken}`;
      const req = this._httpRequest(
        { hostname: '127.0.0.1', port: this.hubPort, path, method: 'POST', headers, timeout: 5000 },
        (res) => {
          const chunks: Buffer[] = [];
          res.on('data', (c: Buffer) => chunks.push(c));
          res.on('end', () => {
            try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf-8'))); }
            catch { resolve(null); }
          });
        },
      );
      req.on('error', () => resolve(null));
      req.on('timeout', () => { req.destroy(); resolve(null); });
      req.end(payload);
    });
  }

  private get(path: string): Promise<unknown> {
    return new Promise((resolve) => {
      const headers: Record<string, string> = {};
      if (this.authToken) headers['Authorization'] = `Bearer ${this.authToken}`;
      const req = this._httpRequest(
        { hostname: '127.0.0.1', port: this.hubPort, path, method: 'GET', headers, timeout: 5000 },
        (res) => {
          const chunks: Buffer[] = [];
          res.on('data', (c: Buffer) => chunks.push(c));
          res.on('end', () => {
            try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf-8'))); }
            catch { resolve(null); }
          });
        },
      );
      req.on('error', () => resolve(null));
      req.on('timeout', () => { req.destroy(); resolve(null); });
      req.end();
    });
  }

  private del(path: string): Promise<void> {
    return new Promise((resolve) => {
      const headers: Record<string, string> = {};
      if (this.authToken) headers['Authorization'] = `Bearer ${this.authToken}`;
      const req = this._httpRequest(
        { hostname: '127.0.0.1', port: this.hubPort, path, method: 'DELETE', headers, timeout: 5000 },
        () => resolve(),
      );
      req.on('error', () => resolve());
      req.on('timeout', () => { req.destroy(); resolve(); });
      req.end();
    });
  }
}
