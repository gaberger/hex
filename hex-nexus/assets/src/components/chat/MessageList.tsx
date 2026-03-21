import { Component, For, Show, createEffect, createSignal, type Accessor } from 'solid-js';
import Message, { type ChatMessage } from './Message';

interface MessageListProps {
  messages: Accessor<ChatMessage[]>;
  streamingText: Accessor<string>;
}

const MessageList: Component<MessageListProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  const [autoScroll, setAutoScroll] = createSignal(true);

  const handleScroll = () => {
    if (!containerRef) return;
    const { scrollTop, scrollHeight, clientHeight } = containerRef;
    setAutoScroll(scrollHeight - scrollTop - clientHeight < 60);
  };

  createEffect(() => {
    props.messages();
    props.streamingText();
    if (autoScroll() && containerRef) {
      containerRef.scrollTo({ top: containerRef.scrollHeight, behavior: 'smooth' });
    }
  });

  return (
    <div
      ref={containerRef}
      class="flex-1 overflow-y-auto scroll-smooth"
      onScroll={handleScroll}
    >
      {/* Empty state */}
      <Show when={props.messages().length === 0 && !props.streamingText()}>
        <div class="flex h-full items-center justify-center">
          <div class="text-center" style={{ gap: '8px', display: 'flex', "flex-direction": 'column', "align-items": 'center' }}>
            <div class="text-[28px] font-light" style={{ color: 'var(--text-dim)' }}>hex</div>
            <p class="text-[14px]" style={{ color: 'var(--text-faint)' }}>No messages yet. Start a conversation below.</p>
          </div>
        </div>
      </Show>

      {/* Messages */}
      <div style={{ padding: '16px 24px', display: 'flex', "flex-direction": 'column', gap: '12px' }}>
        <For each={props.messages()}>
          {(msg) => <Message message={msg} />}
        </For>
      </div>

      {/* Streaming message (in-progress) */}
      <Show when={props.streamingText()}>
        <div style={{ padding: '12px 24px' }}>
          <div
            class="rounded-[10px]"
            style={{ padding: '12px 16px' }}
          >
            <div class="flex items-center gap-2 mb-1.5">
              <span
                class="inline-block rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
                style={{ background: 'rgba(74,222,128,0.2)', color: '#4ade80' }}
              >
                Assistant
              </span>
              <span class="ml-auto flex items-center gap-1 text-[10px]" style={{ color: 'var(--accent)' }}>
                <span class="inline-block h-1.5 w-1.5 rounded-full animate-pulse" style={{ background: 'var(--accent)' }} />
                streaming
              </span>
            </div>
            <div
              class="whitespace-pre-wrap break-words text-[15px] leading-[1.5] streaming-cursor"
              style={{ color: 'var(--text-secondary)' }}
            >
              {props.streamingText()}
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default MessageList;
