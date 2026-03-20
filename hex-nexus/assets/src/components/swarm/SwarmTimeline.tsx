/**
 * SwarmTimeline.tsx — Chronological task events with timestamps.
 *
 * Shows a vertical timeline of task state changes: created, started,
 * completed, failed. Data derived from task timestamps in SpacetimeDB.
 */
import { Component, For, Show, createMemo } from "solid-js";

interface TimelineEvent {
  id: string;
  taskTitle: string;
  event: "created" | "started" | "completed" | "failed";
  timestamp: string;
  agentName?: string;
}

const EVENT_STYLES: Record<string, { dot: string; text: string; label: string }> = {
  created:   { dot: "bg-gray-500", text: "text-gray-300", label: "Created" },
  started:   { dot: "bg-cyan-500", text: "text-cyan-400", label: "Started" },
  completed: { dot: "bg-green-500", text: "text-green-400", label: "Completed" },
  failed:    { dot: "bg-red-500", text: "text-red-400", label: "Failed" },
};

/** Extract timeline events from task data. */
export function tasksToTimeline(tasks: any[]): TimelineEvent[] {
  const events: TimelineEvent[] = [];

  for (const t of tasks) {
    const title = t.title ?? "Untitled";
    const agent = t.assigned_to ?? t.agent_name ?? undefined;

    if (t.createdAt ?? t.created_at) {
      events.push({
        id: `${t.id}-created`,
        taskTitle: title,
        event: "created",
        timestamp: t.createdAt ?? t.created_at,
        agentName: agent,
      });
    }

    if (t.status === "in_progress" && (t.updatedAt ?? t.updated_at)) {
      events.push({
        id: `${t.id}-started`,
        taskTitle: title,
        event: "started",
        timestamp: t.updatedAt ?? t.updated_at,
        agentName: agent,
      });
    }

    if (t.status === "completed" && (t.completedAt ?? t.completed_at ?? t.updatedAt)) {
      events.push({
        id: `${t.id}-completed`,
        taskTitle: title,
        event: "completed",
        timestamp: t.completedAt ?? t.completed_at ?? t.updatedAt ?? t.updated_at,
        agentName: agent,
      });
    }

    if (t.status === "failed") {
      events.push({
        id: `${t.id}-failed`,
        taskTitle: title,
        event: "failed",
        timestamp: t.updatedAt ?? t.updated_at ?? t.createdAt ?? t.created_at,
        agentName: agent,
      });
    }
  }

  // Sort newest first
  events.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime());
  return events;
}

function formatTime(ts: string): string {
  try {
    const d = new Date(ts);
    return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  } catch {
    return ts;
  }
}

const SwarmTimeline: Component<{ events: TimelineEvent[] }> = (props) => {
  return (
    <div class="space-y-0">
      <Show
        when={props.events.length > 0}
        fallback={<p class="text-xs text-gray-300">No events yet</p>}
      >
        <For each={props.events}>
          {(event, i) => {
            const style = EVENT_STYLES[event.event] ?? EVENT_STYLES.created;
            return (
              <div class="flex gap-3">
                {/* Timeline spine */}
                <div class="flex flex-col items-center">
                  <div class={`h-2.5 w-2.5 shrink-0 rounded-full ${style.dot}`} />
                  <Show when={i() < props.events.length - 1}>
                    <div class="w-px flex-1 bg-gray-800" />
                  </Show>
                </div>

                {/* Content */}
                <div class="pb-4 min-w-0">
                  <div class="flex items-center gap-2">
                    <span class={`text-[10px] font-semibold uppercase ${style.text}`}>
                      {style.label}
                    </span>
                    <span class="text-[10px] text-gray-300">
                      {formatTime(event.timestamp)}
                    </span>
                  </div>
                  <p class="mt-0.5 truncate text-xs text-gray-300">
                    {event.taskTitle}
                  </p>
                  <Show when={event.agentName}>
                    <p class="text-[10px] text-gray-300">
                      by {event.agentName}
                    </p>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
      </Show>
    </div>
  );
};

export default SwarmTimeline;
