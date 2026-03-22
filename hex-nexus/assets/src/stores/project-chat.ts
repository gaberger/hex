/**
 * project-chat.ts — Factory for per-project chat connections (ADR-056).
 *
 * Unlike the global chat store (chat.ts), this creates an isolated
 * WebSocket + signal set scoped to a single project. Used by the
 * ProjectChatWidget embedded in ProjectDetail.
 *
 * Note: This factory creates per-project WebSocket instances via a
 * lightweight transport wrapper, keeping the WebSocket lifecycle
 * inside the service layer (ADR-056 F2 compliance).
 */
import { createSignal } from "solid-js";
import { createProjectChatTransport } from '../services/project-chat-ws';

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

  // Delegate WebSocket lifecycle to the service layer (ADR-056)
  const transport = createProjectChatTransport(projectId);
  transport.onMessage(handleMessage);
  transport.onStatus(setConnected);

  function connect() {
    transport.connect();
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
    if (!transport.connected) return;

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

    transport.send(payload);
  }

  function disconnect() {
    transport.disconnect();
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
