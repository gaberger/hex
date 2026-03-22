/**
 * chat.ts — Shared chat store (SolidJS signals).
 *
 * All WebSocket and HTTP I/O is delegated to the chatWs service (ADR-056).
 * This store owns only reactive state and message-handling logic.
 */
import { createSignal } from 'solid-js';
import type { ChatMessage } from '../types/chat';
import { chatWs } from '../services/chat-ws';

// ── Signals ──────────────────────────────────────────────────────────────────

const [chatMessages, setChatMessages] = createSignal<ChatMessage[]>([]);
const [streamingText, setStreamingText] = createSignal('');
const [isStreaming, setIsStreaming] = createSignal(false);
const [chatConnected, setChatConnected] = createSignal(false);
const [loadingHistory, setLoadingHistory] = createSignal(false);
const [selectedModel, setSelectedModel] = createSignal<string>('');

// ── Internal state ───────────────────────────────────────────────────────────

let currentStreamAgent: string | null = null;

// ── Helpers ──────────────────────────────────────────────────────────────────

function makeId(): string {
  return crypto.randomUUID();
}

function nowISO(): string {
  return new Date().toISOString();
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
  const sid = sessionId ?? '';
  if (!sid) return;

  setLoadingHistory(true);
  try {
    const raw: any[] = await chatWs.loadHistory(sid);
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
      // connection ack — load history for active session
      loadChatHistory();
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
  const toolName = msg.tool_name || '';

  setChatMessages((prev) => {
    // Strategy 1: exact match by toolUseId
    let idx = toolUseId
      ? prev.findIndex((m) => m.toolUseId === toolUseId)
      : -1;

    // Strategy 2: fallback — find most recent unmatched tool call with same name prefix
    if (idx < 0 && toolName) {
      for (let i = prev.length - 1; i >= 0; i--) {
        const m = prev[i];
        if (m.role === 'tool' && !m.toolResult && m.toolName &&
            (m.toolName === toolName || m.toolName.startsWith(toolName) || toolName.startsWith(m.toolName))) {
          idx = i;
          break;
        }
      }
    }

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

    // No match at all — append as standalone result
    return [
      ...prev,
      {
        id: makeId(),
        role: 'tool' as const,
        content: resultText,
        toolName: toolName || 'result',
        toolResult: resultText,
        isError,
        timestamp: nowISO(),
      },
    ];
  });
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
  if (!chatWs.connected) return;

  // Add user message locally
  setChatMessages((prev) => [
    ...prev,
    { id: makeId(), role: 'user', content: text, timestamp: nowISO() },
  ]);

  // Delegate to transport service
  chatWs.sendChatMessage(text, { model: selectedModel() || undefined });
}

function clearMessages() {
  setChatMessages([]);
  setStreamingText('');
  setIsStreaming(false);
  currentStreamAgent = null;
}

function initChatConnection() {
  chatWs.onMessage(handleMessage);
  chatWs.onStatus((connected) => {
    setChatConnected(connected);
    if (!connected && isStreaming()) endStream();
  });
  chatWs.connect();
}

function disconnectChat() {
  chatWs.disconnect();
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
  // Model selection
  selectedModel,
  setSelectedModel,
};
