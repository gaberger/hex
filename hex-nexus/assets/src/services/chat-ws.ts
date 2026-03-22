/**
 * chat-ws.ts — Chat WebSocket secondary adapter (ADR-056).
 *
 * Singleton service owning the chat WebSocket connection lifecycle.
 * The chat store subscribes to events via callbacks — it never touches
 * WebSocket or fetch directly.
 */
import type { IChatTransport, MessageHandler, StatusHandler } from '../types/services';
import { restClient } from './rest-client';

type ParsedMessage = any; // raw parsed JSON from WebSocket

class ChatWebSocketService implements IChatTransport {
  private ws: WebSocket | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private reconnectDelay = 1000;
  private messageHandlers: MessageHandler[] = [];
  private statusHandlers: StatusHandler[] = [];
  private _connected = false;

  get connected(): boolean {
    return this._connected;
  }

  connect(): void {
    const token = localStorage.getItem('hex-auth-token')
      ?? new URLSearchParams(location.search).get('token');
    if (token) localStorage.setItem('hex-auth-token', token);

    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const host = location.host || 'localhost:5555';
    const url = token
      ? `${proto}//${host}/ws/chat?token=${encodeURIComponent(token)}`
      : `${proto}//${host}/ws/chat`;

    this.ws = new WebSocket(url);

    this.ws.onopen = () => {
      this._connected = true;
      this.reconnectDelay = 1000;
      this.notifyStatus(true);
    };

    this.ws.onclose = () => {
      this._connected = false;
      this.notifyStatus(false);
      this.scheduleReconnect();
    };

    this.ws.onerror = () => {
      this.ws?.close();
    };

    this.ws.onmessage = (e) => {
      try {
        const raw = JSON.parse(e.data);
        const msg = raw.event && raw.data
          ? { ...raw.data, type: raw.event }
          : raw;
        this.notifyMessage(msg);
      } catch (err) {
        console.error('[chat-ws] parse error', err);
      }
    };
  }

  disconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.ws?.close();
    this.ws = null;
    this._connected = false;
  }

  send(data: string | Record<string, unknown>): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const payload = typeof data === 'string' ? data : JSON.stringify(data);
    this.ws.send(payload);
  }

  sendChatMessage(content: string, options?: { model?: string; agentId?: string }): void {
    const payload: Record<string, string> = { type: 'chat_message', content };
    if (options?.model) payload.model = options.model;

    // @agent routing
    const atMatch = content.match(/^@(\S+)\s+([\s\S]*)$/);
    if (atMatch) {
      payload.agent_id = atMatch[1];
      payload.content = atMatch[2];
    } else if (options?.agentId) {
      payload.agent_id = options.agentId;
    }

    this.send(payload);
  }

  onMessage(handler: MessageHandler): void {
    this.messageHandlers.push(handler);
  }

  onStatus(handler: StatusHandler): void {
    this.statusHandlers.push(handler);
  }

  /** Load chat history for a session via REST. */
  async loadHistory(sessionId: string): Promise<any[]> {
    try {
      return await restClient.get(`/api/sessions/${encodeURIComponent(sessionId)}/messages`);
    } catch {
      console.warn('[chat-ws] failed to load history');
      return [];
    }
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, 30000);
      this.connect();
    }, this.reconnectDelay);
  }

  private notifyMessage(msg: ParsedMessage): void {
    for (const handler of this.messageHandlers) handler(msg);
  }

  private notifyStatus(connected: boolean): void {
    for (const handler of this.statusHandlers) handler(connected);
  }
}

/** Singleton chat WebSocket service. */
export const chatWs: IChatTransport & { loadHistory: (sid: string) => Promise<any[]> } = new ChatWebSocketService();
