import { Component, createSignal, onMount, onCleanup } from 'solid-js';
import MessageList from './MessageList';
import ChatInput from './ChatInput';
import type { ChatMessage } from './Message';

/**
 * ChatView — main chat container for the center pane.
 *
 * Manages a WebSocket connection to /ws/chat for LLM streaming.
 * Messages are held in local signals. The WebSocket protocol mirrors
 * the vanilla JS chat: JSON frames with { type, ... } or { event, data }.
 *
 * Message flow:
 *   User sends  -> { type: "chat_message", content: "..." }
 *   Server sends -> stream_chunk { text }, chat_message { content }, tool_call, tool_result, etc.
 */
const ChatView: Component = () => {
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [streamingText, setStreamingText] = createSignal('');
  const [isStreaming, setIsStreaming] = createSignal(false);
  const [connected, setConnected] = createSignal(false);
  const [sessionId] = createSignal(crypto.randomUUID());

  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let reconnectDelay = 1000;
  let currentStreamAgent: string | null = null;

  // -- helpers ---------------------------------------------------------------

  function makeId(): string {
    return crypto.randomUUID();
  }

  function nowISO(): string {
    return new Date().toISOString();
  }

  function getAuthToken(): string | null {
    const h = location.hash.slice(1);
    if (h) return h;
    const params = new URLSearchParams(location.search);
    const t = params.get('token');
    if (t) {
      location.hash = t;
      return t;
    }
    return null;
  }

  // -- WebSocket -------------------------------------------------------------

  function connect() {
    const token = getAuthToken();
    // Allow connection even without a token (nexus may not require auth)
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const host = location.host || 'localhost:5555';
    const url = token
      ? `${proto}//${host}/ws/chat?token=${encodeURIComponent(token)}`
      : `${proto}//${host}/ws/chat`;

    ws = new WebSocket(url);

    ws.onopen = () => {
      setConnected(true);
      reconnectDelay = 1000;
    };

    ws.onclose = () => {
      setConnected(false);
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
        console.error('[ChatView] ws parse error', err);
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

  function disconnect() {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    ws?.close();
    ws = null;
  }

  // -- message dispatch ------------------------------------------------------

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
        if ((msg.status === 'idle') && isStreaming()) endStream();
        break;
      case 'agent_disconnected':
        if (isStreaming()) endStream();
        break;
      case 'connected':
        // connection ack — no action needed
        break;
      default:
        // system-level events (swarm_created, task_updated, etc.)
        if (msg.type && ['swarm_created', 'task_updated', 'agent_spawned', 'agent_terminated'].includes(msg.type)) {
          addSystemMessage(formatHexFloEvent(msg));
        }
        break;
    }
  }

  // -- streaming -------------------------------------------------------------

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
      setMessages((prev) => [
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

  // -- message helpers -------------------------------------------------------

  function addAssistantMessage(content: string, model?: string) {
    // If we were streaming, end that first
    if (isStreaming()) endStream();
    setMessages((prev) => [
      ...prev,
      { id: makeId(), role: 'assistant', content, model, timestamp: nowISO() },
    ]);
  }

  function addSystemMessage(content: string) {
    setMessages((prev) => [
      ...prev,
      { id: makeId(), role: 'system', content, timestamp: nowISO() },
    ]);
  }

  function handleToolCall(msg: any) {
    const label = msg.tool_name || msg.name || 'tool_call';
    const args = msg.arguments || msg.input || '';
    const content = typeof args === 'string' ? `${label}: ${args}` : `${label}: ${JSON.stringify(args, null, 2)}`;
    setMessages((prev) => [
      ...prev,
      { id: makeId(), role: 'tool', content, timestamp: nowISO() },
    ]);
  }

  function handleToolResult(msg: any) {
    const content = msg.output || msg.result || msg.content || '';
    setMessages((prev) => [
      ...prev,
      { id: makeId(), role: 'tool', content: typeof content === 'string' ? content : JSON.stringify(content, null, 2), timestamp: nowISO() },
    ]);
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

  // -- send ------------------------------------------------------------------

  function handleSend(text: string) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;

    // Add user message locally
    setMessages((prev) => [
      ...prev,
      { id: makeId(), role: 'user', content: text, timestamp: nowISO() },
    ]);

    // Build payload matching vanilla JS protocol
    const payload: any = { type: 'chat_message', content: text };

    // @agent routing
    const atMatch = text.match(/^@(\S+)\s+([\s\S]*)$/);
    if (atMatch) {
      payload.agent_id = atMatch[1];
      payload.content = atMatch[2];
    }

    ws.send(JSON.stringify(payload));
  }

  // -- lifecycle -------------------------------------------------------------

  onMount(() => {
    connect();
  });

  onCleanup(() => {
    disconnect();
  });

  // -- render ----------------------------------------------------------------

  return (
    <div class="flex h-full flex-col bg-gray-950">
      {/* Connection indicator */}
      <div class="flex items-center gap-2 border-b border-gray-800 px-4 py-1.5">
        <span
          class="h-2 w-2 rounded-full transition-colors"
          classList={{
            'bg-green-500': connected(),
            'bg-red-500': !connected(),
          }}
        />
        <span class="text-[11px] text-gray-300">
          {connected() ? 'connected' : 'disconnected'}
        </span>
      </div>

      <MessageList messages={messages} streamingText={streamingText} />
      <ChatInput onSend={handleSend} isStreaming={isStreaming} />
    </div>
  );
};

export default ChatView;
