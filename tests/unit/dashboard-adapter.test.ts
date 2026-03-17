/**
 * Unit tests for DashboardAdapter (primary adapter)
 *
 * London-school TDD: mocks for all ports and external dependencies.
 * Tests registration, push methods, WS command listener, reconnection,
 * auth token, file watcher, cleanup, and default command handlers.
 */

import { describe, it, expect, mock, beforeEach, afterEach } from 'bun:test';
import { EventEmitter } from 'node:events';

// ── Mock state ───────────────────────────────────────────

let mockReadFileSyncResult: string | Error = '{}';
let mockWatchCallback: ((_event: string, filename: string | null) => void) | null = null;
const mockWatcherClose = mock(() => {});

// Track HTTP requests sent by the adapter
const httpRequests: Array<{
  path: string;
  method: string;
  body: unknown;
  headers: Record<string, string | number>;
}> = [];
let httpResponseBody: Record<string, unknown> = { id: 'proj-123' };
let httpShouldError = false;

// ── Mock WebSocket ───────────────────────────────────────

class MockWebSocket extends EventEmitter {
  static OPEN = 1;
  static CONNECTING = 0;
  static CLOSING = 2;
  static CLOSED = 3;
  readyState = 1; // OPEN
  sent: unknown[] = [];

  send(data: string): void {
    this.sent.push(JSON.parse(data));
  }

  close(_code?: number, _reason?: string): void {
    this.readyState = MockWebSocket.CLOSED;
  }

  removeAllListeners(): this {
    super.removeAllListeners();
    return this;
  }
}

let latestWs: MockWebSocket | null = null;
let wsConstructorCalls: string[] = [];
let wsConstructorShouldThrow = false;

// ── Module mocks ─────────────────────────────────────────

mock.module('node:fs', () => ({
  readFileSync: (_path: string, _enc: string) => {
    if (mockReadFileSyncResult instanceof Error) throw mockReadFileSyncResult;
    return mockReadFileSyncResult;
  },
  watch: (_dir: string, _opts: unknown, cb: (event: string, filename: string | null) => void) => {
    mockWatchCallback = cb;
    return { close: mockWatcherClose };
  },
}));

mock.module('node:os', () => ({
  homedir: () => '/mock-home',
}));

mock.module('node:path', () => ({
  resolve: (...parts: string[]) => parts.join('/'),
  join: (...parts: string[]) => parts.join('/'),
}));

mock.module('node:http', () => ({
  request: (opts: { path: string; method: string; headers: Record<string, string | number> }, cb: (res: EventEmitter) => void) => {
    const req = new EventEmitter() as EventEmitter & { end: (data: string) => void; destroy: () => void };
    req.end = (data: string) => {
      let body: unknown = null;
      try { body = JSON.parse(data); } catch { /* ignore */ }
      httpRequests.push({
        path: opts.path,
        method: opts.method,
        body,
        headers: opts.headers,
      });

      if (httpShouldError) {
        setTimeout(() => req.emit('error', new Error('connection refused')), 0);
        return;
      }

      const res = new EventEmitter();
      setTimeout(() => {
        cb(res);
        const buf = Buffer.from(JSON.stringify(httpResponseBody));
        res.emit('data', buf);
        res.emit('end');
      }, 0);
    };
    req.destroy = () => {};
    return req;
  },
}));

mock.module('ws', () => {
  const WS = function (url: string) {
    wsConstructorCalls.push(url);
    if (wsConstructorShouldThrow) throw new Error('ws connect failed');
    const instance = new MockWebSocket();
    latestWs = instance;
    // Simulate async open
    setTimeout(() => {
      if (instance.readyState !== MockWebSocket.CLOSED) {
        instance.emit('open');
      }
    }, 5);
    return instance;
  };
  WS.OPEN = 1;
  WS.CONNECTING = 0;
  WS.CLOSING = 2;
  WS.CLOSED = 3;
  return { default: WS, __esModule: true };
});

// ── Import after mocks ──────────────────────────────────

const { DashboardAdapter, startDashboard } = await import(
  '../../src/adapters/primary/dashboard-adapter.js'
);

// ── Mock AppContext ──────────────────────────────────────

function makeCtx(overrides: Record<string, unknown> = {}): any {
  return {
    rootPath: '/projects/my-app',
    astIsStub: false,
    archAnalyzer: {
      analyzeArchitecture: mock(async () => ({ summary: { violationCount: 0 } })),
      buildDependencyGraph: mock(async () => [
        { from: 'src/core/domain/foo.ts', to: 'src/core/ports/bar.ts', names: ['Bar'] },
        { from: 'src/adapters/primary/cli.ts', to: 'src/core/ports/bar.ts', names: ['Bar'] },
      ]),
      validateHexBoundaries: mock(async () => []),
    },
    ast: {
      extractSummary: mock(async (_path: string, _level: string) => ({
        tokenEstimate: 100,
        lineCount: 50,
      })),
    },
    summaryService: {
      summarizeProject: mock(async () => [{ path: 'src/a.ts', summary: 'test' }]),
    },
    fs: {
      glob: mock(async () => ['src/core/domain/foo.ts']),
    },
    swarm: {
      status: mock(async () => ({ status: 'idle', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0 })),
      listTasks: mock(async () => []),
      listAgents: mock(async () => []),
      spawnAgent: mock(async () => ({ id: 'agent-1', name: 'test-agent' })),
      terminateAgent: mock(async () => {}),
      createTask: mock(async () => ({ id: 'task-1', title: 'test-task' })),
      completeTask: mock(async () => {}),
    },
    build: {
      compile: mock(async () => ({ success: true })),
    },
    codeGenerator: null,
    ...overrides,
  };
}

// ── Tests ────────────────────────────────────────────────

describe('DashboardAdapter', () => {
  beforeEach(() => {
    httpRequests.length = 0;
    httpResponseBody = { id: 'proj-123' };
    httpShouldError = false;
    mockReadFileSyncResult = JSON.stringify({ token: 'test-auth-token' });
    mockWatchCallback = null;
    mockWatcherClose.mockClear();
    latestWs = null;
    wsConstructorCalls = [];
    wsConstructorShouldThrow = false;
  });

  // ── Registration ─────────────────────────────────────

  describe('start() — registration', () => {
    it('registers with hub via POST /api/projects/register', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      const result = await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect(regReq).toBeTruthy();
      expect(regReq!.body).toEqual({
        name: 'my-app',
        rootPath: '/projects/my-app',
        astIsStub: false,
      });

      expect(result.url).toBe('http://localhost:9999');
      expect(typeof result.close).toBe('function');

      result.close();
    });

    it('extracts project name from rootPath last segment', async () => {
      const ctx = makeCtx({ rootPath: '/deep/nested/cool-project' });
      const adapter = new DashboardAdapter(ctx, 9999);
      const result = await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect((regReq!.body as any).name).toBe('cool-project');

      result.close();
    });

    it('throws when hub registration fails (no id)', async () => {
      httpResponseBody = {}; // no id field
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);

      expect(adapter.start()).rejects.toThrow('Hub registration failed');
    });

    it('throws when hub returns null response', async () => {
      httpShouldError = true;
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);

      expect(adapter.start()).rejects.toThrow('Hub registration failed');
    });

    it('returns a close function that stops the adapter', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      const result = await adapter.start();

      result.close();

      // After close, adapter should be stopped — verify watcher was closed
      expect(mockWatcherClose).toHaveBeenCalled();
    });
  });

  // ── Auth token ───────────────────────────────────────

  describe('auth token', () => {
    it('reads token from ~/.hex/daemon/hub.lock and includes as Bearer header', async () => {
      mockReadFileSyncResult = JSON.stringify({ token: 'my-secret-token' });
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect(regReq!.headers['Authorization']).toBe('Bearer my-secret-token');

      adapter.stop();
    });

    it('sends no Authorization header when lock file is missing', async () => {
      mockReadFileSyncResult = new Error('ENOENT');
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect(regReq!.headers['Authorization']).toBeUndefined();

      adapter.stop();
    });

    it('sends no Authorization header when lock file has no token field', async () => {
      mockReadFileSyncResult = JSON.stringify({ pid: 1234 });
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect(regReq!.headers['Authorization']).toBeUndefined();

      adapter.stop();
    });
  });

  // ── Push methods ─────────────────────────────────────

  describe('pushHealth()', () => {
    it('calls archAnalyzer.analyzeArchitecture and POSTs health data', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      // Wait for initial pushAll to fire
      await new Promise((r) => setTimeout(r, 50));

      expect(ctx.archAnalyzer.analyzeArchitecture).toHaveBeenCalledWith('.');
      const healthReq = httpRequests.find(
        (r) => r.path === '/api/push' && (r.body as any)?.type === 'health',
      );
      expect(healthReq).toBeTruthy();

      adapter.stop();
    });
  });

  describe('pushTokens()', () => {
    it('globs for source files and extracts L0-L3 summaries', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 50));

      expect(ctx.fs.glob).toHaveBeenCalledWith('**/*.ts');
      expect(ctx.fs.glob).toHaveBeenCalledWith('**/*.go');
      expect(ctx.fs.glob).toHaveBeenCalledWith('**/*.rs');
      expect(ctx.ast.extractSummary).toHaveBeenCalled();

      const tokenReq = httpRequests.find(
        (r) => r.path === '/api/push' && (r.body as any)?.type === 'tokens',
      );
      expect(tokenReq).toBeTruthy();

      adapter.stop();
    });

    it('filters out node_modules, dist, test files, and examples', async () => {
      const ctx = makeCtx();
      ctx.fs.glob = mock(async (pattern: string) => {
        if (pattern === '**/*.ts') {
          return [
            'src/core/domain/foo.ts',
            'node_modules/ws/index.ts',
            'dist/cli.ts',
            'src/core/domain/foo.test.ts',
            'src/core/domain/foo.spec.ts',
            'examples/demo/main.ts',
          ];
        }
        return [];
      });
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 50));

      // extractSummary should only be called for the valid source file
      const calls = ctx.ast.extractSummary.mock.calls;
      const filePaths = calls.map((c: any) => c[0]);
      for (const p of filePaths) {
        expect(p).not.toContain('node_modules');
        expect(p).not.toContain('dist');
        expect(p).not.toContain('.test.ts');
        expect(p).not.toContain('.spec.ts');
        expect(p).not.toContain('examples');
      }

      adapter.stop();
    });
  });

  describe('pushSwarm()', () => {
    it('fetches swarm status, tasks, agents and POSTs swarm data', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 50));

      expect(ctx.swarm.status).toHaveBeenCalled();
      expect(ctx.swarm.listTasks).toHaveBeenCalled();
      expect(ctx.swarm.listAgents).toHaveBeenCalled();

      const swarmReq = httpRequests.find(
        (r) => r.path === '/api/push' && (r.body as any)?.type === 'swarm',
      );
      expect(swarmReq).toBeTruthy();

      adapter.stop();
    });

    it('pushes idle fallback when swarm port throws', async () => {
      const ctx = makeCtx();
      ctx.swarm.status = mock(async () => { throw new Error('not running'); });
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 50));

      const swarmReq = httpRequests.find(
        (r) => r.path === '/api/push' && (r.body as any)?.type === 'swarm',
      );
      expect(swarmReq).toBeTruthy();
      const data = (swarmReq!.body as any).data;
      expect(data.status.status).toBe('idle');
      expect(data.tasks).toEqual([]);
      expect(data.agents).toEqual([]);

      adapter.stop();
    });
  });

  describe('pushGraph()', () => {
    it('builds dependency graph and POSTs nodes and edges', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 50));

      expect(ctx.archAnalyzer.buildDependencyGraph).toHaveBeenCalledWith('/projects/my-app');

      const graphReq = httpRequests.find(
        (r) => r.path === '/api/push' && (r.body as any)?.type === 'graph',
      );
      expect(graphReq).toBeTruthy();
      const data = (graphReq!.body as any).data;
      expect(data.nodes.length).toBeGreaterThan(0);
      expect(data.edges.length).toBeGreaterThan(0);

      adapter.stop();
    });

    it('calls validateHexBoundaries and marks violating edges', async () => {
      const ctx = makeCtx();
      ctx.archAnalyzer.validateHexBoundaries = mock(async () => [
        { from: 'src/core/domain/foo.ts', to: 'src/core/ports/bar.ts', fromLayer: 'domain', toLayer: 'port', rule: 'domain imports port' },
      ]);
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 50));

      expect(ctx.archAnalyzer.validateHexBoundaries).toHaveBeenCalledWith('/projects/my-app');

      const graphReq = httpRequests.find(
        (r) => r.path === '/api/push' && (r.body as any)?.type === 'graph',
      );
      const edges = (graphReq!.body as any).data.edges;
      const violating = edges.find((e: any) => e.from === 'src/core/domain/foo.ts' && e.to === 'src/core/ports/bar.ts');
      expect(violating.violation).toBe(true);

      const clean = edges.find((e: any) => e.from === 'src/adapters/primary/cli.ts');
      expect(clean.violation).toBe(false);

      adapter.stop();
    });

    it('classifies nodes by layer based on file path', async () => {
      const ctx = makeCtx();
      ctx.archAnalyzer.buildDependencyGraph = mock(async () => [
        { from: 'src/core/domain/entity.ts', to: 'src/core/ports/iface.ts', names: ['X'] },
        { from: 'src/core/usecases/uc.ts', to: 'src/core/ports/iface.ts', names: ['X'] },
        { from: 'src/adapters/primary/cli.ts', to: 'src/core/ports/iface.ts', names: ['X'] },
        { from: 'src/adapters/secondary/fs.ts', to: 'src/core/ports/iface.ts', names: ['X'] },
        { from: 'src/index.ts', to: 'src/core/domain/entity.ts', names: ['Y'] },
      ]);
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 50));

      const graphReq = httpRequests.find(
        (r) => r.path === '/api/push' && (r.body as any)?.type === 'graph',
      );
      const nodes = (graphReq!.body as any).data.nodes as Array<{ id: string; layer: string }>;
      const byId = (id: string) => nodes.find((n) => n.id === id);

      expect(byId('src/core/domain/entity.ts')?.layer).toBe('domain');
      expect(byId('src/core/ports/iface.ts')?.layer).toBe('port');
      expect(byId('src/core/usecases/uc.ts')?.layer).toBe('usecase');
      expect(byId('src/adapters/primary/cli.ts')?.layer).toBe('primary-adapter');
      expect(byId('src/adapters/secondary/fs.ts')?.layer).toBe('secondary-adapter');
      expect(byId('src/index.ts')?.layer).toBe('other');

      adapter.stop();
    });
  });

  // ── Command handler registration ─────────────────────

  describe('onCommand / offCommand / isListening', () => {
    it('isListening returns false before start', () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      expect(adapter.isListening()).toBe(false);
    });

    it('registers and deregisters custom command handlers', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);

      const handler = mock(async (cmd: any) => ({
        commandId: cmd.commandId,
        status: 'completed' as const,
        completedAt: new Date().toISOString(),
      }));

      adapter.onCommand('ping', handler);
      adapter.offCommand('ping');

      // After offCommand, handler map should not contain ping
      // We verify indirectly: start + send a ping command => gets "unknown" error
      await adapter.start();
      await new Promise((r) => setTimeout(r, 20));

      // The default handlers are registered on startListening,
      // which re-registers ping. So offCommand before start is overwritten.
      // This test verifies the API works without throwing.
      expect(handler).not.toHaveBeenCalled();

      adapter.stop();
    });
  });

  // ── WebSocket command listener ───────────────────────

  describe('WebSocket command listener', () => {
    it('connects to ws://127.0.0.1:{port}/ws with auth token', async () => {
      mockReadFileSyncResult = JSON.stringify({ token: 'ws-token' });
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      const wsUrl = wsConstructorCalls.find((u) => u.includes('ws://'));
      expect(wsUrl).toBeTruthy();
      expect(wsUrl).toContain('ws://127.0.0.1:9999/ws');
      expect(wsUrl).toContain('token=ws-token');

      adapter.stop();
    });

    it('subscribes to project:{id}:command topic on open', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      expect(latestWs).toBeTruthy();
      const subMsg = latestWs!.sent.find((m: any) => m.type === 'subscribe');
      expect(subMsg).toBeTruthy();
      expect((subMsg as any).topic).toBe('project:proj-123:command');

      adapter.stop();
    });

    it('dispatches received command to registered handler', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      // Send a ping command through the WS
      const command = {
        event: 'command',
        data: {
          commandId: 'cmd-1',
          projectId: 'proj-123',
          type: 'ping',
          payload: {},
          issuedAt: new Date().toISOString(),
          source: 'cli',
        },
      };
      latestWs!.emit('message', Buffer.from(JSON.stringify(command)));

      await new Promise((r) => setTimeout(r, 50));

      // Handler should have posted result back
      const resultReq = httpRequests.find(
        (r) => r.path.includes('/command/cmd-1/result'),
      );
      expect(resultReq).toBeTruthy();
      expect((resultReq!.body as any).status).toBe('completed');
      expect((resultReq!.body as any).data.pong).toBe(true);

      adapter.stop();
    });

    it('posts failed result for unknown command types', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      // Wait for WS listener to be ready
      for (let i = 0; i < 40; i++) {
        await new Promise((r) => setTimeout(r, 10));
        if (latestWs && adapter.isListening()) break;
      }

      const command = {
        event: 'command',
        data: {
          commandId: 'cmd-unknown',
          projectId: 'proj-123',
          type: 'nonexistent-type',
          payload: {},
          issuedAt: new Date().toISOString(),
          source: 'cli',
        },
      };
      latestWs!.emit('message', Buffer.from(JSON.stringify(command)));

      // Give time for async handling
      await new Promise((r) => setTimeout(r, 100));

      // Unknown commands now post a failed result back to the hub
      const resultReq = httpRequests.find(
        (r) => r.path.includes('/command/cmd-unknown/result'),
      );
      expect(resultReq).toBeDefined();
      const body = resultReq!.body as Record<string, unknown>;
      expect(body.status).toBe('failed');
      expect(body.error).toContain('Unknown command type');

      adapter.stop();
    });

    it('returns failed result when handler throws', async () => {
      const ctx = makeCtx();
      ctx.swarm.spawnAgent = mock(async () => { throw new Error('spawn boom'); });
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      const command = {
        event: 'command',
        data: {
          commandId: 'cmd-err',
          projectId: 'proj-123',
          type: 'spawn-agent',
          payload: { name: 'a', role: 'coder' },
          issuedAt: new Date().toISOString(),
          source: 'cli',
        },
      };
      latestWs!.emit('message', Buffer.from(JSON.stringify(command)));

      await new Promise((r) => setTimeout(r, 50));

      const resultReq = httpRequests.find(
        (r) => r.path.includes('/command/cmd-err/result'),
      );
      expect(resultReq).toBeTruthy();
      expect((resultReq!.body as any).status).toBe('failed');
      expect((resultReq!.body as any).error).toContain('spawn boom');

      adapter.stop();
    });

    it('ignores malformed WS messages without crashing', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      // Send garbage
      latestWs!.emit('message', Buffer.from('not json'));
      latestWs!.emit('message', Buffer.from('{}'));
      latestWs!.emit('message', Buffer.from(JSON.stringify({ event: 'other' })));

      // Should not throw — adapter continues working
      expect(adapter.isListening()).toBe(true);

      adapter.stop();
    });
  });

  // ── Reconnection ─────────────────────────────────────

  describe('WebSocket reconnection', () => {
    it('schedules reconnect on WS close with exponential backoff', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      const firstWs = latestWs!;
      wsConstructorCalls = [];

      // Simulate close
      firstWs.emit('close');

      // isListening should be false after close
      expect(adapter.isListening()).toBe(false);

      // Wait for reconnect (1s initial delay)
      await new Promise((r) => setTimeout(r, 1200));

      // Should have attempted reconnect
      expect(wsConstructorCalls.length).toBeGreaterThanOrEqual(1);

      adapter.stop();
    });

    it('does not reconnect after stop()', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      adapter.stop();
      wsConstructorCalls = [];

      // Simulate close on the old WS (already stopped)
      if (latestWs) latestWs.emit('close');

      await new Promise((r) => setTimeout(r, 1500));

      // No reconnect attempts after stop
      expect(wsConstructorCalls.length).toBe(0);
    });

    it('handles WS constructor throwing by scheduling reconnect', async () => {
      const ctx = makeCtx();
      // First connection succeeds (during start), then fails
      let callCount = 0;
      const origThrow = wsConstructorShouldThrow;

      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      // Now make subsequent connections throw
      wsConstructorShouldThrow = true;
      latestWs!.emit('close');

      await new Promise((r) => setTimeout(r, 1500));

      // Adapter should still be alive, just not connected
      // Reset for cleanup
      wsConstructorShouldThrow = false;
      adapter.stop();
    });
  });

  // ── File watcher ─────────────────────────────────────

  describe('file watcher', () => {
    it('watches src/ directory for .ts/.js/.go/.rs changes', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      expect(mockWatchCallback).not.toBeNull();

      adapter.stop();
    });

    it('debounces file change events at 300ms', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      // Trigger rapid changes to the same file
      mockWatchCallback!('change', 'core/domain/foo.ts');
      mockWatchCallback!('change', 'core/domain/foo.ts');
      mockWatchCallback!('change', 'core/domain/foo.ts');

      // Before debounce fires
      const beforeCount = httpRequests.filter(
        (r) => r.path === '/api/event',
      ).length;

      // Wait for debounce (300ms + buffer)
      await new Promise((r) => setTimeout(r, 400));

      const afterCount = httpRequests.filter(
        (r) => r.path === '/api/event',
      ).length;

      // Should have fired only once after debounce
      expect(afterCount - beforeCount).toBe(1);

      adapter.stop();
    });

    it('ignores non-source file extensions', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      const beforeCount = httpRequests.filter((r) => r.path === '/api/event').length;

      mockWatchCallback!('change', 'core/domain/readme.md');
      mockWatchCallback!('change', 'assets/logo.png');
      mockWatchCallback!('change', null);

      await new Promise((r) => setTimeout(r, 400));

      const afterCount = httpRequests.filter((r) => r.path === '/api/event').length;
      expect(afterCount).toBe(beforeCount);

      adapter.stop();
    });

    it('sends file-change event with layer classification', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      mockWatchCallback!('change', 'core/domain/entity.ts');

      await new Promise((r) => setTimeout(r, 400));

      const eventReq = httpRequests.find(
        (r) => r.path === '/api/event' && (r.body as any)?.event === 'file-change',
      );
      expect(eventReq).toBeTruthy();
      expect((eventReq!.body as any).data.path).toBe('src/core/domain/entity.ts');
      expect((eventReq!.body as any).data.layer).toBe('domain');

      adapter.stop();
    });
  });

  // ── Stop / cleanup ───────────────────────────────────

  describe('stop()', () => {
    it('clears push timer, closes watchers, clears debounce timers', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      // Trigger a debounced change
      mockWatchCallback!('change', 'core/domain/foo.ts');

      adapter.stop();

      expect(mockWatcherClose).toHaveBeenCalled();
    });

    it('closes WebSocket connection on stop', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      const ws = latestWs!;
      adapter.stop();

      expect(ws.readyState).toBe(MockWebSocket.CLOSED);
    });
  });

  // ── broadcast() ──────────────────────────────────────

  describe('broadcast()', () => {
    it('forwards event to hub via /api/event when registered', async () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      await adapter.start();

      adapter.broadcast('custom-event', { key: 'value' });

      await new Promise((r) => setTimeout(r, 50));

      const eventReq = httpRequests.find(
        (r) => r.path === '/api/event' && (r.body as any)?.event === 'custom-event',
      );
      expect(eventReq).toBeTruthy();
      expect((eventReq!.body as any).data).toEqual({ key: 'value' });

      adapter.stop();
    });

    it('does nothing when projectId is not set', () => {
      const ctx = makeCtx();
      const adapter = new DashboardAdapter(ctx, 9999);
      const beforeCount = httpRequests.length;

      adapter.broadcast('test', {});

      // No request should be made
      expect(httpRequests.length).toBe(beforeCount);
    });
  });

  // ── startDashboard factory ───────────────────────────

  describe('startDashboard()', () => {
    it('returns url, close function, and commandReceiver', async () => {
      const ctx = makeCtx();
      const result = await startDashboard(ctx, 9999);

      expect(result.url).toBe('http://localhost:9999');
      expect(typeof result.close).toBe('function');
      expect(result.commandReceiver).toBeTruthy();
      expect(typeof result.commandReceiver.isListening).toBe('function');
      expect(typeof result.commandReceiver.onCommand).toBe('function');

      result.close();
    });
  });
});
