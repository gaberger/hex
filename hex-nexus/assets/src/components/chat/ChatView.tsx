import { Component, createMemo } from 'solid-js';
import MessageList from './MessageList';
import { chatMessages, streamingText } from '../../stores/chat';
import { route } from '../../stores/router';
import { projects } from '../../stores/projects';
import { gitStatus } from '../../stores/git';

/**
 * ChatView — main chat container for the center pane.
 *
 * Matches Pencil "Chat Center" design: context pills + message area.
 * WebSocket lifecycle is managed by the shared chat store.
 */
const ChatView: Component = () => {
  const projectId = createMemo(() => (route() as any).projectId ?? '');
  const project = createMemo(() => projects().find((p) => p.id === projectId()));
  const branch = createMemo(() => gitStatus()?.branch ?? '');

  return (
    <div class="flex h-full flex-col" style={{ background: 'var(--bg-base)' }}>
      {/* Context pills (Pencil: ctxPills) */}
      <div
        class="flex items-center flex-wrap"
        style={{
          padding: '8px 12px',
          gap: '8px',
          background: 'var(--bg-surface)',
          "border-radius": '8px',
          margin: '12px 24px 0 24px',
        }}
      >
        <span class="text-[12px]" style={{ color: 'var(--text-faint)' }}>Context:</span>
        <span
          class="rounded px-2 py-0.5 text-[11px] font-medium"
          style={{ background: 'rgba(30,58,95,0.3)', color: '#60a5fa' }}
        >
          {project()?.name ?? projectId()}
        </span>
        {branch() && (
          <span
            class="rounded px-2 py-0.5 text-[11px] font-medium"
            style={{ background: 'var(--bg-elevated)', color: 'var(--text-muted)' }}
          >
            {branch()}
          </span>
        )}
      </div>

      <MessageList messages={chatMessages} streamingText={streamingText} />
    </div>
  );
};

export default ChatView;
