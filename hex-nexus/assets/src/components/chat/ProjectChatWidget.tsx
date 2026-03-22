import {
  type Component,
  For,
  Show,
  createSignal,
  onMount,
  onCleanup,
  createEffect,
} from "solid-js";
import {
  createProjectChat,
  type ChatMessage,
} from "../../stores/project-chat";

const ProjectChatWidget: Component<{
  projectId: string;
  onClose: () => void;
}> = (props) => {
  const chat = createProjectChat(props.projectId);
  const [input, setInput] = createSignal("");
  let messagesEnd: HTMLDivElement | undefined;
  let inputRef: HTMLInputElement | undefined;

  onMount(() => {
    chat.connect();
    inputRef?.focus();
  });
  onCleanup(() => chat.disconnect());

  // Auto-scroll when messages change or streaming text updates
  createEffect(() => {
    chat.messages();
    chat.streamingText();
    messagesEnd?.scrollIntoView({ behavior: "smooth" });
  });

  const handleSend = () => {
    const text = input().trim();
    if (!text) return;
    chat.send(text);
    setInput("");
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const formatTime = (ts: number) => {
    const d = new Date(ts);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  };

  return (
    <div
      class="flex w-[350px] min-w-[350px] flex-col border-l border-[var(--border-subtle)] bg-[var(--bg-base)]"
    >
      {/* Header */}
      <div
        class="flex items-center justify-between border-b border-[var(--border-subtle)] px-4 py-3"
      >
        <div class="flex items-center gap-2">
          <span
            class="text-[13px] font-semibold text-[var(--text-body)]"
          >
            Project Chat
          </span>
          <span
            class="h-1.5 w-1.5 rounded-full"
            classList={{
              "bg-status-active": chat.connected(),
              "bg-status-error": !chat.connected(),
            }}
          />
        </div>
        <div class="flex items-center gap-1">
          <button
            class="rounded p-1 transition-colors hover:bg-gray-800"
            onClick={() => chat.clear()}
            title="Clear messages"
          >
            <svg
              class="h-3.5 w-3.5 text-[var(--text-faint)]"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
            >
              <path d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" />
            </svg>
          </button>
          <button
            class="rounded p-1 transition-colors hover:bg-gray-800"
            onClick={props.onClose}
            title="Close chat"
          >
            <svg
              class="h-4 w-4 text-[var(--text-faint)]"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
            >
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      </div>

      {/* Messages */}
      <div class="flex-1 overflow-y-auto px-4 py-3 space-y-3">
        <For each={chat.messages()}>
          {(msg: ChatMessage) => (
            <div
              class={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
            >
              <div
                class="max-w-[85%] rounded-lg px-3 py-2 text-[12px]"
                style={{
                  background:
                    msg.role === "user"
                      ? "color-mix(in srgb, var(--accent) 20%, var(--bg-base))"
                      : msg.role === "tool"
                        ? "color-mix(in srgb, var(--purple) 10%, var(--bg-base))"
                        : "var(--bg-surface)",
                  color:
                    msg.role === "user"
                      ? "#93C5FD"
                      : msg.role === "tool"
                        ? "var(--purple)"
                        : "var(--text-secondary)",
                  border:
                    msg.role === "user"
                      ? "none"
                      : msg.role === "tool"
                        ? "1px solid color-mix(in srgb, var(--purple) 30%, transparent)"
                        : "1px solid var(--border-subtle)",
                }}
              >
                <Show when={msg.role === "tool" && msg.toolName}>
                  <div
                    class="mb-1 text-[10px] font-semibold text-[var(--purple)]"
                  >
                    {msg.toolName}
                  </div>
                </Show>
                <p class="whitespace-pre-wrap break-words">{msg.content}</p>
                <div
                  class="mt-1 text-[9px] text-[var(--text-dim)]"
                >
                  {formatTime(msg.timestamp)}
                </div>
              </div>
            </div>
          )}
        </For>

        {/* Streaming indicator */}
        <Show when={chat.streamingText()}>
          <div class="flex justify-start">
            <div
              class="max-w-[85%] rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 text-[12px] text-[var(--text-secondary)]"
            >
              <p class="whitespace-pre-wrap break-words">
                {chat.streamingText()}
              </p>
              <span
                class="inline-block h-2 w-2 animate-pulse rounded-full bg-[var(--accent-hover)]"
              />
            </div>
          </div>
        </Show>

        {/* Empty state */}
        <Show
          when={chat.messages().length === 0 && !chat.streamingText()}
        >
          <div class="flex flex-col items-center justify-center py-8 text-center">
            <svg
              class="mb-3 h-8 w-8 text-[var(--border)]"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.5"
            >
              <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" />
            </svg>
            <p class="text-[11px] text-[var(--text-faint)]">
              Ask about your project's architecture, ADRs, or get coding
              help.
            </p>
          </div>
        </Show>

        <div ref={messagesEnd} />
      </div>

      {/* Input bar */}
      <div
        class="flex items-center gap-2 border-t border-[var(--border-subtle)] px-3 py-3"
      >
        <input
          ref={inputRef}
          type="text"
          placeholder="Ask about this project..."
          value={input()}
          onInput={(e) => setInput(e.currentTarget.value)}
          onKeyDown={handleKeyDown}
          class="flex-1 rounded-md border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-3 py-2 text-[12px] text-[var(--text-body)] focus:outline-none"
        />
        <button
          onClick={handleSend}
          disabled={!input().trim() || !chat.connected()}
          class="shrink-0 rounded-md bg-[var(--accent)] px-3 py-2 text-[11px] font-medium text-white transition-colors disabled:opacity-40"
        >
          Send
        </button>
      </div>
    </div>
  );
};

export default ProjectChatWidget;
