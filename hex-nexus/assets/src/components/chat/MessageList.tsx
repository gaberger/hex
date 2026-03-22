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
          <div class="flex flex-col items-center gap-2 text-center">
            <div class="text-[28px] font-light text-[var(--text-dim)]">hex</div>
            <p class="text-[14px] text-[var(--text-faint)]">No messages yet. Start a conversation below.</p>
          </div>
        </div>
      </Show>

      {/* Messages */}
      <div class="flex flex-col gap-3 px-6 py-4">
        <For each={props.messages()}>
          {(msg) => <Message message={msg} />}
        </For>
      </div>

      {/* Streaming message (in-progress) */}
      <Show when={props.streamingText()}>
        <div class="px-6 py-3">
          <div
            class="rounded-[10px] px-4 py-3"
          >
            <div class="flex items-center gap-2 mb-1.5">
              <span
                class="inline-block rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide"
                class="bg-green-500/20 text-status-active"
              >
                Assistant
              </span>
              <span class="ml-auto flex items-center gap-1 text-[10px] text-[var(--accent)]">
                <span class="inline-block h-1.5 w-1.5 rounded-full animate-pulse bg-[var(--accent)]" />
                streaming
              </span>
            </div>
            <div
              class="whitespace-pre-wrap break-words text-[15px] leading-[1.5] text-[var(--text-secondary)] streaming-cursor"
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
