/**
 * Dashboard Primary Adapter
 *
 * Serves a web dashboard over HTTP and exposes hex-intf ports as
 * JSON API endpoints plus an SSE event stream. Uses only node:http
 * with zero external dependencies.
 *
 * Routes:
 *   GET  /                  Static HTML dashboard
 *   GET  /api/health        Architecture analysis (cached 10s)
 *   GET  /api/tokens/overview  Token efficiency for all TS files (cached 30s)
 *   GET  /api/tokens/:file  L0-L3 summaries for one file
 *   GET  /api/swarm         Swarm status, tasks, agents
 *   GET  /api/graph         Dependency graph as nodes + edges
 *   GET  /api/events        SSE stream (heartbeat every 15s)
 *   POST /api/decisions/:id Decision response
 */

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { AppContext } from '../../core/ports/app-context.js';
import type { ImportEdge } from '../../core/ports/index.js';

// ── Cache helper ────────────────────────────────────────

interface CacheEntry<T> {
  data: T;
  expiresAt: number;
}

function cached<T>(ttlMs: number): {
  get: () => T | null;
  set: (data: T) => void;
} {
  let entry: CacheEntry<T> | null = null;
  return {
    get: () => (entry && Date.now() < entry.expiresAt ? entry.data : null),
    set: (data: T) => { entry = { data, expiresAt: Date.now() + ttlMs }; },
  };
}

// ── Layer classifier for dependency graph nodes ─────────

function classifyLayer(filePath: string): string {
  if (filePath.includes('/core/domain/')) return 'domain';
  if (filePath.includes('/core/ports/')) return 'port';
  if (filePath.includes('/core/usecases/')) return 'usecase';
  if (filePath.includes('/adapters/primary/')) return 'primary-adapter';
  if (filePath.includes('/adapters/secondary/')) return 'secondary-adapter';
  return 'other';
}

// ── Dashboard Adapter ───────────────────────────────────

export class DashboardAdapter {
  private sseClients: Set<ServerResponse> = new Set();
  private readonly healthCache = cached<unknown>(10_000);
  private readonly tokensCache = cached<unknown>(30_000);

  constructor(
    private readonly ctx: AppContext,
    private readonly port: number = 3847,
  ) {}

  async start(): Promise<{ url: string; close: () => void }> {
    const server = createServer((req, res) => {
      void this.handleRequest(req, res);
    });

    return new Promise((ok) => {
      server.listen(this.port, () => {
        const url = `http://localhost:${this.port}`;
        ok({
          url,
          close: () => {
            for (const client of this.sseClients) client.end();
            this.sseClients.clear();
            server.close();
          },
        });
      });
    });
  }

  // ── Request router ──────────────────────────────────

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? '/', `http://localhost:${this.port}`);
    const path = url.pathname;

    // CORS headers for local development
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type');

    if (req.method === 'OPTIONS') {
      res.writeHead(204).end();
      return;
    }

    try {
      if (path === '/' && req.method === 'GET') {
        return this.serveIndex(res);
      }
      if (path === '/api/health' && req.method === 'GET') {
        return await this.handleHealth(res);
      }
      if (path === '/api/tokens/overview' && req.method === 'GET') {
        return await this.handleTokensOverview(res);
      }
      if (path.startsWith('/api/tokens/') && req.method === 'GET') {
        const file = decodeURIComponent(path.slice('/api/tokens/'.length));
        return await this.handleTokenDetail(res, file);
      }
      if (path === '/api/swarm' && req.method === 'GET') {
        return await this.handleSwarm(res);
      }
      if (path === '/api/graph' && req.method === 'GET') {
        return await this.handleGraph(res);
      }
      if (path === '/api/events' && req.method === 'GET') {
        return this.handleSSE(req, res);
      }
      if (path.startsWith('/api/decisions/') && req.method === 'POST') {
        const id = path.slice('/api/decisions/'.length);
        return await this.handleDecision(req, res, id);
      }

      this.json(res, 404, { error: 'Not found' });
    } catch (err) {
      process.stderr.write(`[dashboard] ${req.method} ${path}: ${String(err)}\n`);
      this.json(res, 500, { error: err instanceof Error ? err.message : 'Internal error' });
    }
  }

  // ── GET / ───────────────────────────────────────────

  private serveIndex(res: ServerResponse): void {
    const dir = dirname(fileURLToPath(import.meta.url));
    const htmlPath = resolve(dir, 'dashboard', 'index.html');
    try {
      const html = readFileSync(htmlPath, 'utf-8');
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
      res.end(html);
    } catch {
      this.json(res, 500, { error: 'Dashboard HTML not found. Expected at: ' + htmlPath });
    }
  }

  // ── GET /api/health ─────────────────────────────────

  private async handleHealth(res: ServerResponse): Promise<void> {
    const hit = this.healthCache.get();
    if (hit) return this.json(res, 200, hit);

    const result = await this.ctx.archAnalyzer.analyzeArchitecture(this.ctx.rootPath);
    this.healthCache.set(result);
    this.json(res, 200, result);
  }

  // ── GET /api/tokens/overview ────────────────────────

  private async handleTokensOverview(res: ServerResponse): Promise<void> {
    const hit = this.tokensCache.get();
    if (hit) return this.json(res, 200, hit);

    const allFiles = await this.ctx.fs.glob('**/*.ts');
    const sourceFiles = allFiles.filter(
      (f) => !f.includes('node_modules') && !f.includes('dist') && !f.includes('/tests/'),
    );

    const files = await Promise.all(
      sourceFiles.map(async (filePath) => {
        const [l1, l3] = await Promise.all([
          this.ctx.ast.extractSummary(filePath, 'L1'),
          this.ctx.ast.extractSummary(filePath, 'L3'),
        ]);
        return {
          path: filePath,
          l1Tokens: l1.tokenEstimate,
          l3Tokens: l3.tokenEstimate,
          ratio: l1.tokenEstimate > 0 ? +(l3.tokenEstimate / l1.tokenEstimate).toFixed(3) : 0,
          lineCount: l1.lineCount,
        };
      }),
    );

    const result = { files };
    this.tokensCache.set(result);
    this.json(res, 200, result);
  }

  // ── GET /api/tokens/:file ──────────────────────────

  private async handleTokenDetail(res: ServerResponse, file: string): Promise<void> {
    const [l0, l1, l2, l3] = await Promise.all([
      this.ctx.ast.extractSummary(file, 'L0'),
      this.ctx.ast.extractSummary(file, 'L1'),
      this.ctx.ast.extractSummary(file, 'L2'),
      this.ctx.ast.extractSummary(file, 'L3'),
    ]);
    this.json(res, 200, { l0, l1, l2, l3 });
  }

  // ── GET /api/swarm ─────────────────────────────────

  private async handleSwarm(res: ServerResponse): Promise<void> {
    try {
      const [status, tasks, agents] = await Promise.all([
        this.ctx.swarm.status(),
        this.ctx.swarm.listTasks(),
        this.ctx.swarm.listAgents(),
      ]);
      this.json(res, 200, { status, tasks, agents });
    } catch {
      this.json(res, 200, {
        status: { id: 'none', topology: 'hierarchical', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'idle' },
        tasks: [],
        agents: [],
      });
    }
  }

  // ── GET /api/graph ─────────────────────────────────

  private async handleGraph(res: ServerResponse): Promise<void> {
    const edges: ImportEdge[] = await this.ctx.archAnalyzer.buildDependencyGraph(this.ctx.rootPath);

    const nodeSet = new Set<string>();
    for (const e of edges) {
      nodeSet.add(e.from);
      nodeSet.add(e.to);
    }

    const nodes = Array.from(nodeSet).map((id) => ({ id, layer: classifyLayer(id) }));
    const graphEdges = edges.map((e) => ({ from: e.from, to: e.to, names: e.names }));

    this.json(res, 200, { nodes, edges: graphEdges });
  }

  // ── GET /api/events (SSE) ──────────────────────────

  private handleSSE(req: IncomingMessage, res: ServerResponse): void {
    res.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
    });

    res.write('event: connected\ndata: {}\n\n');
    this.sseClients.add(res);

    // Heartbeat every 15 seconds
    const heartbeat = setInterval(() => {
      res.write(':heartbeat\n\n');
    }, 15_000);

    req.on('close', () => {
      clearInterval(heartbeat);
      this.sseClients.delete(res);
    });
  }

  // ── POST /api/decisions/:id ────────────────────────

  private async handleDecision(
    req: IncomingMessage,
    res: ServerResponse,
    decisionId: string,
  ): Promise<void> {
    const body = await readBody(req);
    let parsed: { selectedOption?: string };
    try {
      parsed = JSON.parse(body) as { selectedOption?: string };
    } catch {
      return this.json(res, 400, { error: 'Invalid JSON body' });
    }

    if (!parsed.selectedOption) {
      return this.json(res, 400, { error: 'Missing selectedOption' });
    }

    await this.ctx.notificationOrchestrator?.respondToDecision({
      requestId: decisionId,
      selectedOption: parsed.selectedOption,
      respondedBy: 'human',
      timestamp: Date.now(),
    });

    this.json(res, 200, { ok: true });
  }

  // ── SSE broadcast ──────────────────────────────────

  broadcast(event: string, data: unknown): void {
    const msg = `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
    for (const client of this.sseClients) {
      client.write(msg);
    }
  }

  // ── JSON response helper ───────────────────────────

  private json(res: ServerResponse, status: number, data: unknown): void {
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(data));
  }
}

// ── Body reader ─────────────────────────────────────────

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((ok, fail) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => ok(Buffer.concat(chunks).toString('utf-8')));
    req.on('error', fail);
  });
}

// ── Convenience factory ─────────────────────────────────

export async function startDashboard(
  ctx: AppContext,
  port?: number,
): Promise<{ url: string; close: () => void }> {
  const adapter = new DashboardAdapter(ctx, port);
  return adapter.start();
}
