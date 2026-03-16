/**
 * WebSocket Broker
 *
 * Bidirectional communication layer for the dashboard hub.
 * Attaches to an existing HTTP server via upgrade handler.
 * Implements IBroadcastPort for outbound fan-out, plus handles
 * inbound messages from CLI agents and worktree workers.
 *
 * Wire protocol (JSON):
 *   Client → Server: { type: 'subscribe', topic: 'project:abc:*' }
 *   Client → Server: { type: 'unsubscribe', topic: 'project:abc:*' }
 *   Client → Server: { type: 'publish', topic: 'project:abc:task-progress', event: '...', data: {...} }
 *   Server → Client: { topic: 'project:abc:task-progress', event: '...', data: {...} }
 *
 * Topic convention:
 *   project:{id}:file-change
 *   project:{id}:task-progress
 *   project:{id}:agent-status
 *   project:{id}:decision-request
 *   hub:project-registered
 *   hub:project-unregistered
 *   hub:health
 */

import { randomUUID } from 'node:crypto';
import type { Server as HttpServer, IncomingMessage } from 'node:http';
import type { Duplex } from 'node:stream';
import type {
  IBroadcastPort,
  BroadcastClient,
  BroadcastMessage,
} from '../../core/ports/broadcast.js';

// ─── ws library types (resolved at runtime) ────────────────

type WsWebSocket = {
  readyState: number;
  send(data: string, cb?: (err?: Error) => void): void;
  close(code?: number, reason?: string): void;
  ping(data?: unknown, mask?: boolean, cb?: (err: Error) => void): void;
  pong(data?: unknown, mask?: boolean, cb?: (err: Error) => void): void;
  on(event: string, listener: (...args: unknown[]) => void): void;
  terminate(): void;
};

type WsWebSocketServer = {
  handleUpgrade(
    request: IncomingMessage,
    socket: Duplex,
    head: Buffer,
    cb: (ws: WsWebSocket) => void,
  ): void;
  close(cb?: (err?: Error) => void): void;
  clients: Set<WsWebSocket>;
};

// ─── Internal types ─────────────────────────────────────────

interface ClientEntry {
  id: string;
  ws: WsWebSocket;
  connectedAt: Date;
  subscriptions: Set<string>;
  projectFilter: string | null;
  remoteAddress: string | undefined;
  authenticated: boolean;
  alive: boolean;
}

interface InboundSubscribe {
  type: 'subscribe';
  topic: string;
}

interface InboundUnsubscribe {
  type: 'unsubscribe';
  topic: string;
}

interface InboundPublish {
  type: 'publish';
  topic: string;
  event: string;
  data: unknown;
}

type InboundMessage = InboundSubscribe | InboundUnsubscribe | InboundPublish;

interface OutboundEnvelope {
  topic: string;
  event: string;
  data: unknown;
}

// ─── Topic matching ─────────────────────────────────────────

/**
 * Match a topic against a subscription pattern.
 * Supports trailing wildcard: `project:abc:*` matches `project:abc:file-change`.
 * Exact match also supported: `hub:health` matches `hub:health`.
 */
function topicMatches(pattern: string, topic: string): boolean {
  if (pattern === topic) return true;
  if (pattern.endsWith(':*')) {
    const prefix = pattern.slice(0, -1); // keep the trailing colon
    return topic.startsWith(prefix);
  }
  return false;
}

// ─── WebSocket Broker ───────────────────────────────────────

export interface WsBrokerOptions {
  /** Expected auth token. If set, publish requires token validation. */
  authToken?: string;
  /** Heartbeat interval in ms (default 30000). */
  heartbeatMs?: number;
  /** Path to accept WebSocket upgrades on (default '/ws'). */
  path?: string;
}

export class WsBroker implements IBroadcastPort {
  private readonly clients = new Map<string, ClientEntry>();
  private readonly broadcastClients = new Map<string, BroadcastClient>();
  private readonly authToken: string | undefined;
  private readonly heartbeatMs: number;
  private readonly wsPath: string;
  private wss: WsWebSocketServer | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private onInboundPublish: ((topic: string, event: string, data: unknown) => void) | null = null;

  constructor(options: WsBrokerOptions = {}) {
    this.authToken = options.authToken;
    this.heartbeatMs = options.heartbeatMs ?? 30_000;
    this.wsPath = options.path ?? '/ws';
  }

  // ─── Lifecycle ──────────────────────────────────────────

  /**
   * Attach the broker to an existing HTTP server.
   * Installs an `upgrade` listener to intercept WebSocket handshakes.
   * Throws if the `ws` library is not installed.
   */
  attach(server: HttpServer): void {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    let WebSocketServer: new (opts: { noServer: true }) => WsWebSocketServer;
    try {
      // Dynamic require so the module is optional at install time
      const ws = require('ws') as { WebSocketServer: typeof WebSocketServer };
      WebSocketServer = ws.WebSocketServer;
    } catch {
      throw new Error(
        'The "ws" package is required for WebSocket support. Install it with: npm install ws',
      );
    }

    this.wss = new WebSocketServer({ noServer: true });

    server.on('upgrade', (req: IncomingMessage, socket: Duplex, head: Buffer) => {
      const url = new URL(req.url ?? '/', `http://${req.headers.host ?? 'localhost'}`);
      if (url.pathname !== this.wsPath) {
        // Not for us — let other upgrade handlers deal with it
        return;
      }

      const token = url.searchParams.get('token');
      const authenticated = this.validateToken(token);

      this.wss!.handleUpgrade(req, socket, head, (ws) => {
        this.onConnection(ws, req, authenticated);
      });
    });

    this.startHeartbeat();
  }

  /**
   * Shut down the broker: terminate all clients, stop heartbeat, close WSS.
   */
  close(): void {
    this.stopHeartbeat();

    for (const entry of this.clients.values()) {
      entry.ws.close(1001, 'Server shutting down');
    }
    this.clients.clear();
    this.broadcastClients.clear();

    if (this.wss) {
      this.wss.close();
      this.wss = null;
    }
  }

  // ─── IBroadcastPort implementation ──────────────────────

  send(message: BroadcastMessage): void {
    const { event, data, projectId } = message;

    if (projectId) {
      // Fan out to all topic subscribers matching project:{id}:*
      // Also send to BroadcastClients with matching projectFilter
      this.fanOutToTopic(`project:${projectId}:${event}`, event, data);
    } else {
      // Global broadcast — goes to every connected client
      this.fanOutToAll(event, data);
    }

    // Also deliver to BroadcastClient registrations (e.g., SSE adapter coexistence)
    for (const bc of this.broadcastClients.values()) {
      if (!projectId || !bc.projectFilter || bc.projectFilter === projectId) {
        bc.write(event, data);
      }
    }
  }

  addClient(client: BroadcastClient): () => void {
    this.broadcastClients.set(client.id, client);
    return () => this.removeClient(client.id);
  }

  removeClient(clientId: string): void {
    this.broadcastClients.delete(clientId);
  }

  get clientCount(): number {
    return this.clients.size + this.broadcastClients.size;
  }

  // ─── Inbound publish hook ───────────────────────────────

  /**
   * Register a callback for inbound publish messages from agents.
   * The dashboard hub can use this to receive events pushed by CLI agents.
   */
  onPublish(handler: (topic: string, event: string, data: unknown) => void): void {
    this.onInboundPublish = handler;
  }

  // ─── WebSocket client metadata ──────────────────────────

  /**
   * Return metadata for all connected WebSocket clients.
   */
  getClients(): Array<{
    id: string;
    connectedAt: Date;
    subscriptions: string[];
    projectFilter: string | null;
    remoteAddress: string | undefined;
    authenticated: boolean;
  }> {
    return Array.from(this.clients.values()).map((c) => ({
      id: c.id,
      connectedAt: c.connectedAt,
      subscriptions: Array.from(c.subscriptions),
      projectFilter: c.projectFilter,
      remoteAddress: c.remoteAddress,
      authenticated: c.authenticated,
    }));
  }

  // ─── Private: connection handling ───────────────────────

  private onConnection(ws: WsWebSocket, req: IncomingMessage, authenticated: boolean): void {
    const id = randomUUID();
    const entry: ClientEntry = {
      id,
      ws,
      connectedAt: new Date(),
      subscriptions: new Set(),
      projectFilter: null,
      remoteAddress: req.socket.remoteAddress,
      authenticated,
      alive: true,
    };

    this.clients.set(id, entry);

    // Send welcome message with client ID
    this.sendToWs(ws, {
      topic: 'hub:health',
      event: 'connected',
      data: { clientId: id, authenticated },
    });

    ws.on('message', (raw: unknown) => {
      this.handleMessage(entry, raw);
    });

    ws.on('close', () => {
      this.clients.delete(id);
    });

    ws.on('error', () => {
      this.clients.delete(id);
      ws.terminate();
    });

    ws.on('pong', () => {
      entry.alive = true;
    });
  }

  private handleMessage(entry: ClientEntry, raw: unknown): void {
    let msg: InboundMessage;
    try {
      const text = typeof raw === 'string' ? raw : String(raw);
      msg = JSON.parse(text) as InboundMessage;
    } catch {
      this.sendToWs(entry.ws, {
        topic: 'hub:health',
        event: 'error',
        data: { message: 'Invalid JSON' },
      });
      return;
    }

    switch (msg.type) {
      case 'subscribe':
        if (typeof msg.topic === 'string' && msg.topic.length > 0) {
          entry.subscriptions.add(msg.topic);
          // Infer projectFilter from subscription
          const projectMatch = msg.topic.match(/^project:([^:]+)/);
          if (projectMatch) {
            entry.projectFilter = projectMatch[1];
          }
        }
        break;

      case 'unsubscribe':
        if (typeof msg.topic === 'string') {
          entry.subscriptions.delete(msg.topic);
        }
        break;

      case 'publish':
        // Publishing requires authentication when authToken is configured
        if (this.authToken && !entry.authenticated) {
          this.sendToWs(entry.ws, {
            topic: 'hub:health',
            event: 'error',
            data: { message: 'Authentication required to publish' },
          });
          return;
        }

        if (typeof msg.topic === 'string' && typeof msg.event === 'string') {
          // Forward to matching subscribers
          this.fanOutToTopic(msg.topic, msg.event, msg.data);

          // Notify hub via callback
          if (this.onInboundPublish) {
            this.onInboundPublish(msg.topic, msg.event, msg.data);
          }
        }
        break;

      default:
        this.sendToWs(entry.ws, {
          topic: 'hub:health',
          event: 'error',
          data: { message: `Unknown message type: ${(msg as { type?: string }).type}` },
        });
    }
  }

  // ─── Private: fan-out ───────────────────────────────────

  private fanOutToTopic(topic: string, event: string, data: unknown): void {
    const envelope: OutboundEnvelope = { topic, event, data };
    for (const entry of this.clients.values()) {
      for (const pattern of entry.subscriptions) {
        if (topicMatches(pattern, topic)) {
          this.sendToWs(entry.ws, envelope);
          break; // Don't send duplicate to same client
        }
      }
    }
  }

  private fanOutToAll(event: string, data: unknown): void {
    const envelope: OutboundEnvelope = { topic: 'hub:broadcast', event, data };
    for (const entry of this.clients.values()) {
      this.sendToWs(entry.ws, envelope);
    }
  }

  private sendToWs(ws: WsWebSocket, envelope: OutboundEnvelope): void {
    // readyState 1 === OPEN
    if (ws.readyState === 1) {
      ws.send(JSON.stringify(envelope));
    }
  }

  // ─── Private: heartbeat ─────────────────────────────────

  private startHeartbeat(): void {
    this.heartbeatTimer = setInterval(() => {
      for (const [id, entry] of this.clients) {
        if (!entry.alive) {
          // Client didn't respond to last ping — terminate
          entry.ws.terminate();
          this.clients.delete(id);
          continue;
        }
        entry.alive = false;
        entry.ws.ping();
      }
    }, this.heartbeatMs);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  // ─── Private: auth ──────────────────────────────────────

  private validateToken(token: string | null): boolean {
    if (!this.authToken) return true; // No auth configured
    return token === this.authToken;
  }
}

// ─── CLI Agent Helper ─────────────────────────────────────

/**
 * Connect a CLI agent to the dashboard hub's WebSocket broker.
 * Returns a simple publish/subscribe interface.
 *
 * Usage:
 *   const conn = await connectToHub(4200, 'my-secret-token');
 *   conn.subscribe('project:abc:*');
 *   conn.publish('project:abc:task-progress', 'task-update', { taskId: '...', status: 'done' });
 *   conn.close();
 */
export async function connectToHub(
  port: number,
  token?: string,
): Promise<{
  subscribe(topic: string): void;
  unsubscribe(topic: string): void;
  publish(topic: string, event: string, data: unknown): void;
  onMessage(handler: (envelope: OutboundEnvelope) => void): void;
  close(): void;
}> {
  let WebSocket: new (url: string) => WsWebSocket;
  try {
    const ws = require('ws') as { default?: new (url: string) => WsWebSocket; WebSocket?: new (url: string) => WsWebSocket };
    WebSocket = ws.default ?? ws.WebSocket ?? (ws as unknown as new (url: string) => WsWebSocket);
  } catch {
    throw new Error(
      'The "ws" package is required for WebSocket support. Install it with: npm install ws',
    );
  }

  const url = `ws://127.0.0.1:${port}/ws${token ? `?token=${encodeURIComponent(token)}` : ''}`;

  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    let messageHandler: ((envelope: OutboundEnvelope) => void) | null = null;

    ws.on('open', () => {
      resolve({
        subscribe(topic: string) {
          ws.send(JSON.stringify({ type: 'subscribe', topic }));
        },
        unsubscribe(topic: string) {
          ws.send(JSON.stringify({ type: 'unsubscribe', topic }));
        },
        publish(topic: string, event: string, data: unknown) {
          ws.send(JSON.stringify({ type: 'publish', topic, event, data }));
        },
        onMessage(handler: (envelope: OutboundEnvelope) => void) {
          messageHandler = handler;
        },
        close() {
          ws.close(1000, 'Client disconnect');
        },
      });
    });

    ws.on('message', (raw: unknown) => {
      if (messageHandler) {
        try {
          const text = typeof raw === 'string' ? raw : String(raw);
          const envelope = JSON.parse(text) as OutboundEnvelope;
          messageHandler(envelope);
        } catch {
          // Ignore malformed messages
        }
      }
    });

    ws.on('error', (err: unknown) => {
      reject(new Error(`WebSocket connection failed: ${err}`));
    });
  });
}
