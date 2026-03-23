/**
 * InboxPane.tsx — Agent inbox notifications (ADR-060).
 *
 * Displays priority-coded notifications for a given agent,
 * with acknowledge action per notification.
 */
import { Component, For, Show, createSignal, createResource } from "solid-js";
import { restClient } from "../../services/rest-client";

interface Notification {
  id: string;
  priority: number;
  source: string;
  message: string;
  timestamp: string;
  acknowledged?: boolean;
}

function priorityLabel(priority: number): string {
  if (priority >= 2) return "critical";
  if (priority >= 1) return "warning";
  return "info";
}

function priorityBadgeClass(priority: number): string {
  if (priority >= 2) return "bg-red-900/40 text-red-400";
  if (priority >= 1) return "bg-yellow-900/40 text-yellow-400";
  return "bg-gray-800 text-gray-400";
}

function priorityBorderClass(priority: number): string {
  if (priority >= 2) return "border-red-800/40";
  if (priority >= 1) return "border-yellow-800/40";
  return "border-gray-800";
}

function relativeTime(timestamp: string): string {
  const diff = Date.now() - new Date(timestamp).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
}

const InboxPane: Component<{ agentId: string }> = (props) => {
  const [ackingId, setAckingId] = createSignal<string | null>(null);

  const [notifications, { refetch }] = createResource(
    () => props.agentId,
    async (agentId) => {
      if (!agentId) return [];
      return restClient.get<Notification[]>(`/api/hexflo/inbox/${agentId}`);
    },
  );

  async function handleAck(id: string) {
    setAckingId(id);
    try {
      await restClient.patch(`/api/hexflo/inbox/${id}/ack`);
      refetch();
    } catch (err) {
      console.error("Failed to acknowledge notification:", err);
    } finally {
      setAckingId(null);
    }
  }

  return (
    <div class="flex flex-col gap-4 p-4">
      {/* Header */}
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-gray-200">Inbox</h2>
        <button
          class="rounded border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors disabled:opacity-50"
          onClick={() => refetch()}
          disabled={notifications.loading}
        >
          <Show when={notifications.loading} fallback="Refresh">
            <span class="animate-pulse">Loading...</span>
          </Show>
        </button>
      </div>

      {/* Loading state */}
      <Show when={notifications.loading && !notifications()}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg
            class="h-8 w-8 animate-spin text-cyan-400"
            viewBox="0 0 24 24"
            fill="none"
          >
            <circle
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              stroke-width="3"
              stroke-dasharray="31.4 31.4"
              stroke-linecap="round"
            />
          </svg>
          <span class="mt-3 text-xs">Loading notifications...</span>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!notifications.loading && notifications() && notifications()!.length === 0}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg
            class="h-10 w-10 text-gray-700"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <path d="M21.75 6.75v10.5a2.25 2.25 0 01-2.25 2.25h-15a2.25 2.25 0 01-2.25-2.25V6.75m19.5 0A2.25 2.25 0 0019.5 4.5h-15a2.25 2.25 0 00-2.25 2.25m19.5 0v.243a2.25 2.25 0 01-1.07 1.916l-7.5 4.615a2.25 2.25 0 01-2.36 0L3.32 8.91a2.25 2.25 0 01-1.07-1.916V6.75" />
          </svg>
          <p class="mt-3 text-xs">No notifications</p>
        </div>
      </Show>

      {/* Notification list */}
      <Show when={notifications() && notifications()!.length > 0}>
        <div class="flex flex-col gap-2">
          <For each={notifications()}>
            {(notif) => (
              <div
                class={`rounded-lg border ${priorityBorderClass(notif.priority)} bg-gray-950 px-3 py-2`}
              >
                <div class="flex items-center gap-2">
                  <span
                    class={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${priorityBadgeClass(notif.priority)}`}
                  >
                    {priorityLabel(notif.priority)}
                  </span>
                  <span class="truncate text-xs font-medium text-gray-300">
                    {notif.source}
                  </span>
                  <span class="ml-auto shrink-0 text-[10px] text-gray-500">
                    {relativeTime(notif.timestamp)}
                  </span>
                </div>
                <p class="mt-1 text-xs text-gray-400">{notif.message}</p>
                <div class="mt-2 flex justify-end">
                  <button
                    class="rounded bg-gray-800 px-2.5 py-1 text-[10px] font-medium text-gray-300 hover:bg-gray-700 hover:text-white transition-colors disabled:opacity-50"
                    onClick={() => handleAck(notif.id)}
                    disabled={ackingId() === notif.id}
                  >
                    {ackingId() === notif.id ? "Acking..." : "Ack"}
                  </button>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default InboxPane;
