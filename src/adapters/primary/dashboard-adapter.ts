/**
 * Dashboard Client Adapter
 *
 * Connects to the central Dashboard Hub (port 5555) as a CLIENT.
 * Gathers data from the local project's AppContext and pushes it
 * to the hub periodically. Also watches local files and pushes
 * change events in real-time.
 *
 * Push model: this adapter is the "project side" — it collects
 * health, tokens, swarm, and graph data locally, then POSTs it
 * to the hub. The hub stores and serves it to browsers.
 */

import { watch, readFileSync, type FSWatcher } from 'node:fs';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { request } from 'node:http';
import type { AppContext } from '../../core/ports/app-context.js';
import type { ImportEdge } from '../../core/ports/index.js';

/** Fixed dashboard hub port — must match dashboard-hub.ts */
const HUB_PORT = 5555;

/** Read auth token from hub lock file. Returns empty string if unavailable. */
function readHubToken(): string {
  try {
    const lockPath = join(homedir(), '.hex', 'daemon', 'hub.lock');
    const lock = JSON.parse(readFileSync(lockPath, 'utf-8'));
    return lock.token ?? '';
  } catch {
    return '';
  }
}

// ── Layer classifier ─────────────────────────────────────

function classifyLayer(filePath: string): string {
  if (filePath.includes('/core/domain/')) return 'domain';
  if (filePath.includes('/core/ports/')) return 'port';
  if (filePath.includes('/core/usecases/')) return 'usecase';
  if (filePath.includes('/adapters/primary/')) return 'primary-adapter';
  if (filePath.includes('/adapters/secondary/')) return 'secondary-adapter';
  return 'other';
}

// ── Dashboard Client ─────────────────────────────────────

export class DashboardAdapter {
  private projectId: string | null = null;
  private pushTimer: ReturnType<typeof setInterval> | null = null;
  private watchers: FSWatcher[] = [];
  private fileChangeDebounce = new Map<string, ReturnType<typeof setTimeout>>();
  private stopped = false;
  private readonly authToken: string;

  constructor(
    private readonly ctx: AppContext,
    private readonly hubPort: number = HUB_PORT,
  ) {
    this.authToken = readHubToken();
  }

  /**
   * Register with the hub and start pushing data.
   * Returns the hub URL for display purposes.
   * If the hub is not running, throws (caller can catch and skip).
   */
  async start(): Promise<{ url: string; close: () => void }> {
    const hubUrl = `http://localhost:${this.hubPort}`;

    // Register with hub
    const name = this.ctx.rootPath.split('/').pop() ?? 'unknown';
    const regResult = await this.post('/api/projects/register', {
      name,
      rootPath: this.ctx.rootPath,
      astIsStub: this.ctx.astIsStub,
    });

    if (!regResult || !regResult.id) {
      throw new Error('Hub registration failed');
    }

    this.projectId = regResult.id as string;

    // Initial push — fire and forget (don't block registration)
    void this.pushAll().catch((err) => this.log('initial push failed:', err));

    // Start periodic push (every 10s)
    this.pushTimer = setInterval(() => {
      if (!this.stopped) void this.pushAll().catch((err) => this.log('periodic push failed:', err));
    }, 10_000);
    this.pushTimer.unref(); // Don't keep process alive

    // Watch local files for changes
    this.startFileWatcher();

    return {
      url: hubUrl,
      close: () => this.stop(),
    };
  }

  /** Stop pushing and unregister. */
  stop(): void {
    this.stopped = true;
    if (this.pushTimer) {
      clearInterval(this.pushTimer);
      this.pushTimer = null;
    }
    for (const w of this.watchers) w.close();
    this.watchers = [];
    for (const t of this.fileChangeDebounce.values()) clearTimeout(t);
    this.fileChangeDebounce.clear();
  }

  // ── Push all state ─────────────────────────────────

  private async pushAll(): Promise<void> {
    if (!this.projectId) return;

    // Push health, tokens, swarm, graph in parallel
    await Promise.allSettled([
      this.pushHealth(),
      this.pushTokens(),
      this.pushSwarm(),
      this.pushGraph(),
    ]);
  }

  private async pushHealth(): Promise<void> {
    try {
      const result = await this.ctx.archAnalyzer.analyzeArchitecture('.');
      await this.pushState('health', result);
    } catch (err) {
      this.log('health push failed:', err);
    }
  }

  private async pushTokens(): Promise<void> {
    try {
      const globResults = await Promise.all([
        this.ctx.fs.glob('**/*.ts'),
        this.ctx.fs.glob('**/*.go'),
        this.ctx.fs.glob('**/*.rs'),
      ]);
      const allFiles = globResults.flat();
      const sourceFiles = allFiles.filter(
        (f) => !f.includes('node_modules') && !f.includes('dist')
          && !f.includes('.test.ts') && !f.includes('.spec.ts')
          && !f.includes('_test.go') && !f.includes('.test.rs')
          && !f.includes('examples'),
      );

      const files = await Promise.all(
        sourceFiles.map(async (filePath) => {
          const [l0, l1, l2, l3] = await Promise.all([
            this.ctx.ast.extractSummary(filePath, 'L0'),
            this.ctx.ast.extractSummary(filePath, 'L1'),
            this.ctx.ast.extractSummary(filePath, 'L2'),
            this.ctx.ast.extractSummary(filePath, 'L3'),
          ]);
          return {
            path: filePath,
            l0Tokens: l0.tokenEstimate,
            l1Tokens: l1.tokenEstimate,
            l2Tokens: l2.tokenEstimate,
            l3Tokens: l3.tokenEstimate,
            // Compression ratio: fraction of tokens saved (0-1).
            // L1 is the summary, L3 is the full source.
            ratio: l3.tokenEstimate > 0 ? +(1 - l1.tokenEstimate / l3.tokenEstimate).toFixed(3) : 0,
            lineCount: l1.lineCount,
          };
        }),
      );

      // Push overview (file list with ratios)
      await this.pushState('tokens', { files });

      // Push per-file token levels so the dashboard can render L0-L3 bars
      await Promise.all(
        files.map((f) =>
          this.post('/api/push', {
            projectId: this.projectId,
            type: 'tokenFile',
            filePath: f.path,
            data: {
              l0: { tokens: f.l0Tokens },
              l1: { tokens: f.l1Tokens },
              l2: { tokens: f.l2Tokens },
              l3: { tokens: f.l3Tokens },
            },
          }),
        ),
      );
    } catch (err) {
      this.log('tokens push failed:', err);
    }
  }

  private async pushSwarm(): Promise<void> {
    try {
      const [status, tasks, agents] = await Promise.all([
        this.ctx.swarm.status(),
        this.ctx.swarm.listTasks(),
        this.ctx.swarm.listAgents(),
      ]);
      await this.pushState('swarm', { status, tasks, agents });
    } catch (err) {
      // Swarm may not be running — push empty state
      await this.pushState('swarm', {
        status: { status: 'idle', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0 },
        tasks: [],
        agents: [],
      }).catch(() => {});
    }
  }

  private async pushGraph(): Promise<void> {
    try {
      const edges: ImportEdge[] = await this.ctx.archAnalyzer.buildDependencyGraph(this.ctx.rootPath);
      const nodeSet = new Set<string>();
      for (const e of edges) { nodeSet.add(e.from); nodeSet.add(e.to); }
      const nodes = Array.from(nodeSet).map((id) => ({ id, layer: classifyLayer(id) }));
      const graphEdges = edges.map((e) => ({ from: e.from, to: e.to, names: e.names }));
      await this.pushState('graph', { nodes, edges: graphEdges });
    } catch (err) {
      this.log('graph push failed:', err);
    }
  }

  // ── File Watcher ───────────────────────────────────

  private startFileWatcher(): void {
    const srcDir = resolve(this.ctx.rootPath, 'src');
    try {
      const watcher = watch(srcDir, { recursive: true }, (_eventType, filename) => {
        if (!filename) return;
        if (!filename.endsWith('.ts') && !filename.endsWith('.js') && !filename.endsWith('.go') && !filename.endsWith('.rs')) return;

        const relPath = `src/${filename}`;
        const existing = this.fileChangeDebounce.get(relPath);
        if (existing) clearTimeout(existing);

        this.fileChangeDebounce.set(relPath, setTimeout(() => {
          this.fileChangeDebounce.delete(relPath);
          void this.pushEvent('file-change', {
            path: relPath,
            layer: classifyLayer(relPath),
            timestamp: Date.now(),
          });
        }, 300));
      });
      this.watchers.push(watcher);
    } catch {
      // src/ may not exist yet — that's fine
    }
  }

  // ── HTTP helpers (push to hub) ─────────────────────

  private pushState(type: string, data: unknown): Promise<void> {
    return this.post('/api/push', {
      projectId: this.projectId,
      type,
      data,
    }).then(() => {});
  }

  private pushEvent(event: string, data: unknown): Promise<void> {
    return this.post('/api/event', {
      projectId: this.projectId,
      event,
      data,
    }).then(() => {});
  }

  /** POST JSON to the hub. Returns parsed response or null on failure. */
  private post(path: string, body: unknown): Promise<Record<string, unknown> | null> {
    return new Promise((resolve) => {
      const payload = JSON.stringify(body);
      const headers: Record<string, string | number> = {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(payload),
      };
      if (this.authToken) {
        headers['Authorization'] = `Bearer ${this.authToken}`;
      }
      const req = request(
        {
          hostname: '127.0.0.1',
          port: this.hubPort,
          path,
          method: 'POST',
          headers,
          timeout: 5000,
        },
        (res) => {
          const chunks: Buffer[] = [];
          res.on('data', (c: Buffer) => chunks.push(c));
          res.on('end', () => {
            try {
              const raw = JSON.parse(Buffer.concat(chunks).toString('utf-8'));
              resolve(raw as Record<string, unknown>);
            } catch {
              resolve(null);
            }
          });
        },
      );
      req.on('error', () => resolve(null));
      req.on('timeout', () => { req.destroy(); resolve(null); });
      req.end(payload);
    });
  }

  /** Broadcast to local SSE clients (kept for backward compat). */
  broadcast(event: string, data: unknown): void {
    // Forward to hub as an event
    if (this.projectId) {
      void this.pushEvent(event, data);
    }
  }

  private log(msg: string, err?: unknown): void {
    const errMsg = err instanceof Error ? err.message : String(err ?? '');
    process.stderr.write(`[dashboard-client] ${msg} ${errMsg}\n`);
  }
}

// ── Convenience factory ──────────────────────────────────

export async function startDashboard(
  ctx: AppContext,
  hubPort?: number,
): Promise<{ url: string; close: () => void }> {
  const adapter = new DashboardAdapter(ctx, hubPort);
  return adapter.start();
}
