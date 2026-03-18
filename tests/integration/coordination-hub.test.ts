/**
 * Integration Test — Coordination endpoints against an in-process mock hub
 *
 * Spins up a lightweight HTTP server that implements the coordination API contract,
 * then exercises the full lock/claim/activity/unstaged lifecycle via HTTP.
 * No external hex-hub process required.
 */

import { describe, it, expect, beforeAll, afterAll } from 'bun:test';
import { createServer, type Server, type IncomingMessage, type ServerResponse } from 'node:http';

// ── In-process mock hub ──────────────────────────────────

interface MockInstance {
  instanceId: string;
  projectId: string;
  pid: number;
  sessionLabel: string;
  registeredAt: string;
  lastSeen: string;
}

interface MockLock {
  instanceId: string;
  projectId: string;
  feature: string;
  layer: string;
  acquiredAt: string;
  heartbeatAt: string;
  ttlSecs: number;
}

interface MockClaim {
  taskId: string;
  instanceId: string;
  claimedAt: string;
  heartbeatAt: string;
}

interface MockActivity {
  instanceId: string;
  projectId: string;
  action: string;
  details: Record<string, unknown>;
  timestamp: string;
}

interface MockUnstaged {
  instanceId: string;
  projectId: string;
  files: Array<{ path: string; status: string; layer: string }>;
  capturedAt: string;
}

function createMockHub(): { server: Server; port: number; start: () => Promise<number> } {
  const instances = new Map<string, MockInstance>();
  const locks = new Map<string, MockLock>();       // key = projectId:feature:layer
  const claims = new Map<string, MockClaim>();      // key = taskId
  const activities: MockActivity[] = [];
  const unstagedMap = new Map<string, MockUnstaged>(); // key = instanceId
  let idCounter = 0;

  function parseBody(req: IncomingMessage): Promise<any> {
    return new Promise((resolve) => {
      const chunks: Buffer[] = [];
      req.on('data', (c: Buffer) => chunks.push(c));
      req.on('end', () => {
        try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf-8'))); }
        catch { resolve({}); }
      });
    });
  }

  function json(res: ServerResponse, data: unknown, status = 200): void {
    const payload = JSON.stringify(data);
    res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) });
    res.end(payload);
  }

  function parseQuery(url: string): Record<string, string> {
    const idx = url.indexOf('?');
    if (idx === -1) return {};
    const params = new URLSearchParams(url.slice(idx + 1));
    const out: Record<string, string> = {};
    params.forEach((v, k) => { out[k] = v; });
    return out;
  }

  const server = createServer(async (req, res) => {
    const url = req.url ?? '/';
    const method = req.method ?? 'GET';
    const pathname = url.split('?')[0];

    // Version endpoint
    if (pathname === '/api/version' && method === 'GET') {
      return json(res, { version: '0.0.0-test' });
    }

    // Register instance
    if (pathname === '/api/coordination/instance/register' && method === 'POST') {
      const body = await parseBody(req);
      const instanceId = `inst-${++idCounter}`;
      const now = new Date().toISOString();
      instances.set(instanceId, {
        instanceId,
        projectId: body.projectId,
        pid: body.pid,
        sessionLabel: body.sessionLabel ?? '',
        registeredAt: now,
        lastSeen: now,
      });
      return json(res, { instanceId });
    }

    // Heartbeat
    if (pathname === '/api/coordination/instance/heartbeat' && method === 'POST') {
      const body = await parseBody(req);
      const inst = instances.get(body.instanceId);
      if (inst) inst.lastSeen = new Date().toISOString();
      if (body.unstagedFiles) {
        unstagedMap.set(body.instanceId, {
          instanceId: body.instanceId,
          projectId: body.projectId,
          files: body.unstagedFiles,
          capturedAt: new Date().toISOString(),
        });
      }
      return json(res, { ok: true });
    }

    // List instances
    if (pathname === '/api/coordination/instances' && method === 'GET') {
      const q = parseQuery(url);
      const list = [...instances.values()].filter((i) => !q.projectId || i.projectId === q.projectId);
      return json(res, list);
    }

    // Acquire lock
    if (pathname === '/api/coordination/worktree/lock' && method === 'POST') {
      const body = await parseBody(req);
      const key = `${body.projectId}:${body.feature}:${body.layer}`;
      const existing = locks.get(key);
      if (existing) {
        return json(res, { acquired: false, lock: null, conflict: existing });
      }
      const now = new Date().toISOString();
      const lock: MockLock = {
        instanceId: body.instanceId,
        projectId: body.projectId,
        feature: body.feature,
        layer: body.layer,
        acquiredAt: now,
        heartbeatAt: now,
        ttlSecs: body.ttlSecs ?? 300,
      };
      locks.set(key, lock);
      return json(res, { acquired: true, lock, conflict: null });
    }

    // List locks
    if (pathname === '/api/coordination/worktree/locks' && method === 'GET') {
      const q = parseQuery(url);
      const list = [...locks.values()].filter((l) => !q.projectId || l.projectId === q.projectId);
      return json(res, list);
    }

    // Release lock
    if (pathname.startsWith('/api/coordination/worktree/lock/') && method === 'DELETE') {
      const key = decodeURIComponent(pathname.slice('/api/coordination/worktree/lock/'.length));
      const existed = locks.delete(key);
      return json(res, { released: existed });
    }

    // Claim task
    if (pathname === '/api/coordination/task/claim' && method === 'POST') {
      const body = await parseBody(req);
      const existing = claims.get(body.taskId);
      if (existing) {
        return json(res, { claimed: false, claim: null, conflict: existing });
      }
      const now = new Date().toISOString();
      const claim: MockClaim = {
        taskId: body.taskId,
        instanceId: body.instanceId,
        claimedAt: now,
        heartbeatAt: now,
      };
      claims.set(body.taskId, claim);
      return json(res, { claimed: true, claim, conflict: null });
    }

    // Release task claim
    if (pathname.startsWith('/api/coordination/task/claim/') && method === 'DELETE') {
      const taskId = decodeURIComponent(pathname.slice('/api/coordination/task/claim/'.length));
      const existed = claims.delete(taskId);
      return json(res, { released: existed });
    }

    // Publish activity
    if (pathname === '/api/coordination/activity' && method === 'POST') {
      const body = await parseBody(req);
      activities.push({
        instanceId: body.instanceId,
        projectId: body.projectId,
        action: body.action,
        details: body.details ?? {},
        timestamp: new Date().toISOString(),
      });
      return json(res, { ok: true });
    }

    // Get activities
    if (pathname === '/api/coordination/activities' && method === 'GET') {
      const q = parseQuery(url);
      let list = [...activities].filter((a) => !q.projectId || a.projectId === q.projectId);
      if (q.limit) list = list.slice(-Number(q.limit));
      return json(res, list);
    }

    // Get unstaged
    if (pathname === '/api/coordination/unstaged' && method === 'GET') {
      const q = parseQuery(url);
      const list = [...unstagedMap.values()].filter((u) => !q.projectId || u.projectId === q.projectId);
      return json(res, list);
    }

    // Fallback
    json(res, { error: 'not found' }, 404);
  });

  let assignedPort = 0;

  return {
    server,
    get port() { return assignedPort; },
    start: () =>
      new Promise<number>((resolve, reject) => {
        server.listen(0, '127.0.0.1', () => {
          const addr = server.address();
          if (addr && typeof addr !== 'string') {
            assignedPort = addr.port;
            resolve(assignedPort);
          } else {
            reject(new Error('Failed to bind'));
          }
        });
      }),
  };
}

// ── HTTP helpers ────────────────────────────────────────

import { request } from 'node:http';

let HUB_PORT = 0;

function post(path: string, body: unknown): Promise<any> {
  return new Promise((resolve, reject) => {
    const payload = JSON.stringify(body);
    const req = request(
      { hostname: '127.0.0.1', port: HUB_PORT, path, method: 'POST',
        headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) },
        timeout: 5000 },
      (res) => {
        const chunks: Buffer[] = [];
        res.on('data', (c: Buffer) => chunks.push(c));
        res.on('end', () => {
          try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf-8'))); }
          catch { resolve(null); }
        });
      },
    );
    req.on('error', reject);
    req.on('timeout', () => { req.destroy(); reject(new Error('timeout')); });
    req.end(payload);
  });
}

function get(path: string): Promise<any> {
  return new Promise((resolve, reject) => {
    const req = request(
      { hostname: '127.0.0.1', port: HUB_PORT, path, method: 'GET', timeout: 5000 },
      (res) => {
        const chunks: Buffer[] = [];
        res.on('data', (c: Buffer) => chunks.push(c));
        res.on('end', () => {
          try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf-8'))); }
          catch { resolve(null); }
        });
      },
    );
    req.on('error', reject);
    req.on('timeout', () => { req.destroy(); reject(new Error('timeout')); });
    req.end();
  });
}

function del(path: string): Promise<any> {
  return new Promise((resolve, reject) => {
    const req = request(
      { hostname: '127.0.0.1', port: HUB_PORT, path, method: 'DELETE', timeout: 5000 },
      (res) => {
        const chunks: Buffer[] = [];
        res.on('data', (c: Buffer) => chunks.push(c));
        res.on('end', () => {
          try { resolve(JSON.parse(Buffer.concat(chunks).toString('utf-8'))); }
          catch { resolve(null); }
        });
      },
    );
    req.on('error', reject);
    req.on('timeout', () => { req.destroy(); reject(new Error('timeout')); });
    req.end();
  });
}

// ── Test suite ──────────────────────────────────────────

const PROJECT_ID = 'test-coordination';
let hub: ReturnType<typeof createMockHub>;

beforeAll(async () => {
  hub = createMockHub();
  HUB_PORT = await hub.start();
});

afterAll(() => {
  hub.server.close();
});

describe('Coordination Hub Integration', () => {
  let instanceId1: string;
  let instanceId2: string;

  // B1: Register instances
  it('registers an instance and returns instanceId', async () => {
    const result = await post('/api/coordination/instance/register', {
      projectId: PROJECT_ID,
      pid: process.pid,
      sessionLabel: 'test-instance-1',
    });
    expect(result.instanceId).toBeDefined();
    expect(typeof result.instanceId).toBe('string');
    expect(result.instanceId.length).toBeGreaterThan(0);
    instanceId1 = result.instanceId;
  });

  it('registers a second instance with different id', async () => {
    const result = await post('/api/coordination/instance/register', {
      projectId: PROJECT_ID,
      pid: process.pid + 1,
      sessionLabel: 'test-instance-2',
    });
    expect(result.instanceId).toBeDefined();
    expect(result.instanceId).not.toBe(instanceId1);
    instanceId2 = result.instanceId;
  });

  // List instances
  it('lists registered instances for the project', async () => {
    const list = await get(`/api/coordination/instances?projectId=${PROJECT_ID}`);
    expect(Array.isArray(list)).toBe(true);
    const ids = list.map((i: any) => i.instanceId);
    expect(ids).toContain(instanceId1);
    expect(ids).toContain(instanceId2);
  });

  // B2: Acquire lock — success
  it('acquires a lock on a free worktree', async () => {
    const result = await post('/api/coordination/worktree/lock', {
      instanceId: instanceId1,
      projectId: PROJECT_ID,
      feature: 'auth',
      layer: 'domain',
      ttlSecs: 60,
    });
    expect(result.acquired).toBe(true);
    expect(result.lock).not.toBeNull();
    expect(result.lock.feature).toBe('auth');
    expect(result.lock.layer).toBe('domain');
    expect(result.conflict).toBeNull();
  });

  // B3: Acquire lock — conflict
  it('returns conflict when lock is already held', async () => {
    const result = await post('/api/coordination/worktree/lock', {
      instanceId: instanceId2,
      projectId: PROJECT_ID,
      feature: 'auth',
      layer: 'domain',
    });
    expect(result.acquired).toBe(false);
    expect(result.lock).toBeNull();
    expect(result.conflict).not.toBeNull();
    expect(result.conflict.instanceId).toBe(instanceId1);
  });

  // List locks
  it('lists locks for the project', async () => {
    const list = await get(`/api/coordination/worktree/locks?projectId=${PROJECT_ID}`);
    expect(Array.isArray(list)).toBe(true);
    expect(list.length).toBeGreaterThanOrEqual(1);
    expect(list[0].feature).toBe('auth');
  });

  // B4: Release lock
  it('releases a lock and allows re-acquisition', async () => {
    const key = encodeURIComponent(`${PROJECT_ID}:auth:domain`);
    const releaseResult = await del(`/api/coordination/worktree/lock/${key}`);
    expect(releaseResult.released).toBe(true);

    // Now instance2 can acquire it
    const result = await post('/api/coordination/worktree/lock', {
      instanceId: instanceId2,
      projectId: PROJECT_ID,
      feature: 'auth',
      layer: 'domain',
      ttlSecs: 60,
    });
    expect(result.acquired).toBe(true);
    expect(result.lock.instanceId).toBe(instanceId2);

    // Cleanup
    await del(`/api/coordination/worktree/lock/${key}`);
  });

  // B5: Claim task — success
  it('claims an unclaimed task', async () => {
    const result = await post('/api/coordination/task/claim', {
      instanceId: instanceId1,
      projectId: PROJECT_ID,
      taskId: 'task-integration-1',
    });
    expect(result.claimed).toBe(true);
    expect(result.claim.taskId).toBe('task-integration-1');
    expect(result.conflict).toBeNull();
  });

  // B6: Claim task — conflict
  it('returns conflict when task is already claimed', async () => {
    const result = await post('/api/coordination/task/claim', {
      instanceId: instanceId2,
      projectId: PROJECT_ID,
      taskId: 'task-integration-1',
    });
    expect(result.claimed).toBe(false);
    expect(result.claim).toBeNull();
    expect(result.conflict.instanceId).toBe(instanceId1);
  });

  // Release task
  it('releases a task claim', async () => {
    const result = await del('/api/coordination/task/claim/task-integration-1');
    expect(result.released).toBe(true);
  });

  // Activity stream
  it('publishes and retrieves activity', async () => {
    await post('/api/coordination/activity', {
      instanceId: instanceId1,
      projectId: PROJECT_ID,
      action: 'test-action',
      details: { key: 'value' },
    });

    const list = await get(`/api/coordination/activities?projectId=${PROJECT_ID}&limit=5`);
    expect(Array.isArray(list)).toBe(true);
    expect(list.length).toBeGreaterThanOrEqual(1);
    const found = list.find((a: any) => a.action === 'test-action');
    expect(found).toBeDefined();
    expect(found.instanceId).toBe(instanceId1);
  });

  // B7: Heartbeat with unstaged files
  it('heartbeat updates lastSeen and pushes unstaged files', async () => {
    const hbResult = await post('/api/coordination/instance/heartbeat', {
      instanceId: instanceId1,
      projectId: PROJECT_ID,
      unstagedFiles: [
        { path: 'src/core/domain/entities.ts', status: 'modified', layer: 'domain' },
        { path: 'src/adapters/primary/cli.ts', status: 'added', layer: 'primary-adapter' },
      ],
    });
    expect(hbResult.ok).toBe(true);

    // Verify unstaged files appear
    const unstaged = await get(`/api/coordination/unstaged?projectId=${PROJECT_ID}`);
    expect(Array.isArray(unstaged)).toBe(true);
    const entry = unstaged.find((u: any) => u.instanceId === instanceId1);
    expect(entry).toBeDefined();
    expect(entry.files).toHaveLength(2);
    expect(entry.files[0].layer).toBe('domain');
  });
});
