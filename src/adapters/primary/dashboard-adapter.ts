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
import { readFileSync, watch, type FSWatcher } from 'node:fs';
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

// ── CORS origin validation ──────────────────────────────

function isLocalOrigin(origin: string): boolean {
  try {
    const url = new URL(origin);
    return url.hostname === 'localhost' || url.hostname === '127.0.0.1';
  } catch {
    return false;
  }
}

// ── Dashboard Adapter ───────────────────────────────────

export class DashboardAdapter {
  private sseClients: Set<ServerResponse> = new Set();
  private readonly healthCache = cached<unknown>(10_000);
  private readonly tokensCache = cached<unknown>(30_000);
  private watchers: FSWatcher[] = [];
  private fileChangeDebounce = new Map<string, ReturnType<typeof setTimeout>>();

  constructor(
    private readonly ctx: AppContext,
    private readonly port: number = 3847,
  ) {}

  async start(): Promise<{ url: string; close: () => void }> {
    const server = createServer((req, res) => {
      void this.handleRequest(req, res);
    });

    this.startFileWatcher();

    return new Promise((ok) => {
      server.listen(this.port, () => {
        const url = `http://localhost:${this.port}`;
        ok({
          url,
          close: () => {
            for (const w of this.watchers) w.close();
            this.watchers = [];
            for (const client of this.sseClients) client.end();
            this.sseClients.clear();
            server.close();
          },
        });
      });
    });
  }

  // ── File Watcher ──────────────────────────────────

  private startFileWatcher(): void {
    const srcDir = resolve(this.ctx.rootPath, 'src');
    try {
      const watcher = watch(srcDir, { recursive: true }, (_eventType, filename) => {
        if (!filename) return;
        // Only watch source files
        if (!filename.endsWith('.ts') && !filename.endsWith('.js') && !filename.endsWith('.go') && !filename.endsWith('.rs')) return;

        const relPath = `src/${filename}`;

        // Debounce rapid saves (IDE auto-save, formatters)
        const existing = this.fileChangeDebounce.get(relPath);
        if (existing) clearTimeout(existing);
        this.fileChangeDebounce.set(relPath, setTimeout(() => {
          this.fileChangeDebounce.delete(relPath);
          this.broadcast('file-change', {
            path: relPath,
            layer: classifyLayer(relPath),
            timestamp: Date.now(),
          });
          // Invalidate caches so next poll gets fresh data
          this.healthCache.set(null as unknown as never);
          this.tokensCache.set(null as unknown as never);
        }, 300));
      });
      this.watchers.push(watcher);
    } catch {
      // src/ may not exist yet in a fresh scaffold — that's fine
      process.stderr.write(`[dashboard] File watcher: src/ not found, graph highlighting disabled\n`);
    }
  }

  // ── Request router ──────────────────────────────────

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? '/', `http://localhost:${this.port}`);
    const path = url.pathname;

    // CORS headers — restrict to localhost origins only (exact hostname match)
    const origin = req.headers.origin ?? '';
    if (isLocalOrigin(origin) || !origin) {
      res.setHeader('Access-Control-Allow-Origin', origin || 'http://localhost');
    }
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
      if (path === '/api/project' && req.method === 'GET') {
        return this.json(res, 200, {
          rootPath: this.ctx.rootPath,
          name: this.ctx.rootPath.split('/').pop(),
          astIsStub: this.ctx.astIsStub,
        });
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
      console.error('[dashboard] %s %s:', req.method, path, err);
      this.json(res, 500, { error: 'Internal server error' });
    }
  }

  // ── GET / ───────────────────────────────────────────

  private serveIndex(res: ServerResponse): void {
    // Search multiple locations: import.meta.url varies between source and bundled
    const dir = dirname(fileURLToPath(import.meta.url));
    const candidates = [
      resolve(dir, 'dashboard', 'index.html'),                           // source: src/adapters/primary/
      resolve(dir, 'adapters', 'primary', 'dashboard', 'index.html'),    // bundled: dist/ or project root
      resolve(dir, 'src', 'adapters', 'primary', 'dashboard', 'index.html'), // project root
      resolve(this.ctx.rootPath, 'src', 'adapters', 'primary', 'dashboard', 'index.html'), // explicit rootPath
      resolve(this.ctx.rootPath, 'dist', 'adapters', 'primary', 'dashboard', 'index.html'), // dist via rootPath
    ];
    for (const htmlPath of candidates) {
      try {
        const html = readFileSync(htmlPath, 'utf-8');
        res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
        res.end(html);
        return;
      } catch { /* try next */ }
    }
    console.error('[dashboard] Dashboard HTML not found. Searched:', candidates);
    this.json(res, 500, { error: 'Internal server error' });
  }

  // ── GET /api/health ─────────────────────────────────

  private async handleHealth(res: ServerResponse): Promise<void> {
    const hit = this.healthCache.get();
    if (hit) return this.json(res, 200, hit);

    const result = await this.ctx.archAnalyzer.analyzeArchitecture('.');
    this.healthCache.set(result);
    this.json(res, 200, result);
  }

  // ── GET /api/tokens/overview ────────────────────────

  private async handleTokensOverview(res: ServerResponse): Promise<void> {
    const hit = this.tokensCache.get();
    if (hit) return this.json(res, 200, hit);

    const allFiles = await this.ctx.fs.glob('**/*.ts');
    const sourceFiles = allFiles.filter(
      (f) => !f.includes('node_modules') && !f.includes('dist')
        && !f.includes('.test.ts') && !f.includes('.spec.ts')
        && !f.includes('examples'),
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
    // Path traversal guard: reject paths with .. or absolute paths
    if (file.includes('..') || file.startsWith('/')) {
      return this.json(res, 400, { error: 'Invalid file path' });
    }
    // Verify file exists before extracting summaries
    if (!(await this.ctx.fs.exists(file))) {
      return this.json(res, 404, { error: 'File not found' });
    }
    const [l0, l1, l2, l3] = await Promise.all([
      this.ctx.ast.extractSummary(file, 'L0'),
      this.ctx.ast.extractSummary(file, 'L1'),
      this.ctx.ast.extractSummary(file, 'L2'),
      this.ctx.ast.extractSummary(file, 'L3'),
    ]);
    this.json(res, 200, {
      l0: { tokens: l0.tokenEstimate, exports: l0.exports.length, imports: l0.imports.length, lines: l0.lineCount },
      l1: { tokens: l1.tokenEstimate, exports: l1.exports.length, imports: l1.imports.length, lines: l1.lineCount },
      l2: { tokens: l2.tokenEstimate, exports: l2.exports.length, imports: l2.imports.length, lines: l2.lineCount },
      l3: { tokens: l3.tokenEstimate, exports: l3.exports.length, imports: l3.imports.length, lines: l3.lineCount },
    });
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
    } catch (err) {
      console.error('[dashboard] swarm query failed:', err);
      this.json(res, 200, {
        status: { id: 'none', topology: 'hierarchical', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'idle', error: 'Swarm unavailable' },
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
      const raw: unknown = JSON.parse(body);
      if (typeof raw !== 'object' || raw === null) {
        return this.json(res, 400, { error: 'Invalid JSON body: expected object' });
      }
      parsed = raw as { selectedOption?: string };
    } catch {
      // Client sent non-JSON body — return 400
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

const MAX_BODY_SIZE = 1024; // 1KB — decisions are tiny JSON objects

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((ok, fail) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on('data', (c: Buffer) => {
      size += c.length;
      if (size > MAX_BODY_SIZE) {
        req.destroy();
        fail(new Error('Request body too large'));
        return;
      }
      chunks.push(c);
    });
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
