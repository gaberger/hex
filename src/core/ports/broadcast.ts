/**
 * Broadcast Port
 *
 * Decouples event fan-out from transport (SSE, WebSocket, etc.).
 * DashboardHub uses this port to push events to connected clients
 * without knowing whether they're SSE responses or WebSocket connections.
 */

// ─── Types ───────────────────────────────────────────────

export interface BroadcastMessage {
  event: string;
  data: unknown;
  projectId?: string; // undefined = global broadcast
}

export interface BroadcastClient {
  id: string;
  projectFilter: string | null; // null = receive all projects
  write(event: string, data: unknown): void;
  close(): void;
}

// ─── Port (Secondary / Driven) ───────────────────────────

export interface IBroadcastPort {
  /** Push a message to all connected transport clients. */
  send(message: BroadcastMessage): void;

  /** Register a client connection. Returns an unregister function. */
  addClient(client: BroadcastClient): () => void;

  /** Remove a client by ID. */
  removeClient(clientId: string): void;

  /** Count of currently connected clients. */
  readonly clientCount: number;
}
