/**
 * Dashboard Hub — multi-project broker
 *
 * Runs a single HTTP server that manages multiple project contexts.
 * Each project registers with a unique ID and gets its own file watcher,
 * caches, and SSE event stream. Clients connect once and switch between
 * projects via query params.
 *
 * Routes:
 *   GET  /                          Static HTML dashboard
 *   GET  /api/projects              List all registered projects
 *   POST /api/projects/register     Register a new project { rootPath }
 *   DELETE /api/projects/:id        Unregister a project
 *   GET  /api/:projectId/health     Architecture analysis
 *   GET  /api/:projectId/tokens/overview  Token efficiency
 *   GET  /api/:projectId/tokens/:file     File token detail
 *   GET  /api/:projectId/swarm      Swarm status
 *   GET  /api/:projectId/progress   Full progress report (tasks, agents, patterns)
 *   GET  /api/:projectId/patterns   Learned patterns from AgentDB
 *   GET  /api/:projectId/graph      Dependency graph
 *   GET  /api/:projectId/project    Project info
 *   GET  /api/events?project=:id    SSE stream (scoped to project, or all)
 *   POST /api/:projectId/decisions/:id  Decision response
 */

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { existsSync, readFileSync, statSync, watch, type FSWatcher } from 'node:fs';
import { resolve, dirname, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { AppContext, AppContextFactory } from '../../core/ports/app-context.js';
import type { ImportEdge } from '../../core/ports/index.js';

// Re-export so existing consumers don't break
export type { AppContextFactory } from '../../core/ports/app-context.js';

// ── Cache helper ────────────────────────────────────────

interface CacheEntry<T> {
  data: T;
  expiresAt: number;
}

function cached<T>(ttlMs: number): {
  get: () => T | null;
  set: (data: T) => void;
  invalidate: () => void;
} {
  let entry: CacheEntry<T> | null = null;
  return {
    get: () => (entry && Date.now() < entry.expiresAt ? entry.data : null),
    set: (data: T) => { entry = { data, expiresAt: Date.now() + ttlMs }; },
    invalidate: () => { entry = null; },
  };
}

// ── Layer classifier ────────────────────────────────────

function classifyLayer(filePath: string): string {
  if (filePath.includes('/core/domain/')) return 'domain';
  if (filePath.includes('/core/ports/')) return 'port';
  if (filePath.includes('/core/usecases/')) return 'usecase';
  if (filePath.includes('/adapters/primary/')) return 'primary-adapter';
  if (filePath.includes('/adapters/secondary/')) return 'secondary-adapter';
  return 'other';
}

// ── Per-project state ───────────────────────────────────

interface ProjectSlot {
  id: string;
  ctx: AppContext;
  healthCache: ReturnType<typeof cached>;
  tokensCache: ReturnType<typeof cached>;
  watchers: FSWatcher[];
  debounceTimers: Map<string, ReturnType<typeof setTimeout>>;
  registeredAt: number;
}

// ── SSE client with optional project filter ─────────────

interface SSEClient {
  res: ServerResponse;
  projectFilter: string | null; // null = all projects
  heartbeat: ReturnType<typeof setInterval>;
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

/** Read optional auth token from HEX_DASHBOARD_TOKEN env var */
function getDashboardToken(): string | null {
  return process.env['HEX_DASHBOARD_TOKEN'] ?? null;
}

/** Check bearer token on mutating requests. GET/OPTIONS always pass. */
function isAuthorized(req: IncomingMessage): boolean {
  const token = getDashboardToken();
  if (!token) return true;
  if (req.method === 'GET' || req.method === 'OPTIONS') return true;
  const authHeader = req.headers.authorization ?? '';
  return authHeader === `Bearer ${token}`;
}

// ── Dashboard Hub ───────────────────────────────────────

export class DashboardHub {
  private readonly projects = new Map<string, ProjectSlot>();
  private readonly sseClients = new Set<SSEClient>();
  private server: ReturnType<typeof createServer> | null = null;

  constructor(
    private readonly contextFactory: AppContextFactory,
    private readonly port: number = 3847,
  ) {}

  // ── Lifecycle ──────────────────────────────────────

  async start(): Promise<{ url: string; close: () => void }> {
    this.server = createServer((req, res) => {
      void this.handleRequest(req, res);
    });

    return new Promise((ok) => {
      this.server!.listen(this.port, () => {
        const url = `http://localhost:${this.port}`;
        ok({
          url,
          close: () => this.shutdown(),
        });
      });
    });
  }

  private shutdown(): void {
    for (const slot of this.projects.values()) {
      for (const w of slot.watchers) w.close();
      for (const t of slot.debounceTimers.values()) clearTimeout(t);
    }
    this.projects.clear();

    for (const client of this.sseClients) {
      clearInterval(client.heartbeat);
      client.res.end();
    }
    this.sseClients.clear();

    this.server?.close();
  }

  // ── Project Registration ──────────────────────────

  async registerProject(rootPath: string): Promise<ProjectSlot> {
    const absPath = resolve(rootPath);
    // Use basename + hash suffix to prevent collisions between identically-named dirs
    const basename = absPath.split('/').pop() ?? 'unknown';
    const hash = Array.from(absPath).reduce((h, c) => ((h << 5) - h + c.charCodeAt(0)) | 0, 0);
    const id = `${basename}-${(hash >>> 0).toString(36)}`;

    // Don't double-register
    if (this.projects.has(id)) {
      return this.projects.get(id)!;
    }

    const ctx = await this.contextFactory(absPath);
    const slot: ProjectSlot = {
      id,
      ctx,
      healthCache: cached(10_000),
      tokensCache: cached(30_000),
      watchers: [],
      debounceTimers: new Map(),
      registeredAt: Date.now(),
    };

    this.startFileWatcher(slot);
    this.projects.set(id, slot);

    this.broadcast('project-registered', {
      id,
      name: id,
      rootPath: absPath,
      timestamp: Date.now(),
    });

    return slot;
  }

  unregisterProject(id: string): boolean {
    const slot = this.projects.get(id);
    if (!slot) return false;

    for (const w of slot.watchers) w.close();
    for (const t of slot.debounceTimers.values()) clearTimeout(t);
    this.projects.delete(id);

    this.broadcast('project-unregistered', { id, timestamp: Date.now() });
    return true;
  }

  // ── File Watcher (per project) ────────────────────

  private startFileWatcher(slot: ProjectSlot): void {
    const srcDir = resolve(slot.ctx.rootPath, 'src');
    try {
      const watcher = watch(srcDir, { recursive: true }, (_eventType, filename) => {
        if (!filename) return;
        if (!filename.endsWith('.ts') && !filename.endsWith('.js') && !filename.endsWith('.go') && !filename.endsWith('.rs')) return;

        const relPath = `src/${filename}`;
        const existing = slot.debounceTimers.get(relPath);
        if (existing) clearTimeout(existing);

        slot.debounceTimers.set(relPath, setTimeout(() => {
          slot.debounceTimers.delete(relPath);
          this.broadcastToProject(slot.id, 'file-change', {
            path: relPath,
            layer: classifyLayer(relPath),
            project: slot.id,
            timestamp: Date.now(),
          });
          slot.healthCache.invalidate();
          slot.tokensCache.invalidate();
        }, 300));
      });
      slot.watchers.push(watcher);
    } catch {
      process.stderr.write(`[hub] File watcher: ${srcDir} not found for project ${slot.id}\n`);
    }
  }

  // ── Request Router ────────────────────────────────

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? '/', `http://localhost:${this.port}`);
    const path = url.pathname;

    // CORS — exact hostname match to prevent origin bypass (e.g. localhost.evil.com)
    const origin = req.headers.origin ?? '';
    if (isLocalOrigin(origin) || !origin) {
      res.setHeader('Access-Control-Allow-Origin', origin || 'http://localhost');
    }
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, DELETE, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization');
    if (req.method === 'OPTIONS') { res.writeHead(204).end(); return; }

    // Auth check — mutating endpoints require bearer token when HEX_DASHBOARD_TOKEN is set
    if (!isAuthorized(req)) {
      this.json(res, 401, { error: 'Unauthorized. Set Authorization: Bearer <HEX_DASHBOARD_TOKEN>' });
      return;
    }

    try {
      // ── Global routes ──
      if (path === '/' && req.method === 'GET') return this.serveIndex(res);
      if (path === '/api/projects' && req.method === 'GET') return this.handleListProjects(res);
      if (path === '/api/projects/register' && req.method === 'POST') return await this.handleRegisterProject(req, res);
      if (path.startsWith('/api/projects/') && req.method === 'DELETE') {
        const id = path.slice('/api/projects/'.length);
        return this.handleUnregisterProject(res, id);
      }
      if (path === '/api/events' && req.method === 'GET') {
        const projectFilter = url.searchParams.get('project');
        return this.handleSSE(req, res, projectFilter);
      }

      // ── Per-project routes: /api/:projectId/... ──
      const match = path.match(/^\/api\/([^/]+)\/(.+)$/);
      if (match) {
        const [, projectId, subPath] = match;
        const slot = this.projects.get(projectId);
        if (!slot) {
          return this.json(res, 404, { error: 'Not found' });
        }
        return await this.handleProjectRoute(slot, subPath, req, res);
      }

      this.json(res, 404, { error: 'Not found' });
    } catch (err) {
      console.error('[hub] %s %s:', req.method, path, err);
      this.json(res, 500, { error: 'Internal server error' });
    }
  }

  // ── Project-scoped route handler ──────────────────

  private async handleProjectRoute(
    slot: ProjectSlot,
    subPath: string,
    req: IncomingMessage,
    res: ServerResponse,
  ): Promise<void> {
    const ctx = slot.ctx;

    if (subPath === 'project' && req.method === 'GET') {
      return this.json(res, 200, {
        id: slot.id,
        rootPath: ctx.rootPath,
        name: slot.id,
        astIsStub: ctx.astIsStub,
        registeredAt: slot.registeredAt,
      });
    }

    if (subPath === 'health' && req.method === 'GET') {
      const hit = slot.healthCache.get();
      if (hit) return this.json(res, 200, hit);
      const result = await ctx.archAnalyzer.analyzeArchitecture('.');
      slot.healthCache.set(result);
      return this.json(res, 200, result);
    }

    if (subPath === 'tokens/overview' && req.method === 'GET') {
      const hit = slot.tokensCache.get();
      if (hit) return this.json(res, 200, hit);
      const globResults = await Promise.all([
        ctx.fs.glob('**/*.ts'),
        ctx.fs.glob('**/*.go'),
        ctx.fs.glob('**/*.rs'),
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
          const [l1, l3] = await Promise.all([
            ctx.ast.extractSummary(filePath, 'L1'),
            ctx.ast.extractSummary(filePath, 'L3'),
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
      slot.tokensCache.set(result);
      return this.json(res, 200, result);
    }

    if (subPath.startsWith('tokens/') && req.method === 'GET') {
      const file = decodeURIComponent(subPath.slice('tokens/'.length));
      if (file.includes('..') || file.startsWith('/')) {
        return this.json(res, 400, { error: 'Invalid file path' });
      }
      if (!(await ctx.fs.exists(file))) {
        return this.json(res, 404, { error: 'File not found' });
      }
      const [l0, l1, l2, l3] = await Promise.all([
        ctx.ast.extractSummary(file, 'L0'),
        ctx.ast.extractSummary(file, 'L1'),
        ctx.ast.extractSummary(file, 'L2'),
        ctx.ast.extractSummary(file, 'L3'),
      ]);
      return this.json(res, 200, {
        l0: { tokens: l0.tokenEstimate, exports: l0.exports.length, imports: l0.imports.length, lines: l0.lineCount },
        l1: { tokens: l1.tokenEstimate, exports: l1.exports.length, imports: l1.imports.length, lines: l1.lineCount },
        l2: { tokens: l2.tokenEstimate, exports: l2.exports.length, imports: l2.imports.length, lines: l2.lineCount },
        l3: { tokens: l3.tokenEstimate, exports: l3.exports.length, imports: l3.imports.length, lines: l3.lineCount },
      });
    }

    if (subPath === 'swarm' && req.method === 'GET') {
      try {
        const [status, tasks, agents] = await Promise.all([
          ctx.swarm.status(),
          ctx.swarm.listTasks(),
          ctx.swarm.listAgents(),
        ]);
        return this.json(res, 200, { status, tasks, agents });
      } catch (err) {
        console.error('[hub] swarm query failed:', err);
        return this.json(res, 200, {
          status: { id: 'none', topology: 'hierarchical', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'idle', error: 'Swarm unavailable' },
          tasks: [],
          agents: [],
        });
      }
    }

    if (subPath === 'progress' && req.method === 'GET') {
      try {
        const report = await ctx.swarm.getProgressReport();
        return this.json(res, 200, report);
      } catch (err) {
        console.error('[hub] progress query failed:', err);
        return this.json(res, 200, {
          swarmId: 'none', tasks: [], agents: [], patterns: { total: 0, recentlyUsed: 0 },
          sessions: [], overallPercent: 0, phase: 'idle',
        });
      }
    }

    if (subPath === 'patterns' && req.method === 'GET') {
      try {
        const patterns = await ctx.swarm.patternSearch('*', undefined, 50);
        return this.json(res, 200, { patterns });
      } catch (err) {
        console.error('[hub] pattern query failed:', err);
        return this.json(res, 200, { patterns: [] });
      }
    }

    if (subPath === 'graph' && req.method === 'GET') {
      const edges: ImportEdge[] = await ctx.archAnalyzer.buildDependencyGraph(ctx.rootPath);
      const nodeSet = new Set<string>();
      for (const e of edges) { nodeSet.add(e.from); nodeSet.add(e.to); }
      const nodes = Array.from(nodeSet).map((id) => ({ id, layer: classifyLayer(id) }));
      const graphEdges = edges.map((e) => ({ from: e.from, to: e.to, names: e.names }));
      return this.json(res, 200, { nodes, edges: graphEdges });
    }

    if (subPath.startsWith('decisions/') && req.method === 'POST') {
      const decisionId = subPath.slice('decisions/'.length);
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
      await ctx.notificationOrchestrator?.respondToDecision({
        requestId: decisionId,
        selectedOption: parsed.selectedOption,
        respondedBy: 'human',
        timestamp: Date.now(),
      });
      return this.json(res, 200, { ok: true });
    }

    this.json(res, 404, { error: 'Not found' });
  }

  // ── Global route handlers ─────────────────────────

  private handleListProjects(res: ServerResponse): void {
    const projects = Array.from(this.projects.values()).map((s) => ({
      id: s.id,
      rootPath: s.ctx.rootPath,
      astIsStub: s.ctx.astIsStub,
      registeredAt: s.registeredAt,
    }));
    this.json(res, 200, { projects });
  }

  private async handleRegisterProject(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const body = await readBody(req);
    let parsed: { rootPath?: string };
    try {
      const raw: unknown = JSON.parse(body);
      if (typeof raw !== 'object' || raw === null) {
        return this.json(res, 400, { error: 'Invalid JSON body: expected object' });
      }
      parsed = raw as { rootPath?: string };
    } catch {
      // Client sent non-JSON body — return 400
      return this.json(res, 400, { error: 'Invalid JSON body' });
    }
    if (!parsed.rootPath) {
      return this.json(res, 400, { error: 'Missing rootPath' });
    }

    // Validate the project path before registering.
    // Use a single generic error message to avoid revealing which check
    // failed, preventing filesystem probing via error differentiation.
    if (!isAbsolute(parsed.rootPath)) {
      return this.json(res, 400, { error: 'Invalid project path' });
    }
    const candidatePath = resolve(parsed.rootPath);
    try {
      const stat = statSync(candidatePath);
      if (!stat.isDirectory()) {
        return this.json(res, 400, { error: 'Invalid project path' });
      }
    } catch {
      return this.json(res, 400, { error: 'Invalid project path' });
    }
    // Must contain package.json or .hex-intf/ to be recognised as a project
    if (!existsSync(join(candidatePath, 'package.json')) && !existsSync(join(candidatePath, '.hex-intf'))) {
      return this.json(res, 400, { error: 'Invalid project path' });
    }

    try {
      const slot = await this.registerProject(parsed.rootPath);
      this.json(res, 200, { id: slot.id, rootPath: slot.ctx.rootPath, registeredAt: slot.registeredAt });
    } catch (err) {
      console.error('[hub] Failed to register project:', err);
      this.json(res, 500, { error: 'Internal server error' });
    }
  }

  private handleUnregisterProject(res: ServerResponse, id: string): void {
    if (this.unregisterProject(id)) {
      this.json(res, 200, { ok: true });
    } else {
      this.json(res, 404, { error: 'Not found' });
    }
  }

  // ── SSE ───────────────────────────────────────────

  private handleSSE(req: IncomingMessage, res: ServerResponse, projectFilter: string | null): void {
    res.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
    });

    // Send current project list on connect
    const projects = Array.from(this.projects.values()).map((s) => ({
      id: s.id, rootPath: s.ctx.rootPath, astIsStub: s.ctx.astIsStub,
    }));
    res.write(`event: connected\ndata: ${JSON.stringify({ projects })}\n\n`);

    const heartbeat = setInterval(() => { res.write(':heartbeat\n\n'); }, 15_000);
    const client: SSEClient = { res, projectFilter, heartbeat };
    this.sseClients.add(client);

    req.on('close', () => {
      clearInterval(heartbeat);
      this.sseClients.delete(client);
    });
  }

  // ── Broadcast ─────────────────────────────────────

  /** Broadcast to all SSE clients (global event). */
  broadcast(event: string, data: unknown): void {
    const msg = `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
    for (const client of this.sseClients) {
      client.res.write(msg);
    }
  }

  /** Broadcast to SSE clients subscribed to a specific project (or all). */
  broadcastToProject(projectId: string, event: string, data: unknown): void {
    const msg = `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
    for (const client of this.sseClients) {
      if (client.projectFilter === null || client.projectFilter === projectId) {
        client.res.write(msg);
      }
    }
  }

  // ── Static HTML ───────────────────────────────────

  private serveIndex(res: ServerResponse): void {
    const dir = dirname(fileURLToPath(import.meta.url));
    const candidates = [
      resolve(dir, 'dashboard', 'index.html'),
      resolve(dir, 'adapters', 'primary', 'dashboard', 'index.html'),
      resolve(dir, 'src', 'adapters', 'primary', 'dashboard', 'index.html'),
    ];
    for (const htmlPath of candidates) {
      try {
        const html = readFileSync(htmlPath, 'utf-8');
        res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
        res.end(html);
        return;
      } catch { /* try next */ }
    }
    this.json(res, 500, { error: 'Dashboard HTML not found' });
  }

  // ── Helpers ───────────────────────────────────────

  private json(res: ServerResponse, status: number, data: unknown): void {
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(data));
  }
}

// ── Body reader ─────────────────────────────────────────

const MAX_BODY_SIZE = 2048;

function readBody(req: IncomingMessage): Promise<string> {
  return new Promise((ok, fail) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on('data', (c: Buffer) => {
      size += c.length;
      if (size > MAX_BODY_SIZE) { req.destroy(); fail(new Error('Request body too large')); return; }
      chunks.push(c);
    });
    req.on('end', () => ok(Buffer.concat(chunks).toString('utf-8')));
    req.on('error', fail);
  });
}
