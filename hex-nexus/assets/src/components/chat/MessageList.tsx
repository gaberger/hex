import { Component, For, Show, createEffect, createSignal, type Accessor } from 'solid-js';
import Message, { type ChatMessage } from './Message';

interface MessageListProps {
  messages: Accessor<ChatMessage[]>;
  streamingText: Accessor<string>;
}

const MessageList: Component<MessageListProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  const [autoScroll, setAutoScroll] = createSignal(true);

  // Detect manual scroll-up to pause auto-scroll
  const handleScroll = () => {
    if (!containerRef) return;
    const { scrollTop, scrollHeight, clientHeight } = containerRef;
    // If user is within 60px of bottom, re-enable auto-scroll
    setAutoScroll(scrollHeight - scrollTop - clientHeight < 60);
  };

  // Auto-scroll when messages change or streaming text updates
  createEffect(() => {
    // Track reactive dependencies
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
      <Show when={props.messages().length === 0 && !props.streamingText()}>
        <div class="flex h-full items-center justify-center">
          <div class="text-center text-gray-300">
            <div class="text-2xl mb-2">hex</div>
            <p class="text-sm">No messages yet. Start a conversation below.</p>
          </div>
        </div>
      </Show>

      <div class="divide-y divide-gray-800/50">
        <For each={props.messages()}>
          {(msg) => <Message message={msg} />}
        </For>
      </div>

      {/* Streaming message (in-progress) */}
      <Show when={props.streamingText()}>
        <div class="px-4 py-3 bg-gray-800/50">
          <div class="flex items-center gap-2 mb-1">
            <span class="inline-block rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide bg-green-600/30 text-green-300">
              Assistant
            </span>
            <span class="ml-auto flex items-center gap-1 text-[10px] text-cyan-400">
              <span class="inline-block h-1.5 w-1.5 rounded-full bg-cyan-400 animate-pulse" />
              streaming
            </span>
          </div>
          <div class="whitespace-pre-wrap break-words text-sm text-gray-300 leading-relaxed streaming-cursor">
            {props.streamingText()}
          </div>
        </div>
      </Show>
    </div>
  );
};

export default MessageList;
