/**
 * Integration Test — Coordination endpoints against live hex-hub
 *
 * Requires hex-hub running on port 5555.
 * Tests the full lock/claim/activity/unstaged lifecycle via HTTP.
 */

import { describe, it, expect, beforeAll, afterAll } from 'bun:test';
import { request } from 'node:http';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';

const HUB_PORT = 5555;
const PROJECT_ID = 'test-coordination';

// Read auth token from hub lock file
let authToken = '';
try {
  const lock = JSON.parse(readFileSync(join(homedir(), '.hex', 'daemon', 'hub.lock'), 'utf-8'));
  authToken = lock.token ?? '';
} catch { /* no lock file — try without auth */ }

function authHeaders(): Record<string, string> {
  return authToken ? { 'Authorization': `Bearer ${authToken}` } : {};
}

// ── HTTP helpers ────────────────────────────────────────

function post(path: string, body: unknown): Promise<any> {
  return new Promise((resolve, reject) => {
    const payload = JSON.stringify(body);
    const req = request(
      { hostname: '127.0.0.1', port: HUB_PORT, path, method: 'POST',
        headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload), ...authHeaders() },
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
      { hostname: '127.0.0.1', port: HUB_PORT, path, method: 'GET', headers: authHeaders(), timeout: 5000 },
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
      { hostname: '127.0.0.1', port: HUB_PORT, path, method: 'DELETE', headers: authHeaders(), timeout: 5000 },
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

// ── Hub availability check ──────────────────────────────

let hubAvailable = false;

beforeAll(async () => {
  try {
    const version = await get('/api/version');
    hubAvailable = !!version?.version;
  } catch {
    hubAvailable = false;
  }
});

// ── Tests ───────────────────────────────────────────────

describe('Coordination Hub Integration', () => {
  let instanceId1: string;
  let instanceId2: string;

  // B1: Register instances
  it('registers an instance and returns instanceId', async () => {
    if (!hubAvailable) return; // skip silently if hub not running
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
    if (!hubAvailable) return;
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
    if (!hubAvailable) return;
    const list = await get(`/api/coordination/instances?projectId=${PROJECT_ID}`);
    expect(Array.isArray(list)).toBe(true);
    const ids = list.map((i: any) => i.instanceId);
    expect(ids).toContain(instanceId1);
    expect(ids).toContain(instanceId2);
  });

  // B2: Acquire lock — success
  it('acquires a lock on a free worktree', async () => {
    if (!hubAvailable) return;
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
    if (!hubAvailable) return;
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
    if (!hubAvailable) return;
    const list = await get(`/api/coordination/worktree/locks?projectId=${PROJECT_ID}`);
    expect(Array.isArray(list)).toBe(true);
    expect(list.length).toBeGreaterThanOrEqual(1);
    expect(list[0].feature).toBe('auth');
  });

  // B4: Release lock
  it('releases a lock and allows re-acquisition', async () => {
    if (!hubAvailable) return;
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
    if (!hubAvailable) return;
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
    if (!hubAvailable) return;
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
    if (!hubAvailable) return;
    const result = await del('/api/coordination/task/claim/task-integration-1');
    expect(result.released).toBe(true);
  });

  // Activity stream
  it('publishes and retrieves activity', async () => {
    if (!hubAvailable) return;
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
    if (!hubAvailable) return;
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
