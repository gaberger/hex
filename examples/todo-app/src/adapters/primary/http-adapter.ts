import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { readFile } from 'node:fs/promises';
import { join, extname } from 'node:path';
import type { ITodoQueryPort, ITodoCommandPort } from '../../core/ports/index.js';
import type { ILoggerPort } from '../../core/ports/logger.js';
import { DomainError, NotFoundError, ValidationError, ConflictError } from '../../core/domain/errors.js';

const ID_PATTERN = /^[a-f0-9-]{1,36}$/;
const startedAt = Date.now();

const MIME_TYPES: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'application/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
};

function sanitizeId(raw: string): string | null {
  return ID_PATTERN.test(raw) ? raw : null;
}

async function readBody(req: IncomingMessage): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    req.on('data', (chunk: Buffer) => {
      size += chunk.length;
      if (size > 1_048_576) { reject(new Error('Body too large')); return; }
      chunks.push(chunk);
    });
    req.on('end', () => {
      try {
        const text = Buffer.concat(chunks).toString('utf-8');
        resolve(text ? JSON.parse(text) : {});
      } catch { resolve({}); }
    });
    req.on('error', reject);
  });
}

function json(res: ServerResponse, status: number, data: unknown): void {
  const body = JSON.stringify(data);
  res.writeHead(status, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(body),
  });
  res.end(body);
}

function errorResponse(res: ServerResponse, err: unknown): void {
  if (err instanceof NotFoundError) {
    return json(res, 404, { error: err.message, code: err.code });
  }
  if (err instanceof ValidationError) {
    return json(res, 400, { error: err.message, code: err.code });
  }
  if (err instanceof ConflictError) {
    return json(res, 409, { error: err.message, code: err.code });
  }
  if (err instanceof DomainError) {
    return json(res, 400, { error: err.message, code: err.code });
  }
  const msg = err instanceof Error ? err.message : 'Internal server error';
  json(res, 500, { error: msg });
}

export class HttpAdapter {
  private readonly publicDir: string;

  constructor(
    private readonly queries: ITodoQueryPort,
    private readonly commands: ITodoCommandPort,
    private readonly logger?: ILoggerPort,
    publicDir?: string,
  ) {
    this.publicDir = publicDir ?? join(process.cwd(), 'public');
  }

  listen(port = 3456): void {
    const server = createServer((req, res) => {
      const start = Date.now();
      this.handle(req, res)
        .catch((err) => errorResponse(res, err))
        .finally(() => {
          this.logger?.debug('HTTP request', {
            method: req.method,
            url: req.url,
            status: res.statusCode,
            ms: Date.now() - start,
          });
        });
    });
    server.listen(port, () => {
      this.logger?.info('HTTP server started', { port, url: `http://localhost:${port}` });
    });
  }

  private async handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? '/', `http://${req.headers.host ?? 'localhost'}`);
    const path = url.pathname;
    const method = req.method ?? 'GET';

    // Health check
    if (method === 'GET' && path === '/api/health') {
      return json(res, 200, {
        status: 'ok',
        uptime: Math.floor((Date.now() - startedAt) / 1000),
        timestamp: new Date().toISOString(),
      });
    }

    // GET /api/stats
    if (method === 'GET' && path === '/api/stats') {
      const stats = await this.queries.stats();
      return json(res, 200, stats);
    }

    // POST /api/todos/:id/complete
    const completeMatch = path.match(/^\/api\/todos\/([^/]+)\/complete$/);
    if (method === 'POST' && completeMatch) {
      const id = sanitizeId(completeMatch[1]);
      if (!id) return json(res, 400, { error: 'Invalid ID format', code: 'VALIDATION' });
      const todo = await this.commands.complete(id);
      return json(res, 200, todo);
    }

    // /api/todos/:id
    const idMatch = path.match(/^\/api\/todos\/([^/]+)$/);
    if (idMatch) {
      const id = sanitizeId(idMatch[1]);
      if (!id) return json(res, 400, { error: 'Invalid ID format', code: 'VALIDATION' });

      if (method === 'GET') {
        const todo = await this.queries.getById(id);
        if (!todo) return json(res, 404, { error: 'Todo not found', code: 'NOT_FOUND' });
        return json(res, 200, todo);
      }
      if (method === 'PATCH') {
        const body = await readBody(req);
        const todo = await this.commands.update(id, body);
        return json(res, 200, todo);
      }
      if (method === 'DELETE') {
        await this.commands.delete(id);
        return json(res, 204, null);
      }
    }

    // /api/todos
    if (path === '/api/todos') {
      if (method === 'GET') {
        const status = url.searchParams.get('status') ?? undefined;
        const priority = url.searchParams.get('priority') ?? undefined;
        const todos = await this.queries.filter(status as any, priority as any);
        return json(res, 200, todos);
      }
      if (method === 'POST') {
        const body = await readBody(req);
        if (!body.title || typeof body.title !== 'string') {
          return json(res, 400, { error: 'title is required', code: 'VALIDATION' });
        }
        const todo = await this.commands.create(
          body.title as string,
          body.priority as any,
          body.tags as any,
        );
        return json(res, 201, todo);
      }
    }

    // Static file serving for Web UI
    if (method === 'GET' && !path.startsWith('/api/')) {
      return this.serveStatic(res, path === '/' ? '/index.html' : path);
    }

    json(res, 404, { error: `Route not found: ${method} ${path}` });
  }

  private async serveStatic(res: ServerResponse, urlPath: string): Promise<void> {
    const safePath = urlPath.replace(/\.\./g, '').replace(/\/\//g, '/');
    const filePath = join(this.publicDir, safePath);

    try {
      const content = await readFile(filePath);
      const ext = extname(filePath);
      const contentType = MIME_TYPES[ext] ?? 'application/octet-stream';
      res.writeHead(200, {
        'Content-Type': contentType,
        'Content-Length': content.length,
        'Cache-Control': 'no-cache',
      });
      res.end(content);
    } catch {
      json(res, 404, { error: 'File not found' });
    }
  }
}
