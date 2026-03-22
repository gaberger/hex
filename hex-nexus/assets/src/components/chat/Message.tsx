import { Component, Show } from 'solid-js';
import MarkdownContent from './MarkdownContent';
import ToolCallCard from './ToolCallCard';

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  model?: string;
  timestamp: string;
  toolName?: string;
  toolInput?: string;
  toolResult?: string;
  toolUseId?: string;
  isError?: boolean;
}

function relativeTime(iso: string): string {
  const now = Date.now();
  const then = new Date(iso).getTime();
  if (isNaN(then)) return '';
  const diffSec = Math.floor((now - then) / 1000);
  if (diffSec < 5) return 'just now';
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.floor(diffHr / 24);
  return `${diffDay}d ago`;
}

const Message: Component<{ message: ChatMessage }> = (props) => {
  const toolInfo = () => {
    const msg = props.message;
    if (msg.role !== 'tool') return { name: '', input: '', result: '', isError: false };
    if (msg.toolName) {
      return {
        name: msg.toolName,
        input: msg.toolInput || '',
        result: msg.toolResult || msg.content,
        isError: !!msg.isError,
      };
    }
    const colonIdx = msg.content.indexOf(': ');
    return {
      name: colonIdx > 0 ? msg.content.slice(0, colonIdx) : 'tool',
      input: colonIdx > 0 ? msg.content.slice(colonIdx + 2) : msg.content,
      result: '',
      isError: false,
    };
  };

  const isUser = () => props.message.role === 'user';

  return (
    <div
      class="rounded-[10px] px-4 py-3"
      classList={{
        "bg-[var(--bg-surface)] border border-blue-900/25": isUser(),
        "bg-transparent border-0": !isUser(),
      }}
    >
      {/* Role badge row */}
      <div class="flex items-center gap-2 mb-1.5">
        <span
          class="inline-block rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
          classList={{
            "bg-blue-500/20 text-blue-400": isUser(),
            "bg-green-500/20 text-green-400": !isUser(),
          }}
        >
          {props.message.role === 'user' ? 'User' : props.message.role === 'assistant' ? 'Discovery' : props.message.role}
        </span>
        <Show when={props.message.model}>
          <span
            class="rounded bg-[var(--bg-elevated)] px-1.5 py-0.5 text-[10px] font-mono text-[var(--text-muted)]"
          >
            {props.message.model}
          </span>
        </Show>
        <span class="ml-auto text-[10px] text-[var(--text-faint)]">
          {relativeTime(props.message.timestamp)}
        </span>
      </div>

      {/* Content */}
      <Show when={props.message.role === 'tool'}>
        <ToolCallCard
          toolName={toolInfo().name}
          input={toolInfo().input}
          result={toolInfo().result}
          isError={toolInfo().isError}
        />
      </Show>

      <Show when={props.message.role === 'assistant'}>
        <MarkdownContent content={props.message.content} />
      </Show>

      <Show when={props.message.role === 'user'}>
        <div
          class="whitespace-pre-wrap break-words text-[15px] leading-[1.5] text-[var(--text-secondary)]"
        >
          {props.message.content}
        </div>
      </Show>

      <Show when={props.message.role === 'system'}>
        <div
          class="whitespace-pre-wrap break-words text-[14px] leading-relaxed italic text-[var(--text-muted)]"
        >
          {props.message.content}
        </div>
      </Show>
    </div>
  );
};

export default Message;
