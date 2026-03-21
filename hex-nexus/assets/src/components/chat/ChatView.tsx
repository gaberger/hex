import { Component } from 'solid-js';
import MessageList from './MessageList';
import { chatMessages, streamingText } from '../../stores/chat';

/**
 * ChatView — main chat container for the center pane.
 *
 * WebSocket lifecycle is managed by the shared chat store.
 * Connection status is shown in the BottomBar.
 */
const ChatView: Component = () => {
  return (
    <div class="flex h-full flex-col bg-gray-950">
      <MessageList messages={chatMessages} streamingText={streamingText} />
    </div>
  );
};

export default ChatView;
