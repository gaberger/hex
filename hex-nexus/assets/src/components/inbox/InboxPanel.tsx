/**
 * InboxPanel.tsx — Agent notification inbox panel.
 *
 * Displays notifications from SpacetimeDB agent_inbox table.
 * Priority-2 (critical) notifications are highlighted.
 * Acknowledge via SpacetimeDB reducer (ADR-060, ADR-2603231309).
 */
import { Component, For, Show, createMemo } from "solid-js";
import { agentInbox, getHexfloConn } from "../../stores/connection";
import { addToast } from "../../stores/toast";

function priorityClass(priority: number): string {
  switch (priority) {
    case 2:
      return "border-red-800 bg-red-900/20";
    case 1:
      return "border-yellow-800 bg-yellow-900/20";
    default:
      return "border-gray-800 bg-gray-900/50";
  }
}

function priorityLabel(priority: number): string {
  switch (priority) {
    case 2:
      return "CRITICAL";
    case 1:
      return "WARNING";
    default:
      return "INFO";
  }
}

function priorityTextClass(priority: number): string {
  switch (priority) {
    case 2:
      return "text-red-400";
    case 1:
      return "text-yellow-400";
    default:
      return "text-gray-400";
  }
}

function relativeTime(timestamp: string | undefined): string {
  if (!timestamp) return "--";
  const diff = Date.now() - new Date(timestamp).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
}

const InboxPanel: Component = () => {
  const notifications = createMemo(() => {
    const all = agentInbox() ?? [];
    // Sort: unacknowledged first, then by priority desc, then by created_at desc
    return [...all]
      .filter((n: any) => !n.expired_at) // hide expired
      .sort((a: any, b: any) => {
        const aAcked = !!a.acknowledged_at;
        const bAcked = !!b.acknowledged_at;
        if (aAcked !== bAcked) return aAcked ? 1 : -1;
        if ((a.priority ?? 0) !== (b.priority ?? 0)) return (b.priority ?? 0) - (a.priority ?? 0);
        return (b.created_at ?? "").localeCompare(a.created_at ?? "");
      });
  });

  const unreadCount = createMemo(() =>
    notifications().filter((n: any) => !n.acknowledged_at).length,
  );

  function handleAck(notificationId: number, agentId: string) {
    const conn = getHexfloConn();
    if (!conn) {
      addToast("error", "SpacetimeDB not connected");
      return;
    }
    const timestamp = new Date().toISOString();
    conn.reducers.acknowledgeNotification(notificationId, agentId, timestamp);
    addToast("success", "Notification acknowledged");
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      {/* Header */}
      <div class="mb-4 flex items-center justify-between">
        <div class="flex items-center gap-2">
          <h2 class="text-sm font-semibold text-gray-200">Inbox</h2>
          <Show when={unreadCount() > 0}>
            <span class="rounded-full bg-red-600 px-2 py-0.5 text-[10px] font-bold text-white">
              {unreadCount()}
            </span>
          </Show>
        </div>
        <span class="text-[10px] text-gray-500">
          {notifications().length} notification{notifications().length !== 1 ? "s" : ""}
        </span>
      </div>

      {/* Empty state */}
      <Show when={notifications().length === 0}>
        <div class="flex flex-1 flex-col items-center justify-center text-gray-500">
          <svg class="h-10 w-10 text-gray-700" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M14.857 17.082a23.848 23.848 0 005.454-1.31A8.967 8.967 0 0118 9.75v-.7V9A6 6 0 006 9v.75a8.967 8.967 0 01-2.312 6.022c1.733.64 3.56 1.085 5.455 1.31m5.714 0a24.255 24.255 0 01-5.714 0m5.714 0a3 3 0 11-5.714 0" />
          </svg>
          <p class="mt-3 text-xs">No notifications</p>
        </div>
      </Show>

      {/* Notification list */}
      <Show when={notifications().length > 0}>
        <div class="space-y-2">
          <For each={notifications()}>
            {(notif: any) => {
              const isAcked = !!notif.acknowledged_at;
              const priority = notif.priority ?? 0;

              return (
                <div
                  class={`rounded-lg border px-3 py-2.5 transition-colors ${priorityClass(priority)}`}
                  classList={{ "opacity-50": isAcked }}
                >
                  <div class="flex items-start gap-2">
                    {/* Priority badge */}
                    <span
                      class={`shrink-0 rounded px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider ${priorityTextClass(priority)}`}
                    >
                      {priorityLabel(priority)}
                    </span>

                    {/* Content */}
                    <div class="min-w-0 flex-1">
                      <p class="text-xs text-gray-200">{notif.payload ?? notif.message ?? ""}</p>
                      <div class="mt-1 flex items-center gap-3 text-[10px] text-gray-500">
                        <span>{notif.kind ?? "notification"}</span>
                        <span>{relativeTime(notif.created_at)}</span>
                        <Show when={notif.agent_id}>
                          <span class="font-mono">{(notif.agent_id ?? "").slice(0, 8)}</span>
                        </Show>
                      </div>
                    </div>

                    {/* Ack button */}
                    <Show when={!isAcked}>
                      <button
                        class="shrink-0 rounded border border-gray-700 px-2 py-1 text-[10px] text-gray-400 hover:border-green-600 hover:text-green-400 transition-colors"
                        onClick={() => handleAck(notif.id, notif.agent_id)}
                      >
                        Ack
                      </button>
                    </Show>
                    <Show when={isAcked}>
                      <span class="shrink-0 text-[10px] text-green-600">Acked</span>
                    </Show>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default InboxPanel;
