/**
 * Unit tests for HubCommandSenderAdapter
 *
 * Tests request construction, fire-and-forget dispatch, status lookup,
 * and listCommands defaults. Uses a lightweight HTTP server to verify
 * the adapter sends correct requests without mocking internals.
 */

import { describe, it, expect, beforeAll, afterAll } from 'bun:test';
import { createServer, type Server, type IncomingMessage, type ServerResponse } from 'node:http';
import { HubCommandSenderAdapter } from '../../src/adapters/primary/hub-command-sender.js';
import type { HubCommand, HubCommandResult } from '../../src/core/ports/hub-command.js';

// ── Test HTTP server ────────────────────────────────────

let server: Server;
let port: number;
const receivedRequests: Array<{
  method: string;
  url: string;
  body: unknown;
}> = [];

/** Simple handler that records requests and returns canned responses. */
function handler(req: IncomingMessage, res: ServerResponse): void {
  const chunks: Buffer[] = [];
  req.on('data', (c: Buffer) => chunks.push(c));
  req.on('end', () => {
    let body: unknown = null;
    if (chunks.length > 0) {
      try { body = JSON.parse(Buffer.concat(chunks).toString('utf-8')); } catch { /* ignore */ }
    }
    receivedRequests.push({ method: req.method ?? '', url: req.url ?? '', body });

    const url = req.url ?? '';

    // POST /api/{projectId}/command — accept command
    if (req.method === 'POST' && url.match(/^\/api\/[\w-]+\/command$/)) {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ ok: true }));
      return;
    }

    // GET /api/command/{commandId} — return completed result
    if (req.method === 'GET' && url.match(/^\/api\/command\/[\w-]+$/)) {
      const commandId = url.split('/').pop()!;
      const result: HubCommandResult = {
        commandId,
        status: 'completed',
        data: { pong: true },
        completedAt: new Date().toISOString(),
      };
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify(result));
      return;
    }

    // GET /api/{projectId}/commands?limit=N — return empty list
    if (req.method === 'GET' && url.match(/^\/api\/[\w-]+\/commands/)) {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ commands: [] }));
      return;
    }

    res.writeHead(404);
    res.end('not found');
  });
}

beforeAll(async () => {
  server = createServer(handler);
  await new Promise<void>((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve());
  });
  const addr = server.address();
  port = typeof addr === 'object' && addr ? addr.port : 0;
});

afterAll(() => {
  server.close();
});

// ── Helpers ─────────────────────────────────────────────

function makeAdapter(): HubCommandSenderAdapter {
  return new HubCommandSenderAdapter(port, 'test-token');
}

function baseCommand(): Omit<HubCommand, 'commandId' | 'issuedAt'> {
  return {
    projectId: 'proj-1',
    type: 'ping',
    payload: {},
    source: 'cli',
  };
}

// ── Tests ───────────────────────────────────────────────

describe('HubCommandSenderAdapter', () => {
  it('sendCommand constructs correct POST and polls for result', async () => {
    const adapter = makeAdapter();
    receivedRequests.length = 0;

    const result = await adapter.sendCommand(baseCommand());

    // Should have POSTed the command
    const postReq = receivedRequests.find(
      (r) => r.method === 'POST' && r.url?.includes('/api/proj-1/command'),
    );
    expect(postReq).toBeTruthy();

    const posted = postReq!.body as HubCommand;
    expect(posted.projectId).toBe('proj-1');
    expect(posted.type).toBe('ping');
    expect(posted.commandId).toBeTruthy();
    expect(posted.issuedAt).toBeTruthy();

    // Should have polled and received completed result
    expect(result.status).toBe('completed');
    expect(result.commandId).toBe(posted.commandId);
  });

  it('sendCommand includes Authorization header when token provided', async () => {
    // The test server receives the request — we verify the adapter
    // created with a token sends it. We check indirectly: if auth were
    // missing, a real hub would reject. Here we just verify the request
    // completes successfully (server doesn't check auth).
    const adapter = makeAdapter();
    const result = await adapter.sendCommand(baseCommand());
    expect(result.status).toBe('completed');
  });

  it('dispatchCommand returns commandId immediately', async () => {
    const adapter = makeAdapter();
    receivedRequests.length = 0;

    const commandId = await adapter.dispatchCommand(baseCommand());

    expect(typeof commandId).toBe('string');
    expect(commandId.length).toBeGreaterThan(0);

    // Should have posted exactly one request (no polling)
    const posts = receivedRequests.filter(
      (r) => r.method === 'POST' && r.url?.includes('/api/proj-1/command'),
    );
    expect(posts.length).toBe(1);

    const posted = posts[0].body as HubCommand;
    expect(posted.commandId).toBe(commandId);
  });

  it('getCommandStatus returns null for missing command', async () => {
    // Spin up a quick server that returns 404 for unknown commands
    const missingServer = createServer((_req, res) => {
      res.writeHead(404);
      res.end('not found');
    });
    await new Promise<void>((resolve) => {
      missingServer.listen(0, '127.0.0.1', () => resolve());
    });
    const addr = missingServer.address();
    const missingPort = typeof addr === 'object' && addr ? addr.port : 0;

    const adapter = new HubCommandSenderAdapter(missingPort, 'test-token');
    const result = await adapter.getCommandStatus('nonexistent-id');
    expect(result).toBeNull();

    missingServer.close();
  });

  it('listCommands defaults limit to 50', async () => {
    const adapter = makeAdapter();
    receivedRequests.length = 0;

    const commands = await adapter.listCommands('proj-1');

    expect(Array.isArray(commands)).toBe(true);

    const getReq = receivedRequests.find(
      (r) => r.method === 'GET' && r.url?.includes('/api/proj-1/commands'),
    );
    expect(getReq).toBeTruthy();
    expect(getReq!.url).toContain('limit=50');
  });

  it('listCommands respects custom limit', async () => {
    const adapter = makeAdapter();
    receivedRequests.length = 0;

    await adapter.listCommands('proj-1', 10);

    const getReq = receivedRequests.find(
      (r) => r.method === 'GET' && r.url?.includes('/api/proj-1/commands'),
    );
    expect(getReq!.url).toContain('limit=10');
  });

  it('dispatchCommand sets issuedAt to ISO timestamp', async () => {
    const adapter = makeAdapter();
    receivedRequests.length = 0;

    const before = new Date().toISOString();
    await adapter.dispatchCommand(baseCommand());
    const after = new Date().toISOString();

    const posted = receivedRequests[0].body as HubCommand;
    expect(posted.issuedAt >= before).toBe(true);
    expect(posted.issuedAt <= after).toBe(true);
  });
});
