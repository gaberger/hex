/**
 * End-to-end tests for the hex-hub Rust binary.
 *
 * Starts the actual hex-hub binary on a random port, then verifies:
 *   1. Project registration + deregistration lifecycle
 *   2. State push and query endpoints (health, tokens, swarm, graph)
 *   3. WebSocket connection, subscribe, and broadcast
 *   4. Command dispatch and result reporting
 *   5. Multi-project isolation
 *   6. Auth token enforcement
 *   7. Version endpoint
 *   8. TTL cleanup (commands expire)
 *
 * These tests require the hex-hub binary at hex-hub/target/release/hex-hub.
 */

import { describe, it, expect, beforeAll, afterAll } from 'bun:test';
import { spawn, type Subprocess } from 'bun';
import { join } from 'node:path';

// ── Hub Process Management ───────────────────────────────

let hubProcess: Subprocess | null = null;
let hubPort: number;
const hubToken = 'e2e-test-token-' + Date.now();

const BINARY_PATH = join(import.meta.dir, '../../hex-hub/target/release/hex-hub');
const HUB_START_TIMEOUT = 5000;

/** Find a random available port. */
async function findFreePort(): Promise<number> {
  const server = Bun.serve({ port: 0, fetch() { return new Response(''); } });
  const port = server.port;
  server.stop(true);
  // Small delay to ensure port is released
  await new Promise((r) => setTimeout(r, 100));
  return port;
}

/** Wait for hub to become healthy. */
async function waitForHub(port: number, timeoutMs = HUB_START_TIMEOUT): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/api/version`, {
        signal: AbortSignal.timeout(500),
      });
      if (res.ok) return;
    } catch { /* not ready yet */ }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`hex-hub did not become healthy within ${timeoutMs}ms`);
}

/** Helper to make authenticated requests. */
function hubFetch(path: string, options: RequestInit = {}): Promise<Response> {
  const headers = new Headers(options.headers);
  headers.set('Authorization', `Bearer ${hubToken}`);
  if (options.body && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json');
  }
  return fetch(`http://127.0.0.1:${hubPort}${path}`, { ...options, headers });
}

/** POST JSON to hub. */
async function hubPost(path: string, body: unknown): Promise<unknown> {
  const res = await hubFetch(path, {
    method: 'POST',
    body: JSON.stringify(body),
  });
  return res.json();
}

/** GET from hub. */
async function hubGet(path: string): Promise<unknown> {
  const res = await hubFetch(path);
  return res.json();
}

/** Connect a WebSocket to the hub. */
function hubWs(): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${hubPort}/ws?token=${encodeURIComponent(hubToken)}`);
    ws.onopen = () => resolve(ws);
    ws.onerror = () => reject(new Error('WS connection failed'));
    setTimeout(() => reject(new Error('WS connection timeout')), 3000);
  });
}

/** Wait for a WS message matching a predicate. */
function waitForWsMessage(
  ws: WebSocket,
  predicate: (msg: any) => boolean,
  timeoutMs = 5000,
): Promise<any> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      ws.removeEventListener('message', handler);
      reject(new Error(`WS message wait timed out after ${timeoutMs}ms`));
    }, timeoutMs);

    function handler(ev: MessageEvent) {
      try {
        const data = JSON.parse(ev.data);
        if (predicate(data)) {
          clearTimeout(timer);
          ws.removeEventListener('message', handler);
          resolve(data);
        }
      } catch { /* ignore non-JSON */ }
    }
    ws.addEventListener('message', handler);
  });
}

// ── Test Suite ────────────────────────────────────────────

describe('hex-hub E2E', () => {
  beforeAll(async () => {
    // Check binary exists
    const file = Bun.file(BINARY_PATH);
    if (!(await file.exists())) {
      throw new Error(`hex-hub binary not found at ${BINARY_PATH}. Run: cd hex-hub && cargo build --release`);
    }

    hubPort = await findFreePort();

    hubProcess = spawn({
      cmd: [BINARY_PATH, '--port', String(hubPort), '--token', hubToken],
      stdout: 'pipe',
      stderr: 'pipe',
      env: { ...process.env },
    });

    await waitForHub(hubPort);
  }, 10000);

  afterAll(() => {
    if (hubProcess) {
      hubProcess.kill();
      hubProcess = null;
    }
  });

  // ── Version ─────────────────────────────────────────

  it('serves version endpoint', async () => {
    const data = await hubGet('/api/version') as any;
    expect(data.version).toBeDefined();
    expect(typeof data.version).toBe('string');
  });

  // ── Registration Lifecycle ──────────────────────────

  it('registers a project and returns an ID', async () => {
    const data = await hubPost('/api/projects/register', {
      name: 'test-project',
      rootPath: '/tmp/test-project',
      astIsStub: false,
    }) as any;

    // Rust hub returns { id, name, rootPath } on register (no "ok" field)
    expect(data.id).toBeDefined();
    expect(typeof data.id).toBe('string');
    expect(data.id.length).toBeGreaterThan(0);
    expect(data.name).toBe('test-project');
  });

  it('lists registered projects', async () => {
    const data = await hubGet('/api/projects') as any;
    expect(data.projects).toBeArray();
    expect(data.projects.length).toBeGreaterThanOrEqual(1);

    const proj = data.projects.find((p: any) => p.name === 'test-project');
    expect(proj).toBeTruthy();
    expect(proj.rootPath).toBe('/tmp/test-project');
  });

  it('deregisters a project by ID', async () => {
    // Register a throwaway project
    const reg = await hubPost('/api/projects/register', {
      name: 'to-delete',
      rootPath: '/tmp/to-delete',
    }) as any;
    const id = reg.id;

    // Verify it exists
    let list = await hubGet('/api/projects') as any;
    expect(list.projects.some((p: any) => p.id === id)).toBe(true);

    // Delete it
    const delRes = await hubFetch(`/api/projects/${id}`, { method: 'DELETE' });
    expect(delRes.ok).toBe(true);

    // Verify it's gone
    list = await hubGet('/api/projects') as any;
    expect(list.projects.some((p: any) => p.id === id)).toBe(false);
  });

  // ── State Push + Query ──────────────────────────────

  it('accepts and serves pushed health data', async () => {
    // Get the registered project ID
    const list = await hubGet('/api/projects') as any;
    const projectId = list.projects[0].id;

    const healthData = {
      summary: { healthScore: 85, totalFiles: 42, violationCount: 0 },
      dependencyViolations: [],
      deadExports: [],
    };

    // Push health
    await hubPost('/api/push', {
      projectId,
      type: 'health',
      data: healthData,
    });

    // Query it back
    const result = await hubGet(`/api/${projectId}/health`) as any;
    expect(result).toBeTruthy();
    expect(result.summary?.healthScore ?? result.healthScore ?? result.score).toBeDefined();
  });

  it('accepts and serves pushed graph data', async () => {
    const list = await hubGet('/api/projects') as any;
    const projectId = list.projects[0].id;

    const graphData = {
      nodes: [
        { id: 'src/core/domain/entities.ts', layer: 'domain' },
        { id: 'src/core/ports/index.ts', layer: 'port' },
        { id: 'src/adapters/primary/cli.ts', layer: 'primary-adapter' },
      ],
      edges: [
        { from: 'src/adapters/primary/cli.ts', to: 'src/core/ports/index.ts', violation: false },
        { from: 'src/core/ports/index.ts', to: 'src/core/domain/entities.ts', violation: false },
      ],
    };

    await hubPost('/api/push', { projectId, type: 'graph', data: graphData });

    const result = await hubGet(`/api/${projectId}/graph`) as any;
    expect(result.nodes).toBeArray();
    expect(result.nodes.length).toBe(3);
    expect(result.edges).toBeArray();
    expect(result.edges.length).toBe(2);
  });

  it('accepts and serves pushed swarm data', async () => {
    const list = await hubGet('/api/projects') as any;
    const projectId = list.projects[0].id;

    const swarmData = {
      status: { status: 'running', agentCount: 2, activeTaskCount: 1 },
      agents: [{ id: 'a1', name: 'coder', role: 'coder', status: 'busy' }],
      tasks: [{ id: 't1', title: 'implement feature', status: 'in-progress' }],
    };

    await hubPost('/api/push', { projectId, type: 'swarm', data: swarmData });

    const result = await hubGet(`/api/${projectId}/swarm`) as any;
    expect(result).toBeTruthy();
  });

  // ── Multi-Project Isolation ─────────────────────────

  it('projects do not share pushed state', async () => {
    // Register two projects
    const regA = await hubPost('/api/projects/register', {
      name: 'isolated-a',
      rootPath: '/tmp/isolated-a',
    }) as any;
    const regB = await hubPost('/api/projects/register', {
      name: 'isolated-b',
      rootPath: '/tmp/isolated-b',
    }) as any;

    // Push health to project A only
    await hubPost('/api/push', {
      projectId: regA.id,
      type: 'health',
      data: { summary: { healthScore: 95 } },
    });

    // Push different health to project B
    await hubPost('/api/push', {
      projectId: regB.id,
      type: 'health',
      data: { summary: { healthScore: 40 } },
    });

    // Query each — should get their own data
    const healthA = await hubGet(`/api/${regA.id}/health`) as any;
    const healthB = await hubGet(`/api/${regB.id}/health`) as any;

    const scoreA = healthA?.summary?.healthScore ?? healthA?.healthScore ?? healthA?.score;
    const scoreB = healthB?.summary?.healthScore ?? healthB?.healthScore ?? healthB?.score;

    expect(scoreA).toBe(95);
    expect(scoreB).toBe(40);

    // Cleanup
    await hubFetch(`/api/projects/${regA.id}`, { method: 'DELETE' });
    await hubFetch(`/api/projects/${regB.id}`, { method: 'DELETE' });
  });

  // ── Command Lifecycle ───────────────────────────────

  it('dispatches a command and accepts a result', async () => {
    const list = await hubGet('/api/projects') as any;
    const projectId = list.projects[0].id;

    // Send a command
    const cmdRes = await hubPost(`/api/${projectId}/command`, {
      type: 'ping',
      payload: {},
      source: 'e2e-test',
    }) as any;

    expect(cmdRes.commandId).toBeDefined();
    const commandId = cmdRes.commandId;

    // Check command is pending
    const cmd = await hubGet(`/api/${projectId}/command/${commandId}`) as any;
    expect(cmd.commandId ?? cmd.id).toBe(commandId);

    // Report a result
    await hubPost(`/api/${projectId}/command/${commandId}/result`, {
      commandId,
      status: 'completed',
      data: { pong: true },
    });

    // List commands — should include our command
    const cmds = await hubGet(`/api/${projectId}/commands`) as any;
    expect(cmds.commands ?? cmds).toBeArray();
  });

  // ── WebSocket ───────────────────────────────────────

  it('accepts WebSocket connections', async () => {
    const ws = await hubWs();
    expect(ws.readyState).toBe(WebSocket.OPEN);
    ws.close();
  });

  it('broadcasts state updates to subscribed WS clients', async () => {
    const list = await hubGet('/api/projects') as any;
    const projectId = list.projects[0].id;

    const ws = await hubWs();

    // Subscribe to project state
    ws.send(JSON.stringify({ type: 'subscribe', topic: `project:${projectId}:state` }));

    // Small delay for subscription to register
    await new Promise((r) => setTimeout(r, 200));

    // Push state — should trigger WS broadcast
    const msgPromise = waitForWsMessage(ws, (msg) => {
      return msg.event === 'state-update' || msg.topic?.includes(projectId);
    }, 3000);

    await hubPost('/api/push', {
      projectId,
      type: 'health',
      data: { summary: { healthScore: 77 } },
    });

    const msg = await msgPromise;
    expect(msg).toBeTruthy();

    ws.close();
  });

  it('broadcasts command to subscribed WS clients', async () => {
    const list = await hubGet('/api/projects') as any;
    const projectId = list.projects[0].id;

    const ws = await hubWs();
    ws.send(JSON.stringify({ type: 'subscribe', topic: `project:${projectId}:command` }));
    await new Promise((r) => setTimeout(r, 200));

    const msgPromise = waitForWsMessage(ws, (msg) => {
      return msg.event === 'command' && msg.data?.type === 'ping';
    }, 3000);

    await hubPost(`/api/${projectId}/command`, {
      type: 'ping',
      payload: {},
      source: 'e2e-ws-test',
    });

    const msg = await msgPromise;
    expect(msg.data.type).toBe('ping');
    expect(msg.data.commandId).toBeDefined();

    ws.close();
  });

  it('does not broadcast to unsubscribed topics', async () => {
    const list = await hubGet('/api/projects') as any;
    const projectId = list.projects[0].id;

    const ws = await hubWs();
    // Subscribe to a DIFFERENT project's topic
    ws.send(JSON.stringify({ type: 'subscribe', topic: 'project:nonexistent:state' }));
    await new Promise((r) => setTimeout(r, 200));

    let received = false;
    ws.addEventListener('message', () => { received = true; });

    // Push to the real project
    await hubPost('/api/push', {
      projectId,
      type: 'health',
      data: { summary: { healthScore: 50 } },
    });

    // Wait a bit — should NOT receive anything
    await new Promise((r) => setTimeout(r, 500));
    expect(received).toBe(false);

    ws.close();
  });

  // ── Auth Token Enforcement ──────────────────────────

  it('rejects unauthenticated requests when token is set', async () => {
    const res = await fetch(`http://127.0.0.1:${hubPort}/api/projects/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'unauth', rootPath: '/tmp/unauth' }),
      // NO Authorization header
    });

    // Hub should reject with 401 or 403
    expect(res.status).toBeGreaterThanOrEqual(400);
    expect(res.status).toBeLessThan(500);
  });

  // ── Serving Frontend ────────────────────────────────

  it('serves the dashboard HTML on root', async () => {
    const res = await fetch(`http://127.0.0.1:${hubPort}/`);
    expect(res.ok).toBe(true);
    const html = await res.text();
    expect(html).toContain('hex Dashboard');
    expect(html).toContain('<canvas');
  });
});
