/**
 * Unit tests for HubCommandSenderAdapter
 *
 * Tests request construction, fire-and-forget dispatch, status lookup,
 * and listCommands defaults.
 *
 * Uses mock.module('node:http') scoped to this file so that the adapter's
 * HTTP calls go through a controlled fake, avoiding conflicts with other
 * test files that also mock node:http (e.g. dashboard-adapter.test.ts).
 */

import { describe, it, expect, beforeEach, mock } from 'bun:test';
import { EventEmitter } from 'node:events';
import type { HubCommand, HubCommandResult } from '../../src/core/ports/hub-command.js';

// ── Per-test HTTP intercept state ──────────────────────

/** Captured requests from the adapter's node:http usage */
let capturedRequests: Array<{
  method: string;
  path: string;
  body: unknown;
  headers: Record<string, string | number>;
}> = [];

/** Map of path patterns to response factories. Set per test. */
let responseHandlers: Array<{
  match: (method: string, path: string) => boolean;
  respond: (method: string, path: string, body: unknown) => { status: number; body: unknown };
}> = [];

/** Default response when no handler matches */
function defaultResponse() {
  return { status: 404, body: 'not found' };
}

// ── Mock node:http ─────────────────────────────────────

mock.module('node:http', () => ({
  request: (
    opts: { hostname: string; port: number; path: string; method: string; headers: Record<string, string | number>; timeout?: number },
    cb: (res: EventEmitter & { statusCode: number }) => void,
  ) => {
    const req = new EventEmitter() as EventEmitter & {
      end: (data?: string) => void;
      destroy: () => void;
    };

    req.destroy = () => {};

    req.end = (data?: string) => {
      let body: unknown = null;
      if (data) {
        try { body = JSON.parse(data); } catch { /* ignore */ }
      }

      capturedRequests.push({
        method: opts.method,
        path: opts.path,
        body,
        headers: opts.headers ?? {},
      });

      // Find matching handler
      let response = defaultResponse();
      for (const handler of responseHandlers) {
        if (handler.match(opts.method, opts.path)) {
          response = handler.respond(opts.method, opts.path, body);
          break;
        }
      }

      // Simulate async response
      setTimeout(() => {
        const res = new EventEmitter() as EventEmitter & { statusCode: number };
        res.statusCode = response.status;
        cb(res);

        if (response.status >= 200 && response.status < 300 && response.body !== null) {
          const buf = Buffer.from(
            typeof response.body === 'string'
              ? response.body
              : JSON.stringify(response.body),
          );
          res.emit('data', buf);
        } else if (response.status >= 400) {
          const buf = Buffer.from(
            typeof response.body === 'string'
              ? response.body
              : JSON.stringify(response.body),
          );
          res.emit('data', buf);
        }
        res.emit('end');
      }, 0);
    };

    return req;
  },
}));

// Import AFTER mock.module so the adapter picks up the mock
const { HubCommandSenderAdapter } = await import('../../src/adapters/primary/hub-command-sender.js');

// ── Tests ───────────────────────────────────────────────

describe('HubCommandSenderAdapter', () => {
  beforeEach(() => {
    capturedRequests = [];
    responseHandlers = [];
  });

  /** Install the standard set of handlers that simulate a working hub */
  function installStandardHandlers(): void {
    responseHandlers = [
      // POST /api/{projectId}/command — accept command
      {
        match: (method, path) => method === 'POST' && /^\/api\/[\w-]+\/command$/.test(path),
        respond: () => ({ status: 200, body: { ok: true } }),
      },
      // GET /api/command/{commandId} — return completed result
      {
        match: (method, path) => method === 'GET' && /^\/api\/command\/[\w-]+$/.test(path),
        respond: (_m, path) => {
          const commandId = path.split('/').pop()!;
          const result: HubCommandResult = {
            commandId,
            status: 'completed',
            data: { pong: true },
            completedAt: new Date().toISOString(),
          };
          return { status: 200, body: result };
        },
      },
      // GET /api/{projectId}/commands?limit=N — return empty list
      {
        match: (method, path) => method === 'GET' && /^\/api\/[\w-]+\/commands/.test(path),
        respond: () => ({ status: 200, body: { commands: [] } }),
      },
    ];
  }

  // ── Helpers ─────────────────────────────────────────────

  function makeAdapter(): InstanceType<typeof HubCommandSenderAdapter> {
    return new HubCommandSenderAdapter(9999, 'test-token');
  }

  function baseCommand(): Omit<HubCommand, 'commandId' | 'issuedAt'> {
    return {
      projectId: 'proj-1',
      type: 'ping',
      payload: {},
      source: 'cli',
    };
  }

  it('sendCommand constructs correct POST and polls for result', async () => {
    installStandardHandlers();
    const adapter = makeAdapter();

    const result = await adapter.sendCommand(baseCommand());

    // Should have POSTed the command
    const postReq = capturedRequests.find(
      (r) => r.method === 'POST' && r.path?.includes('/api/proj-1/command'),
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
  }, 15_000);

  it('sendCommand includes Authorization header when token provided', async () => {
    installStandardHandlers();
    const adapter = makeAdapter();
    const result = await adapter.sendCommand(baseCommand());
    expect(result.status).toBe('completed');

    // Verify the Authorization header was sent
    const postReq = capturedRequests.find(
      (r) => r.method === 'POST' && r.path?.includes('/api/proj-1/command'),
    );
    expect(postReq).toBeTruthy();
    expect(postReq!.headers['Authorization']).toBe('Bearer test-token');
  }, 15_000);

  it('dispatchCommand returns commandId immediately', async () => {
    installStandardHandlers();
    const adapter = makeAdapter();

    const commandId = await adapter.dispatchCommand(baseCommand());

    expect(typeof commandId).toBe('string');
    expect(commandId.length).toBeGreaterThan(0);

    // Should have posted exactly one request (no polling)
    const posts = capturedRequests.filter(
      (r) => r.method === 'POST' && r.path?.includes('/api/proj-1/command'),
    );
    expect(posts.length).toBe(1);

    const posted = posts[0].body as HubCommand;
    expect(posted.commandId).toBe(commandId);
  }, 10_000);

  it('getCommandStatus returns null for missing command', async () => {
    // Install a handler that returns 404 for all requests
    responseHandlers = [
      {
        match: () => true,
        respond: () => ({ status: 404, body: 'not found' }),
      },
    ];

    const adapter = makeAdapter();
    const result = await adapter.getCommandStatus('nonexistent-id');
    expect(result).toBeNull();
  }, 10_000);

  it('listCommands defaults limit to 50', async () => {
    installStandardHandlers();
    const adapter = makeAdapter();

    const commands = await adapter.listCommands('proj-1');

    expect(Array.isArray(commands)).toBe(true);

    const getReq = capturedRequests.find(
      (r) => r.method === 'GET' && r.path?.includes('/api/proj-1/commands'),
    );
    expect(getReq).toBeTruthy();
    expect(getReq!.path).toContain('limit=50');
  }, 10_000);

  it('listCommands respects custom limit', async () => {
    installStandardHandlers();
    const adapter = makeAdapter();

    await adapter.listCommands('proj-1', 10);

    const getReq = capturedRequests.find(
      (r) => r.method === 'GET' && r.path?.includes('/api/proj-1/commands'),
    );
    expect(getReq!.path).toContain('limit=10');
  }, 10_000);

  it('dispatchCommand sets issuedAt to ISO timestamp', async () => {
    installStandardHandlers();
    const adapter = makeAdapter();

    const before = new Date().toISOString();
    await adapter.dispatchCommand(baseCommand());
    const after = new Date().toISOString();

    const posted = capturedRequests[0].body as HubCommand;
    expect(posted.issuedAt >= before).toBe(true);
    expect(posted.issuedAt <= after).toBe(true);
  }, 10_000);
});
