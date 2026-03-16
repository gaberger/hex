import { describe, it, expect, beforeEach, afterEach } from 'bun:test';
import { DashboardHub } from '../../src/adapters/primary/dashboard-hub.js';

function randomPort(): number {
  return 10000 + Math.floor(Math.random() * 50000);
}

describe('DashboardHub', () => {
  let hub: DashboardHub;
  let port: number;
  let baseUrl: string;

  beforeEach(async () => {
    port = randomPort();
    hub = new DashboardHub(port);
    await hub.start();
    baseUrl = `http://localhost:${port}`;
  });

  afterEach(() => {
    // shutdown is private but exposed via the close() returned by start(),
    // however we also need to handle the case where start wasn't called.
    // Access the underlying server to close it.
    const server = hub.httpServer;
    if (server) server.close();
  });

  // ── 1. Hub starts and stops ──────────────────────────

  it('starts and responds to /api/projects', async () => {
    const res = await fetch(`${baseUrl}/api/projects`);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body).toHaveProperty('projects');
    expect(Array.isArray(body.projects)).toBe(true);
  });

  // ── 2. Project registration ──────────────────────────

  it('registers a project and lists it', async () => {
    const regRes = await fetch(`${baseUrl}/api/projects/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'test-proj', rootPath: '/tmp/test-proj' }),
    });
    expect(regRes.status).toBe(200);
    const regBody = await regRes.json();
    expect(regBody.name).toBe('test-proj');
    expect(regBody.rootPath).toBe('/tmp/test-proj');
    expect(typeof regBody.id).toBe('string');

    const listRes = await fetch(`${baseUrl}/api/projects`);
    const listBody = await listRes.json();
    expect(listBody.projects.length).toBe(1);
    expect(listBody.projects[0].name).toBe('test-proj');
  });

  // ── 3. State push ────────────────────────────────────

  it('accepts pushed state and reflects it in project GET', async () => {
    // Register first
    const regRes = await fetch(`${baseUrl}/api/projects/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'push-test', rootPath: '/tmp/push-test' }),
    });
    const { id } = await regRes.json();

    // Push health data
    const healthData = { healthScore: 95, totalFiles: 10 };
    const pushRes = await fetch(`${baseUrl}/api/push`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ projectId: id, type: 'health', data: healthData }),
    });
    expect(pushRes.status).toBe(200);

    // Verify stored state
    const getRes = await fetch(`${baseUrl}/api/${id}/health`);
    expect(getRes.status).toBe(200);
    const body = await getRes.json();
    expect(body.healthScore).toBe(95);
    expect(body.totalFiles).toBe(10);
  });

  // ── 4. Malformed JSON returns 400 ────────────────────

  it('returns 400 for malformed JSON on register', async () => {
    const res = await fetch(`${baseUrl}/api/projects/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '{not valid json!!!',
    });
    expect(res.status).toBe(400);
    const body = await res.json();
    expect(body.error).toBeDefined();
  });

  it('returns 400 for malformed JSON on push', async () => {
    const res = await fetch(`${baseUrl}/api/push`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: '<<<broken>>>',
    });
    expect(res.status).toBe(400);
  });

  // ── 5. Body size limit enforced ──────────────────────

  it('rejects oversized body on register (limit 4096)', async () => {
    const oversized = JSON.stringify({ rootPath: '/tmp/x', padding: 'A'.repeat(5000) });
    try {
      const res = await fetch(`${baseUrl}/api/projects/register`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: oversized,
      });
      // Either the connection is destroyed (fetch throws) or we get an error status
      expect(res.ok).toBe(false);
    } catch {
      // Connection reset is expected — readBody destroys the request
      expect(true).toBe(true);
    }
  });

  // ── 6. CORS headers for localhost ────────────────────

  it('sets CORS headers for localhost origin', async () => {
    const res = await fetch(`${baseUrl}/api/projects`, {
      headers: { Origin: 'http://localhost:3000' },
    });
    expect(res.status).toBe(200);
    const acao = res.headers.get('access-control-allow-origin');
    expect(acao).toBe('http://localhost:3000');
    expect(res.headers.get('access-control-allow-methods')).toContain('GET');
  });

  it('handles OPTIONS preflight for localhost', async () => {
    const res = await fetch(`${baseUrl}/api/projects`, {
      method: 'OPTIONS',
      headers: { Origin: 'http://localhost:3000' },
    });
    expect(res.status).toBe(204);
  });

  // ── 7. Unknown routes return 404 ─────────────────────

  it('returns 404 for unknown routes', async () => {
    const res = await fetch(`${baseUrl}/nonexistent`);
    expect(res.status).toBe(404);
    const body = await res.json();
    expect(body.error).toBe('Not found');
  });
});
