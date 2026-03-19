/**
 * HexFlo Client Port
 *
 * Defines the contract for interacting with the HexFlo coordination layer
 * (swarm init, task lifecycle, memory store/retrieve/search).
 *
 * Both CLI and MCP primary adapters consume this port — neither should
 * know about HTTP endpoints, URLs, or transport details.
 */

// ── Result types ─────────────────────────────────────

export interface HexFloResult {
  ok: true;
  data: unknown;
}

export interface HexFloError {
  ok: false;
  error: string;
}

export type HexFloResponse = HexFloResult | HexFloError;

// ── Port interface ───────────────────────────────────

export interface IHexFloClientPort {
  // Swarm lifecycle
  swarmInit(name: string, projectId: string, topology?: string): Promise<HexFloResponse>;
  swarmStatus(): Promise<HexFloResponse>;

  // Task lifecycle
  taskCreate(swarmId: string, title: string): Promise<HexFloResponse>;
  taskList(swarmId?: string): Promise<HexFloResponse>;
  taskComplete(taskId: string, result?: string): Promise<HexFloResponse>;

  // Memory
  memoryStore(key: string, value: string, scope?: string): Promise<HexFloResponse>;
  memoryRetrieve(key: string): Promise<HexFloResponse>;
  memorySearch(query: string): Promise<HexFloResponse>;
}
