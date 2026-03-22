/**
 * services.ts — Frontend port interfaces (ADR-056).
 *
 * These interfaces decouple stores (domain) from I/O (services).
 * Stores import these types. Services implement them.
 * This file contains zero runtime code — only type definitions.
 */

// ── REST Client Port ──────────────────────────────────────────────

/** Generic HTTP client for communicating with hex-nexus REST API. */
export interface IRestClient {
  get<T = any>(path: string): Promise<T>;
  post<T = any>(path: string, body?: unknown): Promise<T>;
  put<T = any>(path: string, body?: unknown): Promise<T>;
  patch<T = any>(path: string, body?: unknown): Promise<T>;
  delete(path: string): Promise<void>;
}

// ── WebSocket Transport Port ──────────────────────────────────────

/** Callback invoked when a WebSocket message arrives. */
export type MessageHandler = (msg: any) => void;

/** Callback invoked when WebSocket connection status changes. */
export type StatusHandler = (connected: boolean) => void;

/**
 * Low-level WebSocket transport abstraction.
 * Decouples stores from the browser WebSocket API and
 * SpacetimeDB subscription protocol.
 */
export interface IWebSocketTransport {
  connect(): void;
  disconnect(): void;
  send(data: string | Record<string, unknown>): void;
  onMessage(handler: MessageHandler): void;
  onStatus(handler: StatusHandler): void;
  readonly connected: boolean;
}

// ── Chat Transport Port ───────────────────────────────────────────

/**
 * Specialised WebSocket transport for the chat subsystem.
 * Extends the base transport with chat-specific message framing
 * (model selection, agent routing).
 */
export interface IChatTransport extends IWebSocketTransport {
  /** Send a chat message with optional model and agent routing. */
  sendChatMessage(
    content: string,
    options?: {
      model?: string;
      agentId?: string;
    },
  ): void;
}

// ── Storage Adapter Port ──────────────────────────────────────────

/**
 * Key-value storage abstraction (e.g. localStorage, sessionStorage).
 * Used by stores that need to persist UI preferences or session tokens
 * without coupling to a specific browser API.
 */
export interface IStorageAdapter {
  get<T = string>(key: string): T | null;
  set<T = string>(key: string, value: T): void;
  remove(key: string): void;
  has(key: string): boolean;
}
