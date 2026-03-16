/**
 * Dashboard Hub — Central multi-project dashboard server (push model)
 *
 * Runs on a FIXED port (default 5555). Projects are clients that
 * register and push their state via HTTP POST. The browser connects
 * to view and switch between projects.
 *
 * The hub stores no project logic — it is a passive state store
 * that receives data from projects and serves it to browsers.
 *
 * Browser-facing (GET):
 *   GET  /                              Static HTML dashboard
 *   GET  /api/projects                  List registered projects
 *   GET  /api/:projectId/health         Stored health analysis
 *   GET  /api/:projectId/tokens/overview Stored token overview
 *   GET  /api/:projectId/tokens/:file   Stored file token detail
 *   GET  /api/:projectId/swarm          Stored swarm status
 *   GET  /api/:projectId/graph          Stored dependency graph
 *   GET  /api/:projectId/project        Project metadata
 *
 * Project-facing (POST — projects push data IN):
 *   POST /api/projects/register         Register { name, rootPath }
 *   DELETE /api/projects/:id            Unregister
 *   POST /api/push                      Push state { projectId, type, data }
 *   POST /api/event                     Push event { projectId, event, data }
 *   POST /api/:projectId/decisions/:id  Decision response
 */

import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// ── Constants ────────────────────────────────────────────

export const HUB_PORT = 5555;

// ── Types ────────────────────────────────────────────────

interface ProjectEntry {
  id: string;
  name: string;
  rootPath: string;
  registeredAt: number;
  lastPushAt: number;
  state: {
    health: unknown | null;
    tokens: unknown | null;
    tokenFiles: Record<string, unknown>;
    swarm: unknown | null;
    graph: unknown | null;
    project: { rootPath: string; name: string; astIsStub?: boolean } | null;
  };
}

// ── CORS ─────────────────────────────────────────────────

function isLocalOrigin(origin: string): boolean {
  try {
    const url = new URL(origin);
    return url.hostname === 'localhost' || url.hostname === '127.0.0.1';
  } catch {
    return false;
  }
}

// ── Auth ─────────────────────────────────────────────────

function getDashboardToken(): string | null {
  return process.env['HEX_DASHBOARD_TOKEN'] ?? null;
}

function isAuthorized(req: IncomingMessage): boolean {
  const token = getDashboardToken();
  if (!token) return true;
  if (req.method === 'GET' || req.method === 'OPTIONS') return true;
  const authHeader = req.headers.authorization ?? '';
  return authHeader === `Bearer ${token}`;
}

// ── Project ID generation ────────────────────────────────

function makeProjectId(rootPath: string): string {
  const basename = rootPath.split('/').pop() ?? 'unknown';
  const hash = Array.from(rootPath).reduce((h, c) => ((h << 5) - h + c.charCodeAt(0)) | 0, 0);
  return `${basename}-${(hash >>> 0).toString(36)}`;
}

// ── Dashboard Hub ────────────────────────────────────────

export class DashboardHub {
  private readonly projects = new Map<string, ProjectEntry>();
  private server: ReturnType<typeof createServer> | null = null;

  constructor(private readonly port: number = HUB_PORT) {}

  /** Expose underlying server for WebSocket upgrade attachment. */
  get httpServer(): ReturnType<typeof createServer> | null {
    return this.server;
  }

  // ── Lifecycle ──────────────────────────────────────

  async start(): Promise<{ url: string; close: () => void }> {
    this.server = createServer((req, res) => {
      void this.handleRequest(req, res);
    });

    return new Promise((ok, fail) => {
      this.server!.on('error', fail);
      this.server!.listen(this.port, () => {
        const url = `http://localhost:${this.port}`;
        ok({ url, close: () => this.shutdown() });
      });
    });
  }

  private shutdown(): void {
    this.projects.clear();
    this.server?.close();
  }

  // ── Request Router ─────────────────────────────────

  private async handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? '/', `http://localhost:${this.port}`);
    const path = url.pathname;

    // CORS
    const origin = req.headers.origin ?? '';
    if (isLocalOrigin(origin) || !origin) {
      res.setHeader('Access-Control-Allow-Origin', origin || 'http://localhost');
    }
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, DELETE, OPTIONS');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type, Authorization');
    if (req.method === 'OPTIONS') { res.writeHead(204).end(); return; }

    if (!isAuthorized(req)) {
      this.json(res, 401, { error: 'Unauthorized' });
      return;
    }

    try {
      // ── Static ──
      if (path === '/' && req.method === 'GET') return this.serveIndex(res);

      // ── Global routes ──
      if (path === '/api/projects' && req.method === 'GET') return this.handleListProjects(res);
      if (path === '/api/projects/register' && req.method === 'POST') return await this.handleRegister(req, res);
      if (path === '/api/push' && req.method === 'POST') return await this.handlePush(req, res);
      if (path === '/api/event' && req.method === 'POST') return await this.handleEvent(req, res);
      // ── DELETE /api/projects/:id ──
      if (path.startsWith('/api/projects/') && req.method === 'DELETE') {
        const id = path.slice('/api/projects/'.length);
        return this.handleUnregister(res, id);
      }

      // ── Per-project GET routes: /api/:projectId/... ──
      const match = path.match(/^\/api\/([^/]+)\/(.+)$/);
      if (match && req.method === 'GET') {
        const [, projectId, subPath] = match;
        return this.handleProjectGet(res, projectId, subPath);
      }

      // ── Per-project POST routes (decisions) ──
      if (match && req.method === 'POST') {
        const [, projectId, subPath] = match;
        const decMatch = subPath.match(/^decisions\/(.+)$/);
        if (decMatch) {
          return await this.handleDecision(req, res, projectId, decMatch[1]);
        }
      }

      this.json(res, 404, { error: 'Not found' });
    } catch (err) {
      console.error('[hub] %s %s:', req.method, path, err);
      this.json(res, 500, { error: 'Internal server error' });
    }
  }

  // ── Registration ───────────────────────────────────

  private async handleRegister(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const body = await readBody(req, 4096);
    const parsed = safeParse(body);
    if (!parsed || typeof parsed.rootPath !== 'string') {
      return this.json(res, 400, { error: 'Missing rootPath' });
    }

    const rootPath = parsed.rootPath as string;
    const name = (parsed.name as string) || rootPath.split('/').pop() || 'unknown';
    const id = makeProjectId(rootPath);

    if (!this.projects.has(id)) {
      this.projects.set(id, {
        id,
        name,
        rootPath,
        registeredAt: Date.now(),
        lastPushAt: 0,
        state: {
          health: null,
          tokens: null,
          tokenFiles: {},
          swarm: null,
          graph: null,
          project: { rootPath, name, astIsStub: (parsed.astIsStub as boolean) ?? false },
        },
      });

      // WS broadcasting handled by Rust hub
    } else {
      // Update metadata on re-register
      const entry = this.projects.get(id)!;
      entry.name = name;
      entry.state.project = { rootPath, name, astIsStub: (parsed.astIsStub as boolean) ?? false };
    }

    this.json(res, 200, { id, name, rootPath });
  }

  private handleUnregister(res: ServerResponse, id: string): void {
    if (this.projects.delete(id)) {
      // WS broadcasting handled by Rust hub
      this.json(res, 200, { ok: true });
    } else {
      this.json(res, 404, { error: 'Not found' });
    }
  }

  private handleListProjects(res: ServerResponse): void {
    const projects = Array.from(this.projects.values()).map((p) => ({
      id: p.id,
      name: p.name,
      rootPath: p.rootPath,
      registeredAt: p.registeredAt,
      lastPushAt: p.lastPushAt,
      astIsStub: p.state.project?.astIsStub ?? false,
    }));
    this.json(res, 200, { projects });
  }

  // ── Push (projects send state IN) ──────────────────

  private async handlePush(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const body = await readBody(req, 256_000); // 256KB — analysis payloads can be large
    const parsed = safeParse(body);
    if (!parsed || typeof parsed.projectId !== 'string' || typeof parsed.type !== 'string') {
      return this.json(res, 400, { error: 'Missing projectId or type' });
    }

    const { projectId, type, data } = parsed as { projectId: string; type: string; data: unknown };
    const entry = this.projects.get(projectId);
    if (!entry) {
      return this.json(res, 404, { error: 'Project not registered' });
    }

    entry.lastPushAt = Date.now();

    switch (type) {
      case 'health':
        entry.state.health = data;
        break;
      case 'tokens':
        entry.state.tokens = data;
        break;
      case 'tokenFile': {
        const filePath = (parsed as Record<string, unknown>).filePath as string;
        if (filePath) entry.state.tokenFiles[filePath] = data;
        break;
      }
      case 'swarm':
        entry.state.swarm = data;
        break;
      case 'graph':
        entry.state.graph = data;
        break;
      case 'project':
        entry.state.project = data as ProjectEntry['state']['project'];
        break;
      default:
        return this.json(res, 400, { error: `Unknown state type: ${type}` });
    }

    // WS broadcasting handled by Rust hub
    this.json(res, 200, { ok: true });
  }

  // ── Event (projects push real-time events) ─────────

  private async handleEvent(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const body = await readBody(req, 16_000);
    const parsed = safeParse(body);
    if (!parsed || typeof parsed.projectId !== 'string' || typeof parsed.event !== 'string') {
      return this.json(res, 400, { error: 'Missing projectId or event' });
    }

    const { projectId, event, data } = parsed as { projectId: string; event: string; data: unknown };
    if (!this.projects.has(projectId)) {
      return this.json(res, 404, { error: 'Project not registered' });
    }

    this.projects.get(projectId)!.lastPushAt = Date.now();
    // WS broadcasting handled by Rust hub
    this.json(res, 200, { ok: true });
  }

  // ── Per-project GET (browser reads stored state) ───

  private handleProjectGet(res: ServerResponse, projectId: string, subPath: string): void {
    const entry = this.projects.get(projectId);
    if (!entry) {
      return this.json(res, 404, { error: 'Not found' });
    }

    if (subPath === 'project') {
      return this.json(res, 200, entry.state.project ?? { rootPath: entry.rootPath, name: entry.name });
    }
    if (subPath === 'health') {
      return this.json(res, 200, entry.state.health ?? { summary: { healthScore: 0, totalFiles: 0, totalExports: 0, deadExportCount: 0, violationCount: 0, circularCount: 0 } });
    }
    if (subPath === 'tokens/overview') {
      return this.json(res, 200, entry.state.tokens ?? { files: [] });
    }
    if (subPath.startsWith('tokens/')) {
      const file = decodeURIComponent(subPath.slice('tokens/'.length));
      const fileData = entry.state.tokenFiles[file];
      if (!fileData) return this.json(res, 404, { error: 'File not found' });
      return this.json(res, 200, fileData);
    }
    if (subPath === 'swarm') {
      return this.json(res, 200, entry.state.swarm ?? {
        status: { status: 'idle', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0 },
        tasks: [],
        agents: [],
      });
    }
    if (subPath === 'graph') {
      return this.json(res, 200, entry.state.graph ?? { nodes: [], edges: [] });
    }

    this.json(res, 404, { error: 'Not found' });
  }

  // ── Decision (browser → hub) ────────────────────────

  private async handleDecision(
    req: IncomingMessage,
    res: ServerResponse,
    projectId: string,
    decisionId: string,
  ): Promise<void> {
    const body = await readBody(req, 2048);
    const parsed = safeParse(body);
    if (!parsed || typeof parsed.selectedOption !== 'string') {
      return this.json(res, 400, { error: 'Missing selectedOption' });
    }

    // Log the decision (WS broadcasting handled by the Rust hub)
    void ({
      decisionId,
      selectedOption: parsed.selectedOption,
      respondedBy: 'human',
      timestamp: Date.now(),
    });

    this.json(res, 200, { ok: true });
  }

  // ── Static HTML ────────────────────────────────────

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

  // ── Helpers ────────────────────────────────────────

  private json(res: ServerResponse, status: number, data: unknown): void {
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(data));
  }
}

// ── Body reader ──────────────────────────────────────────

function readBody(req: IncomingMessage, maxSize: number): Promise<string> {
  return new Promise((ok, fail) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on('data', (c: Buffer) => {
      size += c.length;
      if (size > maxSize) { req.destroy(); fail(new Error('Request body too large')); return; }
      chunks.push(c);
    });
    req.on('end', () => ok(Buffer.concat(chunks).toString('utf-8')));
    req.on('error', fail);
  });
}

function safeParse(body: string): Record<string, unknown> | null {
  try {
    const raw: unknown = JSON.parse(body);
    if (typeof raw !== 'object' || raw === null) return null;
    return raw as Record<string, unknown>;
  } catch {
    return null;
  }
}
