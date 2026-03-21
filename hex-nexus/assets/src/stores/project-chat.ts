/**
 * project-chat.ts — Factory for per-project chat connections.
 *
 * Unlike the global chat store (chat.ts), this creates an isolated
 * WebSocket + signal set scoped to a single project. Used by the
 * ProjectChatWidget embedded in ProjectDetail.
 */
import { createSignal } from "solid-js";

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system" | "tool";
  content: string;
  timestamp: number;
  toolName?: string;
  toolInput?: string;
  toolResult?: string;
}

function makeId(): string {
  return crypto.randomUUID();
}

export function createProjectChat(projectId: string) {
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [streamingText, setStreamingText] = createSignal("");
  const [isStreaming, setIsStreaming] = createSignal(false);
  const [connected, setConnected] = createSignal(false);
  let ws: WebSocket | null = null;
  let reconnectTimer: number | undefined;
  let reconnectDelay = 1000;

  function connect() {
    if (ws && ws.readyState < 2) return; // already open or connecting

    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const host = location.host || "localhost:5555";
    const params = new URLSearchParams();
    params.set("project_id", projectId);

    // Check for auth token in localStorage
    const token = localStorage.getItem("stdb_token_hexflo-coordination");
    if (token) params.set("token", token);

    ws = new WebSocket(`${proto}//${host}/ws/chat?${params}`);

    ws.onopen = () => {
      setConnected(true);
      reconnectDelay = 1000;
    };

    ws.onclose = () => {
      setConnected(false);
      ws = null;
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
        // Normalize { event, data } envelope into flat { type, ...data }
        const msg =
          raw.event && raw.data ? { ...raw.data, type: raw.event } : raw;
        handleMessage(msg);
      } catch {
        // ignore parse errors
      }
    };
  }

  function handleMessage(msg: any) {
    switch (msg.type) {
      case "stream_chunk":
        setIsStreaming(true);
        setStreamingText((prev) => prev + (msg.text || msg.content || msg.chunk || ""));
        break;
      case "chat_message":
        finalizeStream(msg.content ?? "");
        break;
      case "tool_call":
        handleToolCall(msg);
        break;
      case "tool_result":
        handleToolResult(msg);
        break;
      case "agent_status":
        if (msg.status === "idle") finalizeStream();
        break;
      case "agent_disconnected":
        finalizeStream();
        break;
    }
  }

  function finalizeStream(content?: string) {
    const text = content || streamingText();
    if (text) {
      setMessages((prev) => [
        ...prev,
        {
          id: makeId(),
          role: "assistant",
          content: text,
          timestamp: Date.now(),
        },
      ]);
    }
    setStreamingText("");
    setIsStreaming(false);
  }

  function handleToolCall(msg: any) {
    const toolName = msg.tool_name || msg.name || "tool_call";
    const args = msg.arguments || msg.input || "";
    const toolInput = typeof args === "string" ? args : JSON.stringify(args, null, 2);

    setMessages((prev) => [
      ...prev,
      {
        id: makeId(),
        role: "tool",
        content: `${toolName}: ${toolInput}`,
        toolName,
        toolInput,
        timestamp: Date.now(),
      },
    ]);
  }

  function handleToolResult(msg: any) {
    const raw = msg.output || msg.result || msg.content || "";
    const resultText = typeof raw === "string" ? raw : JSON.stringify(raw, null, 2);

    setMessages((prev) => [
      ...prev,
      {
        id: makeId(),
        role: "tool",
        content: resultText,
        toolName: msg.tool_name || "result",
        toolResult: resultText,
        timestamp: Date.now(),
      },
    ]);
  }

  function send(text: string) {
    if (!ws || ws.readyState !== WebSocket.OPEN) return;

    // Add user message immediately
    setMessages((prev) => [
      ...prev,
      {
        id: makeId(),
        role: "user",
        content: text,
        timestamp: Date.now(),
      },
    ]);

    const payload: Record<string, string> = {
      type: "chat_message",
      content: text,
    };

    // @agent routing
    const atMatch = text.match(/^@(\S+)\s+([\s\S]*)$/);
    if (atMatch) {
      payload.agent_id = atMatch[1];
      payload.content = atMatch[2];
    }

    ws.send(JSON.stringify(payload));
  }

  function disconnect() {
    if (reconnectTimer) clearTimeout(reconnectTimer);
    ws?.close();
    ws = null;
    setConnected(false);
  }

  function clear() {
    setMessages([]);
    setStreamingText("");
    setIsStreaming(false);
  }

  return {
    messages,
    streamingText,
    isStreaming,
    connected,
    connect,
    disconnect,
    send,
    clear,
  };
}
