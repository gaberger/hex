/**
 * Unit tests for CoordinationAdapter (secondary adapter)
 *
 * Uses dependency injection (CoordinationAdapterDeps) instead of mock.module()
 * to avoid permanent process-global module replacement that contaminates
 * other test files running in the same Bun process.
 */

import { describe, it, expect, mock, beforeEach, afterEach } from 'bun:test';
import { EventEmitter } from 'node:events';
import {
  CoordinationAdapter,
  type CoordinationAdapterDeps,
} from '../../src/adapters/secondary/coordination-adapter.js';

// ── Mock state ───────────────────────────────────────────

let httpResponseBody: unknown = {};
let httpShouldError = false;
const httpRequests: Array<{ path: string; method: string; body: unknown }> = [];

let execFileCalls: Array<{ cmd: string; args: string[] }> = [];
let execFileResult = { stdout: '', stderr: '' };
let execFileShouldError = false;

// ── Injectable fake HTTP request ─────────────────────────

function createFakeHttpRequest() {
  return (_opts: any, cb: any) => {
    const method = _opts.method || 'GET';
    const path = _opts.path || '/';

    const fakeReq = new EventEmitter() as any;
    fakeReq.destroy = mock(() => {});
    fakeReq.end = (payload?: string) => {
      let body: unknown = null;
      if (payload) try { body = JSON.parse(payload); } catch { body = payload; }
      httpRequests.push({ path, method, body });

      if (httpShouldError) {
        setTimeout(() => fakeReq.emit('error', new Error('connection refused')), 0);
        return;
      }

      const responseBody = Buffer.from(JSON.stringify(httpResponseBody));
      const fakeRes = new EventEmitter() as any;
      fakeRes.statusCode = 200;
      setTimeout(() => {
        cb(fakeRes);
        fakeRes.emit('data', responseBody);
        fakeRes.emit('end');
      }, 0);
    };
    return fakeReq;
  };
}

// ── Injectable fake execFile ─────────────────────────────

function createFakeExecFile() {
  return async (cmd: string, args: string[], _opts: any) => {
    execFileCalls.push({ cmd, args });
    if (execFileShouldError) throw new Error('git failed');
    return execFileResult;
  };
}

// ── Build deps helper ────────────────────────────────────

function makeDeps(overrides: Partial<CoordinationAdapterDeps> = {}): CoordinationAdapterDeps {
  return {
    httpRequest: createFakeHttpRequest() as any,
    execFileAsync: createFakeExecFile(),
    authToken: 'test-token',
    ...overrides,
  };
}

// ── Helpers ─────────────────────────────────────────────

function lastRequest() {
  return httpRequests[httpRequests.length - 1];
}

// ── Tests ───────────────────────────────────────────────

describe('CoordinationAdapter', () => {
  let adapter: InstanceType<typeof CoordinationAdapter>;

  beforeEach(() => {
    httpRequests.length = 0;
    execFileCalls = [];
    httpResponseBody = {};
    httpShouldError = false;
    execFileShouldError = false;
    execFileResult = { stdout: '', stderr: '' };
    adapter = new CoordinationAdapter('proj-123', '/tmp/test', 5555, makeDeps());
  });

  afterEach(() => {
    adapter.stop();
  });

  // ── B1: registerInstance ──────────────────────────────

  describe('registerInstance', () => {
    it('returns the instanceId from hub response', async () => {
      httpResponseBody = { instanceId: 'abc-123-def' };
      const id = await adapter.registerInstance('my-session');
      expect(id).toBe('abc-123-def');
    });

    it('sends projectId, pid, and sessionLabel', async () => {
      httpResponseBody = { instanceId: 'x' };
      await adapter.registerInstance('test-label');
      const req = httpRequests.find(r => r.path === '/api/coordination/instance/register');
      expect(req).toBeDefined();
      expect((req!.body as any).projectId).toBe('proj-123');
      expect((req!.body as any).pid).toBe(process.pid);
      expect((req!.body as any).sessionLabel).toBe('test-label');
    });

    it('throws when hub returns no instanceId', async () => {
      httpResponseBody = {};
      await expect(adapter.registerInstance()).rejects.toThrow('Failed to register');
    });

    it('uses default sessionLabel when none provided', async () => {
      httpResponseBody = { instanceId: 'x' };
      await adapter.registerInstance();
      const req = httpRequests.find(r => r.path === '/api/coordination/instance/register');
      expect((req!.body as any).sessionLabel).toMatch(/^session-\d+$/);
    });
  });

  // ── B2: acquireLock success ──────────────────────────

  describe('acquireLock', () => {
    it('returns acquired=true when lock is free', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      const lockData = {
        instanceId: 'inst-1',
        projectId: 'proj-123',
        feature: 'auth',
        layer: 'domain',
        acquiredAt: '2026-03-17T00:00:00Z',
        heartbeatAt: '2026-03-17T00:00:00Z',
        ttlSecs: 300,
      };
      httpResponseBody = { acquired: true, lock: lockData, conflict: null };
      const result = await adapter.acquireLock('auth', 'domain');

      expect(result.acquired).toBe(true);
      expect(result.lock).not.toBeNull();
      expect(result.lock!.feature).toBe('auth');
      expect(result.conflict).toBeNull();
    });

    // ── B3: acquireLock conflict ──────────────────────

    it('returns acquired=false with conflict when lock is held', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      const conflictLock = {
        instanceId: 'other-inst',
        projectId: 'proj-123',
        feature: 'auth',
        layer: 'domain',
        acquiredAt: '2026-03-17T00:00:00Z',
        heartbeatAt: '2026-03-17T00:00:00Z',
        ttlSecs: 300,
      };
      httpResponseBody = { acquired: false, lock: null, conflict: conflictLock };
      const result = await adapter.acquireLock('auth', 'domain');

      expect(result.acquired).toBe(false);
      expect(result.lock).toBeNull();
      expect(result.conflict).not.toBeNull();
      expect(result.conflict!.instanceId).toBe('other-inst');
    });

    it('throws when instance is not registered', async () => {
      await expect(adapter.acquireLock('feat', 'layer')).rejects.toThrow('Instance not registered');
    });
  });

  // ── B4: releaseLock ──────────────────────────────────

  describe('releaseLock', () => {
    it('sends DELETE to the correct key-encoded path', async () => {
      await adapter.releaseLock('auth', 'domain');
      const req = httpRequests.find(r => r.method === 'DELETE');
      expect(req).toBeDefined();
      expect(req!.path).toContain('proj-123');
      expect(req!.path).toContain('auth');
      expect(req!.path).toContain('domain');
    });
  });

  // ── B5: claimTask success ────────────────────────────

  describe('claimTask', () => {
    it('returns claimed=true when task is unclaimed', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      httpResponseBody = {
        claimed: true,
        claim: { taskId: 'task-42', instanceId: 'inst-1', claimedAt: '2026-03-17T00:00:00Z', heartbeatAt: '2026-03-17T00:00:00Z' },
        conflict: null,
      };
      const result = await adapter.claimTask('task-42');
      expect(result.claimed).toBe(true);
      expect(result.claim!.taskId).toBe('task-42');
    });

    // ── B6: claimTask conflict ─────────────────────────

    it('returns claimed=false with conflict when already claimed', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      httpResponseBody = {
        claimed: false,
        claim: null,
        conflict: { taskId: 'task-42', instanceId: 'other-inst', claimedAt: '2026-03-17T00:00:00Z', heartbeatAt: '2026-03-17T00:00:00Z' },
      };
      const result = await adapter.claimTask('task-42');
      expect(result.claimed).toBe(false);
      expect(result.conflict!.instanceId).toBe('other-inst');
    });

    it('throws when instance is not registered', async () => {
      await expect(adapter.claimTask('task-1')).rejects.toThrow('Instance not registered');
    });
  });

  // ── B7: heartbeat ────────────────────────────────────

  describe('heartbeat', () => {
    it('auto-captures git status when no files provided', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      execFileResult = { stdout: ' M src/core/domain/foo.ts\n?? src/adapters/primary/new.ts\n', stderr: '' };
      httpResponseBody = { ok: true };
      await adapter.heartbeat();

      const hbReq = httpRequests.find(r => r.path === '/api/coordination/instance/heartbeat');
      expect(hbReq).toBeDefined();
      expect((hbReq!.body as any).instanceId).toBe('inst-1');

      const files = (hbReq!.body as any).unstagedFiles;
      expect(files.length).toBe(2);
      expect(files[0].path).toBe('src/core/domain/foo.ts');
      expect(files[0].status).toBe('modified');
      expect(files[0].layer).toBe('domain');
      expect(files[1].status).toBe('added');
    });

    it('uses provided files instead of git capture', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      const files = [{ path: 'a.ts', status: 'modified' as const, layer: 'domain' }];
      httpResponseBody = { ok: true };
      await adapter.heartbeat(files);

      const hbReq = httpRequests.find(r => r.path === '/api/coordination/instance/heartbeat');
      expect((hbReq!.body as any).unstagedFiles).toEqual(files);
    });

    it('silently ignores heartbeat when not registered', async () => {
      await adapter.heartbeat();
      const hbReq = httpRequests.find(r => r.path === '/api/coordination/instance/heartbeat');
      expect(hbReq).toBeUndefined();
    });
  });

  // ── B8: publishActivity ────────────────────────────

  describe('publishActivity', () => {
    it('sends activity to hub', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      httpResponseBody = { ok: true };
      await adapter.publishActivity('task-start', { taskId: 't1' });

      const req = httpRequests.find(r => r.path === '/api/coordination/activity');
      expect(req).toBeDefined();
      expect((req!.body as any).action).toBe('task-start');
      expect((req!.body as any).details).toEqual({ taskId: 't1' });
    });
  });

  // ── B9: stop ──────────────────────────────────────────

  describe('stop', () => {
    it('clears heartbeat timer without throwing', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      // stop should not throw
      expect(() => adapter.stop()).not.toThrow();
    });
  });
});
