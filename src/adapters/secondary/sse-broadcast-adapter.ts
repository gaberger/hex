/**
 * SSE Broadcast Adapter
 *
 * Implements IBroadcastPort using Server-Sent Events.
 * Owns the client set and handles fan-out with project-scoped filtering.
 * This adapter is injected into DashboardHub so the hub doesn't manage
 * transport details directly.
 */

import type {
  IBroadcastPort,
  BroadcastClient,
  BroadcastMessage,
} from '../../core/ports/broadcast.js';

export class SSEBroadcastAdapter implements IBroadcastPort {
  private readonly clients = new Map<string, BroadcastClient>();

  get clientCount(): number {
    return this.clients.size;
  }

  send(message: BroadcastMessage): void {
    const payload = JSON.stringify(message.data);
    for (const client of this.clients.values()) {
      // Global broadcast or matching project filter
      if (!message.projectId || !client.projectFilter || client.projectFilter === message.projectId) {
        try {
          client.write(message.event, payload);
        } catch {
          // Client may have disconnected — remove on next tick
          this.clients.delete(client.id);
        }
      }
    }
  }

  addClient(client: BroadcastClient): () => void {
    this.clients.set(client.id, client);
    return () => this.clients.delete(client.id);
  }

  removeClient(clientId: string): void {
    const client = this.clients.get(clientId);
    if (client) {
      try { client.close(); } catch { /* already closed */ }
      this.clients.delete(clientId);
    }
  }
}
