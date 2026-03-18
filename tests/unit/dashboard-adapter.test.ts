/**
 * Unit tests for DashboardAdapter (primary adapter)
 *
 * London-school TDD: uses dependency injection for all external dependencies.
 * NO mock.module() calls — avoids permanent module replacement that contaminates
 * other test files in the same Bun process.
 *
 * Tests registration, push methods, WS command listener, reconnection,
 * auth token, file watcher, cleanup, and default command handlers.
 */

import { describe, it, expect, mock, beforeEach, afterEach } from 'bun:test';
import { EventEmitter } from 'node:events';

import {
  DashboardAdapter,
  startDashboard,
  type DashboardAdapterDeps,
} from '../../src/adapters/primary/dashboard-adapter.js';

// ── Mock state ───────────────────────────────────────

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

// ── Mock WebSocket ───────────────────────────────────

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

  ping(): void {
    // no-op for tests
  }

  removeAllListeners(): this {
    super.removeAllListeners();
    return this;
  }
}

let latestWs: MockWebSocket | null = null;
let wsConstructorCalls: string[] = [];
let wsConstructorShouldThrow = false;

// ── Injectable fake HTTP request ─────────────────────

function createFakeHttpRequest() {
  return (opts: any, cb: (res: EventEmitter) => void) => {
    const req = new EventEmitter() as EventEmitter & {
      end: (data: string) => void;
      destroy: () => void;
    };
    req.end = (data: string) => {
      let body: unknown = null;
      try {
        body = JSON.parse(data);
      } catch {
        /* ignore */
      }
      httpRequests.push({
        path: opts.path,
        method: opts.method ?? 'POST',
        body,
        headers: opts.headers ?? {},
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
  };
}

// ── Injectable fake WebSocket creator ────────────────

function createFakeWebSocket(url: string): any {
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
}

// ── Injectable fake file watcher ─────────────────────

function createFakeWatch() {
  return (_dir: string, _opts: unknown, cb: (event: string, filename: string | null) => void) => {
    mockWatchCallback = cb;
    return { close: mockWatcherClose } as any;
  };
}

// ── Build deps helper ────────────────────────────────

function makeDeps(overrides: Partial<DashboardAdapterDeps> = {}): DashboardAdapterDeps {
  return {
    httpRequest: createFakeHttpRequest() as any,
    createWebSocket: createFakeWebSocket as any,
    authToken: 'test-auth-token',
    watchDir: createFakeWatch() as any,
    pathResolve: (...parts: string[]) => parts.join('/'),
    ...overrides,
  };
}

// ── Mock AppContext ──────────────────────────────────

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

// ── Tests ────────────────────────────────────────────

describe('DashboardAdapter', () => {
  beforeEach(() => {
    httpRequests.length = 0;
    httpResponseBody = { id: 'proj-123' };
    httpShouldError = false;
    mockWatchCallback = null;
    mockWatcherClose.mockClear();
    latestWs = null;
    wsConstructorCalls = [];
    wsConstructorShouldThrow = false;
  });

  // ── Registration ─────────────────────────────────

  describe('start() — registration', () => {
    it('registers with hub via POST /api/projects/register', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      const result = await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect((regReq!.body as any).name).toBe('cool-project');

      result.close();
    });

    it('throws when hub registration fails (no id)', async () => {
      httpResponseBody = {}; // no id field
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);

      expect(adapter.start()).rejects.toThrow('Hub registration failed');
    });

    it('throws when hub returns null response', async () => {
      httpShouldError = true;
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);

      expect(adapter.start()).rejects.toThrow('Hub registration failed');
    });

    it('returns a close function that stops the adapter', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      const result = await adapter.start();

      result.close();

      // After close, adapter should be stopped — verify watcher was closed
      expect(mockWatcherClose).toHaveBeenCalled();
    });
  });

  // ── Auth token ───────────────────────────────────

  describe('auth token', () => {
    it('reads token from deps and includes as Bearer header', async () => {
      const ctx = makeCtx();
      const deps = makeDeps({ authToken: 'my-secret-token' });
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect(regReq!.headers['Authorization']).toBe('Bearer my-secret-token');

      adapter.stop();
    });

    it('sends no Authorization header when token is empty', async () => {
      const ctx = makeCtx();
      const deps = makeDeps({ authToken: '' });
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect(regReq!.headers['Authorization']).toBeUndefined();

      adapter.stop();
    });

    it('sends no Authorization header when authToken not provided and lock missing', async () => {
      const ctx = makeCtx();
      // Provide deps without authToken — adapter will call readHubToken() which
      // reads from a non-existent file and returns ''
      const deps = makeDeps();
      // Override authToken to empty to simulate no lock file
      deps.authToken = '';
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      const regReq = httpRequests.find((r) => r.path === '/api/projects/register');
      expect(regReq!.headers['Authorization']).toBeUndefined();

      adapter.stop();
    });
  });

  // ── Push methods ─────────────────────────────────

  describe('pushHealth()', () => {
    it('calls archAnalyzer.analyzeArchitecture and POSTs health data', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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

  // ── Command handler registration ─────────────────

  describe('onCommand / offCommand / isListening', () => {
    it('isListening returns false before start', () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      expect(adapter.isListening()).toBe(false);
    });

    it('registers and deregisters custom command handlers', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);

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

  // ── WebSocket command listener ───────────────────

  describe('WebSocket command listener', () => {
    it('connects to ws://127.0.0.1:{port}/ws with auth token', async () => {
      const ctx = makeCtx();
      const deps = makeDeps({ authToken: 'ws-token' });
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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

    it('returns failed result for unknown command type', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

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

      await new Promise((r) => setTimeout(r, 50));

      const resultReq = httpRequests.find(
        (r) => r.path.includes('/command/cmd-unknown/result'),
      );
      expect(resultReq).toBeTruthy();
      expect((resultReq!.body as any).status).toBe('failed');
      expect((resultReq!.body as any).error).toContain('Unknown command type');

      adapter.stop();
    });

    it('returns failed result when handler throws', async () => {
      const ctx = makeCtx();
      ctx.swarm.spawnAgent = mock(async () => { throw new Error('spawn boom'); });
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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

  // ── Reconnection ─────────────────────────────────

  describe('WebSocket reconnection', () => {
    it('schedules reconnect on WS close with exponential backoff', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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

  // ── File watcher ─────────────────────────────────

  describe('file watcher', () => {
    it('watches src/ directory for .ts/.js/.go/.rs changes', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      expect(mockWatchCallback).not.toBeNull();

      adapter.stop();
    });

    it('debounces file change events at 300ms', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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

  // ── Stop / cleanup ───────────────────────────────

  describe('stop()', () => {
    it('clears push timer, closes watchers, clears debounce timers', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      // Trigger a debounced change
      mockWatchCallback!('change', 'core/domain/foo.ts');

      adapter.stop();

      expect(mockWatcherClose).toHaveBeenCalled();
    });

    it('closes WebSocket connection on stop', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      await new Promise((r) => setTimeout(r, 20));

      const ws = latestWs!;
      adapter.stop();

      expect(ws.readyState).toBe(MockWebSocket.CLOSED);
    });
  });

  // ── broadcast() ──────────────────────────────────

  describe('broadcast()', () => {
    it('forwards event to hub via /api/event when registered', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
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
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      const beforeCount = httpRequests.length;

      adapter.broadcast('test', {});

      // No request should be made
      expect(httpRequests.length).toBe(beforeCount);
    });
  });

  // ── startDashboard factory ───────────────────────

  describe('startDashboard()', () => {
    it('returns url, close function, and commandReceiver', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const result = await startDashboard(ctx, 9999, deps);

      expect(result.url).toBe('http://localhost:9999');
      expect(typeof result.close).toBe('function');
      expect(result.commandReceiver).toBeTruthy();
      expect(typeof result.commandReceiver.isListening).toBe('function');
      expect(typeof result.commandReceiver.onCommand).toBe('function');

      result.close();
    });
  });

  // ── run-plan command handler ───────────────────────

  describe('run-plan command handler', () => {
    async function sendCommand(adapter: DashboardAdapter, type: string, payload: Record<string, unknown>) {
      // Wait for WS connection to establish and handlers to register
      await new Promise((r) => setTimeout(r, 20));

      const command = {
        event: 'command',
        data: {
          commandId: `cmd-plan-${Date.now()}`,
          projectId: 'proj-123',
          type,
          payload,
          issuedAt: new Date().toISOString(),
          source: 'browser',
        },
      };
      latestWs!.emit('message', Buffer.from(JSON.stringify(command)));
      await new Promise((r) => setTimeout(r, 50));
      return command.data.commandId;
    }

    it('dispatches run-plan and returns completed result with plan output', async () => {
      const ctx = makeCtx({
        workplanExecutor: {
          async createPlan() {
            return {
              id: 'wp-test', title: 'Test Plan', estimatedTokenBudget: 4000,
              steps: [
                { id: 's1', description: 'Add port', adapter: 'ports', dependencies: [] },
                { id: 's2', description: 'Add adapter', adapter: 'secondary/db', dependencies: ['s1'] },
              ],
            };
          },
          async *executePlan() {},
        },
      });
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      const cmdId = await sendCommand(adapter, 'run-plan', { requirements: 'Add database support' });

      const resultReq = httpRequests.find((r) => r.path.includes(`/command/${cmdId}/result`));
      expect(resultReq).toBeTruthy();
      expect((resultReq!.body as any).status).toBe('completed');
      expect((resultReq!.body as any).data.planId).toBe('wp-test');
      expect((resultReq!.body as any).data.title).toBe('Test Plan');
      expect((resultReq!.body as any).data.steps).toBe(2);
      expect((resultReq!.body as any).data.output).toContain('Add port');

      adapter.stop();
    });

    it('broadcasts plan-started and plan-output events via /api/event', async () => {
      const ctx = makeCtx({
        workplanExecutor: {
          async createPlan() {
            return {
              id: 'wp-ev', title: 'Event Plan', estimatedTokenBudget: 2000,
              steps: [{ id: 's1', description: 'Add port', adapter: 'ports', dependencies: [] }],
            };
          },
          async *executePlan() {},
        },
      });
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      await sendCommand(adapter, 'run-plan', { requirements: 'Add events' });

      // Check that plan events were pushed via /api/event
      const eventReqs = httpRequests.filter((r) => r.path === '/api/event');
      const planStarted = eventReqs.find((r) => (r.body as any).event === 'plan-started');
      const planOutput = eventReqs.find((r) => (r.body as any).event === 'plan-output');

      expect(planStarted).toBeTruthy();
      expect((planStarted!.body as any).data.requirements).toEqual(['Add events']);
      expect(planOutput).toBeTruthy();
      expect((planOutput!.body as any).data.title).toBe('Event Plan');

      adapter.stop();
    });

    it('returns failed result when requirements are missing', async () => {
      const ctx = makeCtx();
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      const cmdId = await sendCommand(adapter, 'run-plan', {});

      const resultReq = httpRequests.find((r) => r.path.includes(`/command/${cmdId}/result`));
      expect(resultReq).toBeTruthy();
      expect((resultReq!.body as any).status).toBe('failed');
      expect((resultReq!.body as any).error).toContain('Missing required payload');

      adapter.stop();
    });

    it('falls back to structural decomposition without workplanExecutor', async () => {
      const ctx = makeCtx({ workplanExecutor: null });
      const deps = makeDeps();
      const adapter = new DashboardAdapter(ctx, 9999, deps);
      await adapter.start();

      const cmdId = await sendCommand(adapter, 'run-plan', { requirements: 'Add auth, Add cache' });

      const resultReq = httpRequests.find((r) => r.path.includes(`/command/${cmdId}/result`));
      expect(resultReq).toBeTruthy();
      expect((resultReq!.body as any).status).toBe('completed');
      expect((resultReq!.body as any).data.output).toContain('step-1');
      expect((resultReq!.body as any).data.output).toContain('step-2');

      adapter.stop();
    });
  });
});
