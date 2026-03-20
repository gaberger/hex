import { Component, Show, createSignal } from 'solid-js';

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  model?: string;
  timestamp: string;
}

const roleBadgeStyles: Record<ChatMessage['role'], string> = {
  user: 'bg-blue-600/30 text-blue-300',
  assistant: 'bg-green-600/30 text-green-300',
  system: 'bg-gray-600/30 text-gray-300',
  tool: 'bg-purple-600/30 text-purple-300',
};

const roleLabels: Record<ChatMessage['role'], string> = {
  user: 'User',
  assistant: 'Assistant',
  system: 'System',
  tool: 'Tool',
};

const messageBgStyles: Record<ChatMessage['role'], string> = {
  user: 'bg-blue-900/20 border-l-2 border-blue-500',
  assistant: 'bg-gray-800/50',
  system: 'bg-gray-800/30 italic',
  tool: 'bg-gray-800/30 border-l-2 border-purple-500',
};

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
  const [expanded, setExpanded] = createSignal(false);

  return (
    <div class={`px-4 py-3 ${messageBgStyles[props.message.role]}`}>
      <div class="flex items-center gap-2 mb-1">
        <span class={`inline-block rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${roleBadgeStyles[props.message.role]}`}>
          {roleLabels[props.message.role]}
        </span>
        <Show when={props.message.model}>
          <span class="rounded bg-gray-700/60 px-1.5 py-0.5 text-[10px] text-gray-300 font-mono">
            {props.message.model}
          </span>
        </Show>
        <span class="ml-auto text-[10px] text-gray-300">
          {relativeTime(props.message.timestamp)}
        </span>
      </div>

      <Show when={props.message.role === 'tool'}>
        <button
          class="mb-1 text-[11px] text-purple-400 hover:text-purple-300 transition-colors"
          onClick={() => setExpanded(!expanded())}
        >
          {expanded() ? 'Hide output' : 'Show output'}
        </button>
        <Show when={expanded()}>
          <pre class="whitespace-pre-wrap break-words text-sm text-gray-300 font-mono bg-gray-900/50 rounded p-2">
            {props.message.content}
          </pre>
        </Show>
      </Show>

      <Show when={props.message.role !== 'tool'}>
        <div class="whitespace-pre-wrap break-words text-sm text-gray-300 leading-relaxed">
          {props.message.content}
        </div>
      </Show>
    </div>
  );
};

export default Message;
