/**
 * Hub Command Sender Adapter
 *
 * Sends commands to projects via the hex-hub HTTP API.
 * Used by CLI, MCP, and browser UI to issue commands like
 * spawn-agent, run-analyze, etc.
 *
 * This is a PRIMARY (driving) adapter implementing IHubCommandSenderPort.
 */

import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { request as httpRequest } from 'node:http';
import type {
  HubCommand,
  HubCommandResult,
  IHubCommandSenderPort,
} from '../../core/ports/hub-command.js';

/** Default hub port — must match dashboard-hub.ts */
const DEFAULT_HUB_PORT = 5555;

/** Max time to wait for a command result (ms) */
const SEND_TIMEOUT_MS = 30_000;

/** Interval between status polls (ms) */
const POLL_INTERVAL_MS = 500;

/** Default command list limit */
const DEFAULT_LIST_LIMIT = 50;

/** Read auth token from hub lock file. Returns empty string if unavailable. */
function readHubToken(): string {
  try {
    const lockPath = join(homedir(), '.hex', 'daemon', 'hub.lock');
    const lock = JSON.parse(readFileSync(lockPath, 'utf-8'));
    return lock.token ?? '';
  } catch {
    return '';
  }
}

// ── Hub Command Sender ──────────────────────────────────

export class HubCommandSenderAdapter implements IHubCommandSenderPort {
  private readonly authToken: string;

  constructor(
    private readonly hubPort: number = DEFAULT_HUB_PORT,
    authToken?: string,
  ) {
    this.authToken = authToken ?? readHubToken();
  }

  async sendCommand(
    command: Omit<HubCommand, 'commandId' | 'issuedAt'>,
  ): Promise<HubCommandResult> {
    const commandId = randomUUID();
    const full: HubCommand = {
      ...command,
      commandId,
      issuedAt: new Date().toISOString(),
    };

    await this.post(`/api/${command.projectId}/command`, full);

    // Poll until completed/failed or timeout
    const deadline = Date.now() + SEND_TIMEOUT_MS;
    while (Date.now() < deadline) {
      await sleep(POLL_INTERVAL_MS);
      const result = await this.getCommandStatus(commandId);
      if (result && (result.status === 'completed' || result.status === 'failed')) {
        return result;
      }
    }

    return {
      commandId,
      status: 'failed',
      error: `Command timed out after ${SEND_TIMEOUT_MS}ms`,
      completedAt: new Date().toISOString(),
    };
  }

  async dispatchCommand(
    command: Omit<HubCommand, 'commandId' | 'issuedAt'>,
  ): Promise<string> {
    const commandId = randomUUID();
    const full: HubCommand = {
      ...command,
      commandId,
      issuedAt: new Date().toISOString(),
    };

    await this.post(`/api/${command.projectId}/command`, full);
    return commandId;
  }

  async getCommandStatus(commandId: string): Promise<HubCommandResult | null> {
    // The hub stores command results at a well-known path.
    // We need the projectId context, but the port interface only gives commandId.
    // Hub supports a global lookup: GET /api/command/{commandId}
    const result = await this.get(`/api/command/${commandId}`);
    if (!result) return null;
    return result as unknown as HubCommandResult;
  }

  async listCommands(projectId: string, limit?: number): Promise<HubCommand[]> {
    const effectiveLimit = limit ?? DEFAULT_LIST_LIMIT;
    const result = await this.get(
      `/api/${projectId}/commands?limit=${effectiveLimit}`,
    );
    if (!result) return [];
    const commands = (result as Record<string, unknown>).commands;
    return (Array.isArray(commands) ? commands : []) as HubCommand[];
  }

  // ── HTTP helpers ──────────────────────────────────────

  private post(path: string, body: unknown): Promise<Record<string, unknown> | null> {
    return new Promise((resolve) => {
      const payload = JSON.stringify(body);
      const headers: Record<string, string | number> = {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(payload),
      };
      if (this.authToken) {
        headers['Authorization'] = `Bearer ${this.authToken}`;
      }
      const req = httpRequest(
        {
          hostname: '127.0.0.1',
          port: this.hubPort,
          path,
          method: 'POST',
          headers,
          timeout: 5000,
        },
        (res) => {
          const chunks: Buffer[] = [];
          res.on('data', (c: Buffer) => chunks.push(c));
          res.on('end', () => {
            try {
              resolve(JSON.parse(Buffer.concat(chunks).toString('utf-8')));
            } catch {
              resolve(null);
            }
          });
        },
      );
      req.on('error', () => resolve(null));
      req.on('timeout', () => { req.destroy(); resolve(null); });
      req.end(payload);
    });
  }

  private get(path: string): Promise<Record<string, unknown> | null> {
    return new Promise((resolve) => {
      const headers: Record<string, string> = {};
      if (this.authToken) {
        headers['Authorization'] = `Bearer ${this.authToken}`;
      }
      const req = httpRequest(
        {
          hostname: '127.0.0.1',
          port: this.hubPort,
          path,
          method: 'GET',
          headers,
          timeout: 5000,
        },
        (res) => {
          const chunks: Buffer[] = [];
          res.on('data', (c: Buffer) => chunks.push(c));
          res.on('end', () => {
            try {
              const parsed = JSON.parse(Buffer.concat(chunks).toString('utf-8'));
              resolve(parsed as Record<string, unknown>);
            } catch {
              resolve(null);
            }
          });
        },
      );
      req.on('error', () => resolve(null));
      req.on('timeout', () => { req.destroy(); resolve(null); });
      req.end();
    });
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
