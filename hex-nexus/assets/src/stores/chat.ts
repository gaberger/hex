/**
 * chat.ts — Shared chat store (SolidJS signals + single persistent WebSocket).
 *
 * Extracts WebSocket management from ChatView.tsx so that both ChatView
 * and BottomBar can share the same connection. Fixes the critical bug
 * where BottomBar created a NEW WebSocket per message.
 */
import { createSignal } from 'solid-js';
import type { ChatMessage } from '../components/chat/Message';

// ── Signals ──────────────────────────────────────────────────────────────────

const [chatMessages, setChatMessages] = createSignal<ChatMessage[]>([]);
const [streamingText, setStreamingText] = createSignal('');
const [isStreaming, setIsStreaming] = createSignal(false);
const [chatConnected, setChatConnected] = createSignal(false);
const [loadingHistory, setLoadingHistory] = createSignal(false);

// ── Internal state ───────────────────────────────────────────────────────────

let ws: WebSocket | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let reconnectDelay = 1000;
let currentStreamAgent: string | null = null;

// ── Helpers ──────────────────────────────────────────────────────────────────

function makeId(): string {
  return crypto.randomUUID();
}

function nowISO(): string {
  return new Date().toISOString();
}

function getAuthToken(): string | null {
  const stored = localStorage.getItem('hex-auth-token');
  if (stored) return stored;
  const params = new URLSearchParams(location.search);
  const t = params.get('token');
  if (t) {
    localStorage.setItem('hex-auth-token', t);
    return t;
  }
  return null;
}

// ── History loading ──────────────────────────────────────────────────────────

/** Map a backend MessagePart to a flat ChatMessage. */
function backendMessageToChatMessage(msg: any): ChatMessage | null {
  const parts: any[] = msg.parts ?? [];
  if (parts.length === 0) return null;

  // Find the primary text part (if any)
  const textPart = parts.find((p: any) => p.type === 'Text' || p.content !== undefined && !p.call_id);
  const toolCallPart = parts.find((p: any) => p.type === 'ToolCall' || p.tool_name !== undefined);
  const toolResultPart = parts.find((p: any) => p.type === 'ToolResult' || (p.call_id !== undefined && p.is_error !== undefined));

  const role = (msg.role ?? 'user').toLowerCase() as ChatMessage['role'];
  const base = {
    id: msg.id ?? crypto.randomUUID(),
    role,
    timestamp: msg.created_at ?? new Date().toISOString(),
    model: msg.model ?? undefined,
  };

  if (toolCallPart) {
    const toolName = toolCallPart.tool_name ?? toolCallPart.name ?? 'tool';
    const args = toolCallPart.arguments ?? '';
    const toolInput = typeof args === 'string' ? args : JSON.stringify(args, null, 2);
    return {
      ...base,
      role: 'tool',
      content: `${toolName}: ${toolInput}`,
      toolName,
      toolInput,
      toolUseId: toolCallPart.call_id,
      toolResult: toolResultPart?.content,
      isError: toolResultPart?.is_error,
    };
  }

  const content = textPart?.content ?? parts[0]?.content ?? '';
  return { ...base, content };
}

/**
 * Fetch persisted messages for a session and populate the chatMessages signal.
 * Called on WebSocket connect and when switching sessions.
 */
async function loadChatHistory(sessionId?: string): Promise<void> {
  const sid = sessionId ?? localStorage.getItem('hex-active-session') ?? '';
  if (!sid) return;

  setLoadingHistory(true);
  try {
    const res = await fetch(`/api/sessions/${encodeURIComponent(sid)}/messages`);
    if (!res.ok) {
      console.warn(`[chat store] failed to load history: HTTP ${res.status}`);
      return;
    }
    const raw: any[] = await res.json();
    const messages: ChatMessage[] = [];
    for (const m of raw) {
      const mapped = backendMessageToChatMessage(m);
      if (mapped) messages.push(mapped);
    }
    setChatMessages(messages);
  } catch (err) {
    console.warn('[chat store] failed to load history:', err);
  } finally {
    setLoadingHistory(false);
  }
}

// ── WebSocket lifecycle ──────────────────────────────────────────────────────

function connect() {
  const token = getAuthToken();
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const host = location.host || 'localhost:5555';
  const url = token
    ? `${proto}//${host}/ws/chat?token=${encodeURIComponent(token)}`
    : `${proto}//${host}/ws/chat`;

  ws = new WebSocket(url);

  ws.onopen = () => {
    setChatConnected(true);
    reconnectDelay = 1000;
    // Load persisted chat history for the active session
    loadChatHistory();
  };

  ws.onclose = () => {
    setChatConnected(false);
    if (isStreaming()) endStream();
    scheduleReconnect();
  };

  ws.onerror = () => {
    ws?.close();
  };

  ws.onmessage = (e) => {
    try {
      const raw = JSON.parse(e.data);
      // Normalize { event, data } envelope into flat { type, ...data }
      const msg = raw.event && raw.data
        ? { ...raw.data, type: raw.event }
        : raw;
      handleMessage(msg);
    } catch (err) {
      console.error('[chat store] ws parse error', err);
    }
  };
}

function scheduleReconnect() {
  if (reconnectTimer) return;
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    reconnectDelay = Math.min(reconnectDelay * 2, 30000);
    connect();
  }, reconnectDelay);
}

// ── Message dispatch ─────────────────────────────────────────────────────────

function handleMessage(msg: any) {
  switch (msg.type) {
    case 'stream_chunk':
      handleStreamChunk(msg);
      break;
    case 'chat_message':
      addAssistantMessage(msg.content || '', msg.model);
      break;
    case 'tool_call':
      handleToolCall(msg);
      break;
    case 'tool_result':
      handleToolResult(msg);
      break;
    case 'agent_status':
      if (msg.status === 'idle' && isStreaming()) endStream();
      break;
    case 'agent_disconnected':
      if (isStreaming()) endStream();
      break;
    case 'connected':
      // connection ack — no action needed
      break;
    default:
      if (
        msg.type &&
        ['swarm_created', 'task_updated', 'agent_spawned', 'agent_terminated'].includes(msg.type)
      ) {
        addSystemMessage(formatHexFloEvent(msg));
      }
      break;
  }
}

// ── Streaming ────────────────────────────────────────────────────────────────

function handleStreamChunk(msg: any) {
  const incomingAgent = msg.agent_name || null;
  // If a different agent starts streaming, finalize the previous stream first
  if (isStreaming() && incomingAgent && incomingAgent !== currentStreamAgent) {
    endStream();
  }
  if (!isStreaming()) {
    setIsStreaming(true);
    currentStreamAgent = incomingAgent;
  }
  setStreamingText((prev) => prev + (msg.text || ''));
}

function endStream() {
  const text = streamingText();
  if (text) {
    setChatMessages((prev) => [
      ...prev,
      {
        id: makeId(),
        role: 'assistant',
        content: text,
        model: currentStreamAgent || undefined,
        timestamp: nowISO(),
      },
    ]);
  }
  setStreamingText('');
  setIsStreaming(false);
  currentStreamAgent = null;
}

// ── Message helpers ──────────────────────────────────────────────────────────

function addAssistantMessage(content: string, model?: string) {
  if (isStreaming()) endStream();
  setChatMessages((prev) => [
    ...prev,
    { id: makeId(), role: 'assistant', content, model, timestamp: nowISO() },
  ]);
}

function addSystemMessage(content: string) {
  setChatMessages((prev) => [
    ...prev,
    { id: makeId(), role: 'system', content, timestamp: nowISO() },
  ]);
}

function handleToolCall(msg: any) {
  const toolName = msg.tool_name || msg.name || 'tool_call';
  const args = msg.arguments || msg.input || '';
  const toolInput = typeof args === 'string' ? args : JSON.stringify(args, null, 2);
  const toolUseId = msg.tool_use_id || `${toolName}_${Date.now()}`;

  setChatMessages((prev) => [
    ...prev,
    {
      id: makeId(),
      role: 'tool' as const,
      content: `${toolName}: ${toolInput}`,
      toolName,
      toolInput,
      toolUseId,
      timestamp: nowISO(),
    },
  ]);
}

function handleToolResult(msg: any) {
  const raw = msg.output || msg.result || msg.content || '';
  const resultText = typeof raw === 'string' ? raw : JSON.stringify(raw, null, 2);
  const toolUseId = msg.tool_use_id;
  const isError = !!msg.is_error;

  if (toolUseId) {
    // Try to find and update the matching tool_call message
    setChatMessages((prev) => {
      const idx = prev.findIndex((m) => m.toolUseId === toolUseId);
      if (idx >= 0) {
        const updated = [...prev];
        updated[idx] = {
          ...updated[idx],
          toolResult: resultText,
          isError,
          content: `${updated[idx].toolName}: ${updated[idx].toolInput}\n\u2192 ${resultText}`,
        };
        return updated;
      }
      // No matching call found — add as new message
      return [
        ...prev,
        {
          id: makeId(),
          role: 'tool' as const,
          content: resultText,
          toolName: msg.tool_name || 'result',
          toolResult: resultText,
          isError,
          timestamp: nowISO(),
        },
      ];
    });
  } else {
    // No tool_use_id — just append
    setChatMessages((prev) => [
      ...prev,
      {
        id: makeId(),
        role: 'tool' as const,
        content: resultText,
        toolResult: resultText,
        isError,
        timestamp: nowISO(),
      },
    ]);
  }
}

function formatHexFloEvent(msg: any): string {
  switch (msg.type) {
    case 'swarm_created':
      return `Swarm created: ${msg.name || msg.id || 'new swarm'}`;
    case 'task_updated':
      return `Task ${msg.task_id || '?'} -> ${msg.status || 'updated'}`;
    case 'agent_spawned':
      return `Agent spawned: ${msg.agent?.name || msg.agent?.agent_name || msg.agent?.id || 'new agent'}`;
    case 'agent_terminated':
      return `Agent terminated: ${msg.agent_id || '?'}`;
    default:
      return msg.type;
  }
}

// ── Public actions ───────────────────────────────────────────────────────────

function sendMessage(text: string) {
  if (!ws || ws.readyState !== WebSocket.OPEN) return;

  // Add user message locally
  setChatMessages((prev) => [
    ...prev,
    { id: makeId(), role: 'user', content: text, timestamp: nowISO() },
  ]);

  // Build payload matching vanilla JS protocol
  const payload: Record<string, string> = { type: 'chat_message', content: text };

  // @agent routing
  const atMatch = text.match(/^@(\S+)\s+([\s\S]*)$/);
  if (atMatch) {
    payload.agent_id = atMatch[1];
    payload.content = atMatch[2];
  }

  ws.send(JSON.stringify(payload));
}

function clearMessages() {
  setChatMessages([]);
  setStreamingText('');
  setIsStreaming(false);
  currentStreamAgent = null;
}

function initChatConnection() {
  connect();
}

function disconnectChat() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  ws?.close();
  ws = null;
}

// ── Exports ──────────────────────────────────────────────────────────────────

export {
  // Connection
  chatConnected,
  initChatConnection,
  disconnectChat,
  // Messages
  chatMessages,
  streamingText,
  isStreaming,
  loadingHistory,
  // Actions
  sendMessage,
  clearMessages,
  loadChatHistory,
};
