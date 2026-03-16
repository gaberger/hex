import { describe, it, expect, beforeAll, afterAll } from 'bun:test';
import { createServer, type Server } from 'node:http';
import { TodoService } from '../../src/core/usecases/todo-service.js';
import { HttpAdapter } from '../../src/adapters/primary/http-adapter.js';
import type { ITodoStoragePort } from '../../src/core/ports/index.js';
import type { TodoData } from '../../src/core/domain/entities.js';

class InMemoryStorage implements ITodoStoragePort {
  private data: TodoData[] = [];
  async load(): Promise<TodoData[]> { return structuredClone(this.data); }
  async save(todos: TodoData[]): Promise<void> { this.data = structuredClone(todos); }
}

const PORT = 13456;
const BASE = `http://localhost:${PORT}`;

async function api(path: string, options: RequestInit = {}) {
  const res = await fetch(`${BASE}${path}`, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  const body = res.status === 204 ? null : await res.json();
  return { status: res.status, body };
}

describe('HTTP Adapter Integration', () => {
  let adapter: HttpAdapter;

  beforeAll(() => {
    const storage = new InMemoryStorage();
    const service = new TodoService(storage);
    adapter = new HttpAdapter(service, service, undefined, '/dev/null');
    adapter.listen(PORT);
  });

  it('GET /api/health returns ok', async () => {
    const { status, body } = await api('/api/health');
    expect(status).toBe(200);
    expect(body.status).toBe('ok');
    expect(body.uptime).toBeGreaterThanOrEqual(0);
  });

  it('POST /api/todos creates a todo', async () => {
    const { status, body } = await api('/api/todos', {
      method: 'POST',
      body: JSON.stringify({ title: 'Integration test', priority: 'high' }),
    });
    expect(status).toBe(201);
    expect(body.title).toBe('Integration test');
    expect(body.priority).toBe('high');
    expect(body.status).toBe('pending');
  });

  it('GET /api/todos returns all todos', async () => {
    const { status, body } = await api('/api/todos');
    expect(status).toBe(200);
    expect(body.length).toBeGreaterThanOrEqual(1);
  });

  it('GET /api/stats returns statistics', async () => {
    const { status, body } = await api('/api/stats');
    expect(status).toBe(200);
    expect(body.total).toBeGreaterThanOrEqual(1);
    expect(typeof body.pending).toBe('number');
  });

  it('POST /api/todos/:id/complete marks as completed', async () => {
    const create = await api('/api/todos', {
      method: 'POST',
      body: JSON.stringify({ title: 'Complete me' }),
    });
    const { status, body } = await api(`/api/todos/${create.body.id}/complete`, {
      method: 'POST',
    });
    expect(status).toBe(200);
    expect(body.status).toBe('completed');
  });

  it('DELETE /api/todos/:id removes a todo', async () => {
    const create = await api('/api/todos', {
      method: 'POST',
      body: JSON.stringify({ title: 'Delete me' }),
    });
    const { status } = await api(`/api/todos/${create.body.id}`, {
      method: 'DELETE',
    });
    expect(status).toBe(204);
  });

  it('POST /api/todos with no title returns 400', async () => {
    const { status, body } = await api('/api/todos', {
      method: 'POST',
      body: JSON.stringify({}),
    });
    expect(status).toBe(400);
    expect(body.error).toContain('title');
  });

  it('GET /api/todos/:id returns 404 for unknown id', async () => {
    const { status } = await api('/api/todos/00000000-0000-0000-0000-000000000000');
    expect(status).toBe(404);
  });

  it('POST /api/todos/:id/complete returns error for unknown id', async () => {
    const { status } = await api('/api/todos/00000000-0000-0000-0000-000000000000/complete', {
      method: 'POST',
    });
    expect(status).toBe(404);
  });
});
