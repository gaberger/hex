import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { ITodoQueryPort, ITodoCommandPort } from '../../core/ports/index.js';

const ID_PATTERN = /^[a-f0-9-]{1,36}$/;

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

function notFound(res: ServerResponse, msg = 'Not found'): void {
  json(res, 404, { error: msg });
}

function badRequest(res: ServerResponse, msg: string): void {
  json(res, 400, { error: msg });
}

export class HttpAdapter {
  constructor(
    private readonly queries: ITodoQueryPort,
    private readonly commands: ITodoCommandPort,
  ) {}

  listen(port = 3456): void {
    const server = createServer((req, res) => {
      this.handle(req, res).catch((err) => {
        const msg = err instanceof Error ? err.message : 'Internal error';
        json(res, 500, { error: msg });
      });
    });
    server.listen(port, () => {
      process.stdout.write(`HTTP server listening on http://localhost:${port}\n`);
    });
  }

  private async handle(req: IncomingMessage, res: ServerResponse): Promise<void> {
    const url = new URL(req.url ?? '/', `http://${req.headers.host ?? 'localhost'}`);
    const path = url.pathname;
    const method = req.method ?? 'GET';

    // GET /api/stats
    if (method === 'GET' && path === '/api/stats') {
      const stats = await this.queries.stats();
      return json(res, 200, stats);
    }

    // POST /api/todos/:id/complete
    const completeMatch = path.match(/^\/api\/todos\/([^/]+)\/complete$/);
    if (method === 'POST' && completeMatch) {
      const id = sanitizeId(completeMatch[1]);
      if (!id) return badRequest(res, 'Invalid ID format');
      try {
        const todo = await this.commands.complete(id);
        return json(res, 200, todo);
      } catch (err) {
        const msg = err instanceof Error ? err.message : 'Error';
        return msg.includes('not found') ? notFound(res, msg) : badRequest(res, msg);
      }
    }

    // /api/todos/:id
    const idMatch = path.match(/^\/api\/todos\/([^/]+)$/);
    if (idMatch) {
      const id = sanitizeId(idMatch[1]);
      if (!id) return badRequest(res, 'Invalid ID format');

      if (method === 'GET') {
        const todo = await this.queries.getById(id);
        return todo ? json(res, 200, todo) : notFound(res);
      }
      if (method === 'PATCH') {
        const body = await readBody(req);
        try {
          const todo = await this.commands.update(id, body);
          return json(res, 200, todo);
        } catch (err) {
          const msg = err instanceof Error ? err.message : 'Error';
          return msg.includes('not found') ? notFound(res, msg) : badRequest(res, msg);
        }
      }
      if (method === 'DELETE') {
        try {
          await this.commands.delete(id);
          return json(res, 204, null);
        } catch (err) {
          const msg = err instanceof Error ? err.message : 'Error';
          return msg.includes('not found') ? notFound(res, msg) : badRequest(res, msg);
        }
      }
    }

    // /api/todos
    if (path === '/api/todos') {
      if (method === 'GET') {
        const status = url.searchParams.get('status') ?? undefined;
        const priority = url.searchParams.get('priority') ?? undefined;
        const todos = await this.queries.filter(
          status as any,
          priority as any,
        );
        return json(res, 200, todos);
      }
      if (method === 'POST') {
        const body = await readBody(req);
        if (!body.title || typeof body.title !== 'string') {
          return badRequest(res, 'title is required');
        }
        const todo = await this.commands.create(
          body.title as string,
          body.priority as any,
          body.tags as any,
        );
        return json(res, 201, todo);
      }
    }

    notFound(res, `Route not found: ${method} ${path}`);
  }
}
