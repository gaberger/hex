/**
 * project-chat-ws.ts — Per-project chat WebSocket factory (ADR-056).
 *
 * Unlike chat-ws.ts (singleton), this creates isolated WebSocket
 * connections scoped to a single project. Used by project-chat store.
 */
import { storage } from './local-storage';

export interface ProjectChatTransport {
  connect(): void;
  disconnect(): void;
  send(payload: Record<string, string>): void;
  onMessage(handler: (msg: any) => void): void;
  onStatus(handler: (connected: boolean) => void): void;
  readonly connected: boolean;
}

export function createProjectChatTransport(projectId: string): ProjectChatTransport {
  let ws: WebSocket | null = null;
  let reconnectTimer: number | undefined;
  let reconnectDelay = 1000;
  let _connected = false;
  const messageHandlers: ((msg: any) => void)[] = [];
  const statusHandlers: ((connected: boolean) => void)[] = [];

  function connect(): void {
    if (ws && ws.readyState < 2) return;

    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const host = location.host || 'localhost:5555';
    const params = new URLSearchParams();
    params.set('project_id', projectId);

    const token = storage.get<string>('stdb_token_hexflo-coordination');
    if (token) params.set('token', token);

    ws = new WebSocket(`${proto}//${host}/ws/chat?${params}`);

    ws.onopen = () => {
      _connected = true;
      reconnectDelay = 1000;
      for (const h of statusHandlers) h(true);
    };

    ws.onclose = () => {
      _connected = false;
      ws = null;
      for (const h of statusHandlers) h(false);
      reconnectTimer = window.setTimeout(() => {
        reconnectDelay = Math.min(reconnectDelay * 1.5, 15000);
        connect();
      }, reconnectDelay);
    };

    ws.onerror = () => {
      ws?.close();
    };

    ws.onmessage = (ev) => {
      try {
        const raw = JSON.parse(ev.data);
        const msg = raw.event && raw.data
          ? { ...raw.data, type: raw.event }
          : raw;
        for (const h of messageHandlers) h(msg);
      } catch { /* ignore parse errors */ }
    };
  }

  function disconnect(): void {
    if (reconnectTimer) clearTimeout(reconnectTimer);
    ws?.close();
    ws = null;
    _connected = false;
  }

  function send(payload: Record<string, string>): void {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify(payload));
  }

  function onMessage(handler: (msg: any) => void): void {
    messageHandlers.push(handler);
  }

  function onStatus(handler: (connected: boolean) => void): void {
    statusHandlers.push(handler);
  }

  return {
    connect,
    disconnect,
    send,
    onMessage,
    onStatus,
    get connected() { return _connected; },
  };
}
