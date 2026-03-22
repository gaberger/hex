/**
 * AgentCard.tsx — Compact agent status card.
 *
 * Shows agent name, status, project, current task, and heartbeat age.
 * Click opens AgentLog pane. Right section has kill button.
 */
import { Component, Show, createMemo } from "solid-js";
import { agentHeartbeats, swarmTasks } from "../../stores/connection";
import { openPane } from "../../stores/panes";
import { restClient } from "../../services/rest-client";

export interface AgentInfo {
  id: string;
  name: string;
  status: string;
  project?: string;
  agentType?: string;
}

function statusColor(status: string): string {
  if (status === "active" || status === "online") return "bg-green-500";
  if (status === "stale" || status === "warning") return "bg-yellow-500";
  if (status === "dead" || status === "offline" || status === "error") return "bg-red-500";
  return "bg-gray-500";
}

function statusBorderColor(status: string): string {
  if (status === "active" || status === "online") return "border-green-800/50";
  if (status === "stale" || status === "warning") return "border-yellow-800/50";
  if (status === "dead" || status === "offline") return "border-red-800/50";
  return "border-gray-800";
}

const AgentCard: Component<{ agent: AgentInfo }> = (props) => {
  const heartbeat = createMemo(() =>
    agentHeartbeats().find((h: any) => (h.agent_id ?? "") === props.agent.id)
  );

  const currentTask = createMemo(() =>
    swarmTasks().find(
      (t: any) =>
        (t.assigned_to ?? t.agent_id ?? "") === props.agent.id &&
        (t.status === "in_progress" || t.status === "active")
    )
  );

  const heartbeatAge = () => {
    const hb = heartbeat();
    if (!hb?.timestamp) return null;
    const diff = Date.now() - new Date(hb.timestamp).getTime();
    if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    return `${Math.floor(diff / 3_600_000)}h ago`;
  };

  function handleClick() {
    openPane("agent-log", props.agent.name, { agentId: props.agent.id });
  }

  async function handleKill(e: MouseEvent) {
    e.stopPropagation();
    try {
      await restClient.post(`/api/agents/${encodeURIComponent(props.agent.id)}/kill`);
    } catch {
      // Agent will disappear from SpacetimeDB subscription when it dies
    }
  }

  return (
    <div
      class={`group flex items-center gap-3 rounded-lg border ${statusBorderColor(props.agent.status)} bg-gray-900/60 px-3 py-2.5 transition-all hover:bg-gray-900 cursor-pointer`}
      onClick={handleClick}
    >
      {/* Status dot */}
      <span class={`h-2.5 w-2.5 shrink-0 rounded-full ${statusColor(props.agent.status)}`} />

      {/* Info */}
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <span class="truncate text-xs font-semibold text-gray-100">
            {props.agent.name}
          </span>
          <Show when={props.agent.agentType}>
            <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[9px] uppercase text-gray-300">
              {props.agent.agentType}
            </span>
          </Show>
        </div>

        <Show when={currentTask()}>
          <p class="mt-0.5 truncate text-[10px] text-gray-300">
            {currentTask()!.title ?? "working..."}
          </p>
        </Show>
      </div>

      {/* Right side — heartbeat + kill */}
      <div class="flex shrink-0 items-center gap-2">
        <Show when={heartbeatAge()}>
          <span class="text-[10px] text-gray-300">{heartbeatAge()}</span>
        </Show>

        {/* Kill button — hidden until hover */}
        <button
          class="hidden rounded p-1 text-gray-300 hover:bg-red-900/30 hover:text-red-400 group-hover:block transition-colors"
          onClick={handleKill}
          title="Kill agent"
        >
          <svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default AgentCard;
