import { Component, createMemo, createSignal } from 'solid-js';
import MessageList from './MessageList';
import SessionListPanel from './SessionListPanel';
import ModelSelector from './ModelSelector';
import { chatMessages, streamingText } from '../../stores/chat';
import { route } from '../../stores/router';
import { projects } from '../../stores/projects';
import { gitStatus } from '../../stores/git';

/**
 * ChatView — main chat container for the center pane.
 *
 * Three-part layout: optional session sidebar | context bar + messages.
 * WebSocket lifecycle is managed by the shared chat store.
 */
const ChatView: Component = () => {
  const projectId = createMemo(() => (route() as any).projectId ?? '');
  const project = createMemo(() => projects().find((p) => p.id === projectId()));
  const branch = createMemo(() => gitStatus()?.branch ?? '');
  const [showSessions, setShowSessions] = createSignal(true);

  return (
    <div class="flex h-full">
      {/* Session sidebar (toggleable) */}
      {showSessions() && <SessionListPanel />}

      {/* Main chat area */}
      <div class="flex flex-1 flex-col min-w-0 bg-gray-950">
        {/* Context bar: project pill + branch pill + model selector + session toggle */}
        <div class="flex items-center gap-2 px-4 py-2 border-b border-gray-800">
          {/* Session toggle */}
          <button
            class="p-1 rounded text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors"
            title={showSessions() ? 'Hide sessions' : 'Show sessions'}
            onClick={() => setShowSessions(!showSessions())}
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M4 6h16M4 12h16M4 18h16" />
            </svg>
          </button>

          <span class="text-xs text-gray-500">Context:</span>
          <span class="rounded bg-blue-500/10 px-2 py-0.5 text-[11px] font-medium text-blue-400">
            {(project()?.name ?? projectId()) || 'No project'}
          </span>
          {branch() && (
            <span class="rounded bg-gray-800 px-2 py-0.5 text-[11px] font-medium text-gray-400">
              {branch()}
            </span>
          )}

          {/* Spacer */}
          <div class="flex-1" />

          {/* Model selector */}
          <ModelSelector />
        </div>

        <MessageList messages={chatMessages} streamingText={streamingText} />
      </div>
    </div>
  );
};

export default ChatView;
