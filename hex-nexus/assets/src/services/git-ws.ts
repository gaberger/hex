/**
 * git-ws.ts — Git WebSocket secondary adapter (ADR-056).
 *
 * Singleton service for real-time git event subscriptions.
 * Connects to /ws and filters events by project topic.
 */
import type { IWebSocketTransport, MessageHandler, StatusHandler } from '../types/services';

class GitWebSocketService implements IWebSocketTransport {
  private ws: WebSocket | null = null;
  private messageHandlers: MessageHandler[] = [];
  private statusHandlers: StatusHandler[] = [];
  private _connected = false;
  private subscribedProjectId: string | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  get connected(): boolean {
    return this._connected;
  }

  /** Subscribe to git events for a specific project. */
  subscribe(projectId: string): void {
    if (this.subscribedProjectId === projectId && this.ws?.readyState === WebSocket.OPEN) {
      return;
    }
    this.disconnect();
    this.subscribedProjectId = projectId;
    this.connect();
  }

  connect(): void {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const url = `${proto}//${location.host}/ws`;

    try {
      this.ws = new WebSocket(url);

      this.ws.onopen = () => {
        this._connected = true;
        this.notifyStatus(true);
      };

      this.ws.onmessage = (e) => {
        try {
          const msg = JSON.parse(e.data);
          const expectedTopic = `project:${this.subscribedProjectId}:git`;
          if (msg.topic !== expectedTopic) return;
          this.notifyMessage(msg);
        } catch { /* ignore parse errors */ }
      };

      this.ws.onclose = () => {
        this._connected = false;
        this.notifyStatus(false);
        // Auto-reconnect if still subscribed
        if (this.subscribedProjectId) {
          this.reconnectTimer = setTimeout(() => {
            this.reconnectTimer = null;
            if (this.subscribedProjectId) this.connect();
          }, 5000);
        }
      };

      this.ws.onerror = () => {
        this.ws?.close();
      };
    } catch { /* WebSocket unavailable */ }
  }

  disconnect(): void {
    this.subscribedProjectId = null;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.ws) {
      this.ws.onclose = null;
      this.ws.close();
      this.ws = null;
    }
    this._connected = false;
  }

  send(data: string | Record<string, unknown>): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;
    const payload = typeof data === 'string' ? data : JSON.stringify(data);
    this.ws.send(payload);
  }

  onMessage(handler: MessageHandler): void {
    this.messageHandlers.push(handler);
  }

  onStatus(handler: StatusHandler): void {
    this.statusHandlers.push(handler);
  }

  private notifyMessage(msg: any): void {
    for (const handler of this.messageHandlers) handler(msg);
  }

  private notifyStatus(connected: boolean): void {
    for (const handler of this.statusHandlers) handler(connected);
  }
}

/** Singleton git WebSocket service. */
export const gitWs = new GitWebSocketService();
