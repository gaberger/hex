/**
 * Unit tests for CoordinationAdapter (secondary adapter)
 *
 * London-school TDD: mocks HTTP transport to test coordination logic
 * without requiring a running hex-hub instance.
 */

import { describe, it, expect, mock, beforeEach, afterEach } from 'bun:test';
import { EventEmitter } from 'node:events';

// ── Mock state ───────────────────────────────────────────

let httpResponseBody: unknown = {};
let httpShouldError = false;
const httpRequests: Array<{ path: string; method: string; body: unknown }> = [];

let execFileCalls: Array<{ cmd: string; args: string[] }> = [];
let execFileResult = { stdout: '', stderr: '' };
let execFileShouldError = false;

// ── Module mocks ─────────────────────────────────────────

mock.module('node:fs', () => ({
  readFileSync: () => '{"token":"test-token"}',
  watch: () => ({ close() {}, unref() {} }),
}));

mock.module('node:http', () => ({
  request: (_opts: any, cb: any) => {
    const method = _opts.method || 'GET';
    const path = _opts.path || '/';

    // Capture body from req.end(payload)
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

      // Simulate response
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
  },
}));

mock.module('node:child_process', () => ({
  execFile: (cmd: string, args: string[], _opts: any, cb: any) => {
    execFileCalls.push({ cmd, args });
    if (execFileShouldError) {
      cb(new Error('git failed'));
    } else {
      cb(null, execFileResult);
    }
  },
}));

mock.module('node:util', () => ({
  promisify: (fn: any) => (...args: any[]) => new Promise((resolve, reject) => {
    const allArgs = [...args.slice(0, -1)]; // strip options
    fn(...args, (err: any, result: any) => {
      if (err) reject(err);
      else resolve(result);
    });
  }),
}));

// ── Import after mocks ──────────────────────────────────

const { CoordinationAdapter } = await import('../../src/adapters/secondary/coordination-adapter.js');

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
    adapter = new CoordinationAdapter('proj-123', '/tmp/test', 5555);
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
    it('sends provided unstaged files', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      httpResponseBody = { ok: true };
      const files = [{ path: 'src/core/domain/foo.ts', status: 'modified' as const, layer: 'domain' }];
      await adapter.heartbeat(files);

      const req = httpRequests.find(r => r.path === '/api/coordination/instance/heartbeat');
      expect(req).toBeDefined();
      expect((req!.body as any).unstagedFiles).toHaveLength(1);
      expect((req!.body as any).unstagedFiles[0].path).toBe('src/core/domain/foo.ts');
    });

    it('skips heartbeat when instance is not registered', async () => {
      await adapter.heartbeat();
      const req = httpRequests.find(r => r.path?.includes('heartbeat'));
      expect(req).toBeUndefined();
    });
  });

  // ── B10: classifyLayer (via captureUnstagedFiles) ────

  describe('captureUnstagedFiles (layer classification)', () => {
    it('classifies domain files correctly', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      execFileResult = {
        stdout: ' M src/core/domain/entities.ts\n?? src/adapters/primary/cli.ts\n D src/core/ports/index.ts\n',
        stderr: '',
      };
      httpResponseBody = { ok: true };
      await adapter.heartbeat(); // triggers captureUnstagedFiles

      const hbReq = httpRequests.filter(r => r.path === '/api/coordination/instance/heartbeat');
      const lastHb = hbReq[hbReq.length - 1];
      const files = (lastHb?.body as any)?.unstagedFiles;
      expect(files).toBeDefined();
      expect(files.length).toBe(3);

      const domainFile = files.find((f: any) => f.path === 'src/core/domain/entities.ts');
      expect(domainFile.layer).toBe('domain');
      expect(domainFile.status).toBe('modified');

      const primaryFile = files.find((f: any) => f.path === 'src/adapters/primary/cli.ts');
      expect(primaryFile.layer).toBe('primary-adapter');
      expect(primaryFile.status).toBe('added');

      const portFile = files.find((f: any) => f.path === 'src/core/ports/index.ts');
      expect(portFile.layer).toBe('port');
      expect(portFile.status).toBe('deleted');
    });

    it('returns empty array when git fails', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      execFileShouldError = true;
      httpResponseBody = { ok: true };
      await adapter.heartbeat();

      const hbReq = httpRequests.filter(r => r.path === '/api/coordination/instance/heartbeat');
      const lastHb = hbReq[hbReq.length - 1];
      expect((lastHb?.body as any)?.unstagedFiles).toEqual([]);
    });
  });

  // ── Activity & Unstaged queries ──────────────────────

  describe('publishActivity', () => {
    it('sends action and details to hub', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      httpResponseBody = { ok: true };
      await adapter.publishActivity('lock-acquired', { feature: 'auth' });

      const req = httpRequests.find(r => r.path === '/api/coordination/activity');
      expect(req).toBeDefined();
      expect((req!.body as any).action).toBe('lock-acquired');
      expect((req!.body as any).details.feature).toBe('auth');
    });

    it('skips when instance is not registered', async () => {
      await adapter.publishActivity('test');
      const req = httpRequests.find(r => r.path?.includes('activity'));
      expect(req).toBeUndefined();
    });
  });

  describe('listLocks', () => {
    it('queries hub with projectId', async () => {
      httpResponseBody = [];
      const result = await adapter.listLocks();
      expect(result).toEqual([]);
      const req = httpRequests.find(r => r.path?.includes('/api/coordination/worktree/locks'));
      expect(req).toBeDefined();
      expect(req!.path).toContain('proj-123');
    });
  });

  describe('releaseTask', () => {
    it('sends DELETE for the task', async () => {
      await adapter.releaseTask('task-99');
      const req = httpRequests.find(r => r.method === 'DELETE' && r.path?.includes('task-99'));
      expect(req).toBeDefined();
    });
  });

  describe('stop', () => {
    it('clears heartbeat timer without error', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();
      adapter.stop();
      // Double-stop should not throw
      adapter.stop();
    });
  });

  describe('HTTP error resilience', () => {
    it('acquireLock returns safe default when hub is unreachable', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();

      httpShouldError = true;
      const result = await adapter.acquireLock('feat', 'layer');
      expect(result.acquired).toBe(false);
    });
  });

  describe('getActivities', () => {
    it('passes limit as query parameter', async () => {
      httpResponseBody = { instanceId: 'inst-1' };
      await adapter.registerInstance();
      httpResponseBody = [];
      const result = await adapter.getActivities(5);
      expect(result).toEqual([]);
      const req = httpRequests.find(r => r.path?.includes('/api/coordination/activities'));
      expect(req).toBeDefined();
      expect(req!.path).toContain('limit=5');
      expect(req!.path).toContain('proj-123');
    });
  });

  describe('listClaims', () => {
    it('queries hub with projectId', async () => {
      httpResponseBody = [];
      const result = await adapter.listClaims();
      expect(result).toEqual([]);
      const req = httpRequests.find(r => r.path?.includes('/api/coordination/task/claims'));
      expect(req).toBeDefined();
      expect(req!.path).toContain('proj-123');
    });
  });

  describe('getUnstagedAcrossInstances', () => {
    it('queries hub with projectId', async () => {
      httpResponseBody = [];
      const result = await adapter.getUnstagedAcrossInstances();
      expect(result).toEqual([]);
      const req = httpRequests.find(r => r.path?.includes('/api/coordination/unstaged'));
      expect(req).toBeDefined();
      expect(req!.path).toContain('proj-123');
    });
  });
});
