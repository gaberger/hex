import { Component, Show, createSignal, type Accessor } from 'solid-js';

interface ChatInputProps {
  onSend: (text: string) => void;
  isStreaming: Accessor<boolean>;
}

const MAX_ROWS = 6;
const LINE_HEIGHT = 20; // px approx for text-sm

const ChatInput: Component<ChatInputProps> = (props) => {
  let textareaRef: HTMLTextAreaElement | undefined;
  const [value, setValue] = createSignal('');

  const autoGrow = () => {
    if (!textareaRef) return;
    textareaRef.style.height = 'auto';
    const maxHeight = LINE_HEIGHT * MAX_ROWS;
    textareaRef.style.height = Math.min(textareaRef.scrollHeight, maxHeight) + 'px';
  };

  const handleInput = (e: InputEvent) => {
    setValue((e.target as HTMLTextAreaElement).value);
    autoGrow();
  };

  const send = () => {
    const text = value().trim();
    if (!text || props.isStreaming()) return;
    props.onSend(text);
    setValue('');
    if (textareaRef) {
      textareaRef.style.height = 'auto';
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  return (
    <div class="border-t border-gray-700 bg-gray-900 px-4 py-3">
      <Show when={props.isStreaming()}>
        <div class="mb-2 flex items-center gap-2 text-xs text-cyan-400">
          <span class="inline-block h-2 w-2 rounded-full bg-cyan-400 animate-pulse" />
          Generating...
        </div>
      </Show>

      <div class="flex items-end gap-2">
        <textarea
          ref={textareaRef}
          value={value()}
          onInput={handleInput}
          onKeyDown={handleKeyDown}
          disabled={props.isStreaming()}
          placeholder={props.isStreaming() ? 'Waiting for response...' : 'Send a message... (Shift+Enter for newline)'}
          rows={1}
          class="flex-1 resize-none rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 placeholder-gray-500 outline-none transition-colors focus:border-cyan-500 disabled:opacity-50 disabled:cursor-not-allowed"
          style={{ "max-height": `${LINE_HEIGHT * MAX_ROWS}px` }}
        />
        <button
          onClick={send}
          disabled={props.isStreaming() || !value().trim()}
          class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-cyan-600 text-white transition-colors hover:bg-cyan-500 disabled:opacity-30 disabled:cursor-not-allowed"
          title="Send message"
        >
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="22" y1="2" x2="11" y2="13" />
            <polygon points="22 2 15 22 11 13 2 9 22 2" />
          </svg>
        </button>
      </div>

      <Show when={value().startsWith('/')}>
        <div class="mt-1 text-[10px] text-gray-300">
          Slash commands coming soon
        </div>
      </Show>
    </div>
  );
};

export default ChatInput;
