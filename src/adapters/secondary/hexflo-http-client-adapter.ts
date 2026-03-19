/**
 * HexFlo HTTP Client Adapter
 *
 * Secondary adapter implementing IHexFloClientPort via HTTP calls to
 * the hex-nexus daemon at localhost:5555.
 *
 * This is the SINGLE place that knows about HexFlo REST endpoints.
 * Both CLI and MCP primary adapters delegate here via the port.
 */

import type { IHexFloClientPort, HexFloResponse } from '../../core/ports/hexflo-client.js';

export class HexFloHttpClientAdapter implements IHexFloClientPort {
  constructor(private readonly hubUrl: string = 'http://localhost:5555') {}

  // ── Swarm ──────────────────────────────────────────

  async swarmInit(name: string, projectId: string, topology?: string): Promise<HexFloResponse> {
    return this.call('POST', '/api/swarms', {
      project_id: projectId,
      name,
      topology: topology || 'hierarchical',
    });
  }

  async swarmStatus(): Promise<HexFloResponse> {
    return this.call('GET', '/api/swarms');
  }

  // ── Tasks ──────────────────────────────────────────

  async taskCreate(swarmId: string, title: string): Promise<HexFloResponse> {
    return this.call('POST', `/api/swarms/${encodeURIComponent(swarmId)}/tasks`, { title });
  }

  async taskList(swarmId?: string): Promise<HexFloResponse> {
    if (swarmId) {
      return this.call('GET', `/api/swarms/${encodeURIComponent(swarmId)}`);
    }
    return this.call('GET', '/api/swarms');
  }

  async taskComplete(taskId: string, result?: string): Promise<HexFloResponse> {
    return this.call('PATCH', `/api/swarms/tasks/${encodeURIComponent(taskId)}`, {
      status: 'completed',
      result: result || null,
    });
  }

  // ── Memory ─────────────────────────────────────────

  async memoryStore(key: string, value: string, scope?: string): Promise<HexFloResponse> {
    return this.call('POST', '/api/hexflo/memory', { key, value, scope: scope || 'global' });
  }

  async memoryRetrieve(key: string): Promise<HexFloResponse> {
    return this.call('GET', `/api/hexflo/memory/${encodeURIComponent(key)}`);
  }

  async memorySearch(query: string): Promise<HexFloResponse> {
    return this.call('GET', `/api/hexflo/memory/search?q=${encodeURIComponent(query)}`);
  }

  // ── Internal ───────────────────────────────────────

  private async call(method: string, path: string, body?: unknown): Promise<HexFloResponse> {
    try {
      const opts: RequestInit = {
        method,
        headers: { 'Content-Type': 'application/json' },
      };
      if (body) opts.body = JSON.stringify(body);
      const resp = await fetch(`${this.hubUrl}${path}`, opts);
      const data = await resp.json();
      return { ok: true, data };
    } catch (err) {
      return {
        ok: false,
        error: `HexFlo hub not running. Start with: hex daemon start\nError: ${err}`,
      };
    }
  }
}
