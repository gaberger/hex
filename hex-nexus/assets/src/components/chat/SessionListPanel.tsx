import { Component, For, Show, createSignal } from 'solid-js';
import {
  sessions,
  activeSessionId,
  loading,
  switchSession,
  createSession,
  deleteSession,
  loadSessions,
} from '../../stores/session';
import type { Session } from '../../stores/session';

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

const statusColors: Record<Session['status'], string> = {
  active: 'bg-emerald-500/20 text-emerald-400',
  paused: 'bg-amber-500/20 text-amber-400',
  completed: 'bg-gray-500/20 text-gray-400',
};

const SkeletonRow: Component = () => (
  <div class="flex items-center gap-2 px-3 py-2.5 animate-pulse">
    <div class="flex-1 space-y-1.5">
      <div class="h-3 w-28 rounded bg-gray-700/60" />
      <div class="h-2.5 w-16 rounded bg-gray-700/40" />
    </div>
  </div>
);

const SessionListPanel: Component = () => {
  const [confirmDeleteId, setConfirmDeleteId] = createSignal<string | null>(null);
  const [forking, setForking] = createSignal(false);

  const handleDelete = (id: string) => {
    if (confirmDeleteId() === id) {
      deleteSession(id);
      setConfirmDeleteId(null);
    } else {
      setConfirmDeleteId(id);
      // Auto-dismiss confirmation after 3 seconds
      setTimeout(() => setConfirmDeleteId((prev) => (prev === id ? null : prev)), 3000);
    }
  };

  const handleFork = async (id: string) => {
    setForking(true);
    try {
      const res = await fetch(`/api/sessions/${id}/fork`, { method: 'POST' });
      if (!res.ok) throw new Error(`Fork failed: ${res.statusText}`);
      await loadSessions();
    } catch (e: any) {
      console.error('[session] fork failed:', e.message);
    } finally {
      setForking(false);
    }
  };

  return (
    <div class="w-64 h-full flex flex-col border-r border-gray-700/50 bg-gray-900/50">
      {/* Header */}
      <div class="flex items-center justify-between px-3 py-2.5 border-b border-gray-700/50">
        <span class="text-xs font-semibold uppercase tracking-wide text-gray-400">Sessions</span>
        <button
          class="flex items-center gap-1 rounded px-2 py-1 text-xs font-medium text-cyan-400 hover:bg-cyan-500/10 transition-colors"
          onClick={() => createSession()}
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4" />
          </svg>
          New
        </button>
      </div>

      {/* Session list */}
      <div class="flex-1 overflow-y-auto">
        {/* Loading skeleton */}
        <Show when={loading()}>
          <SkeletonRow />
          <SkeletonRow />
          <SkeletonRow />
        </Show>

        {/* Empty state */}
        <Show when={!loading() && sessions().length === 0}>
          <div class="px-4 py-8 text-center text-xs text-gray-500 leading-relaxed">
            No sessions yet — start chatting to create one
          </div>
        </Show>

        {/* Session rows */}
        <Show when={!loading()}>
          <For each={sessions()}>
            {(session) => {
              const isActive = () => session.id === activeSessionId();
              return (
                <div
                  class={`group flex items-center gap-2 px-3 py-2.5 cursor-pointer transition-colors hover:bg-gray-800/70 ${
                    isActive() ? 'border-l-2 border-cyan-500 bg-gray-800/50' : 'border-l-2 border-transparent'
                  }`}
                  onClick={() => switchSession(session.id)}
                >
                  {/* Session info */}
                  <div class="flex-1 min-w-0">
                    <div class="flex items-center gap-1.5">
                      <span class="text-sm text-gray-200 truncate">{session.name}</span>
                      <span
                        class={`shrink-0 rounded-full px-1.5 py-0.5 text-[9px] font-semibold uppercase leading-none ${
                          statusColors[session.status]
                        }`}
                      >
                        {session.status}
                      </span>
                    </div>
                    <div class="flex items-center gap-2 mt-0.5">
                      <span class="text-[11px] text-gray-500">{relativeTime(session.createdAt)}</span>
                      <span class="text-[11px] text-gray-400">
                        {session.messageCount} msg{session.messageCount !== 1 ? 's' : ''}
                      </span>
                    </div>
                  </div>

                  {/* Action buttons */}
                  <div class="flex items-center gap-0.5 shrink-0 opacity-0 group-hover:opacity-100 transition-opacity">
                    {/* Fork button (active session only) */}
                    <Show when={isActive()}>
                      <button
                        class="p-1 rounded text-gray-500 hover:text-cyan-400 hover:bg-gray-700/50 transition-colors"
                        title="Fork session"
                        disabled={forking()}
                        onClick={(e) => {
                          e.stopPropagation();
                          handleFork(session.id);
                        }}
                      >
                        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                          <path stroke-linecap="round" stroke-linejoin="round" d="M7 7V3m10 4V3M5 21h4a2 2 0 002-2v-4a2 2 0 00-2-2H5a2 2 0 00-2 2v4a2 2 0 002 2zm10 0h4a2 2 0 002-2v-4a2 2 0 00-2-2h-4a2 2 0 00-2 2v4a2 2 0 002 2zM9 7h6a2 2 0 012 2v2H7V9a2 2 0 012-2z" />
                        </svg>
                      </button>
                    </Show>

                    {/* Delete button */}
                    <button
                      class={`p-1 rounded transition-colors ${
                        confirmDeleteId() === session.id
                          ? 'text-red-400 bg-red-500/10'
                          : 'text-gray-500 hover:text-red-400 hover:bg-gray-700/50'
                      }`}
                      title={confirmDeleteId() === session.id ? 'Click again to confirm' : 'Delete session'}
                      onClick={(e) => {
                        e.stopPropagation();
                        handleDelete(session.id);
                      }}
                    >
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path
                          stroke-linecap="round"
                          stroke-linejoin="round"
                          d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                        />
                      </svg>
                    </button>
                  </div>
                </div>
              );
            }}
          </For>
        </Show>
      </div>
    </div>
  );
};

export default SessionListPanel;
