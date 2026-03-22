/**
 * AgentList.tsx — Project-scoped agent list.
 *
 * Shows all agents working on a given project, sourced from swarm agents
 * and the global agent registry. Data from SpacetimeDB subscriptions.
 */
import { Component, For, Show, createMemo } from "solid-js";
import {
  swarms,
  swarmAgents,
  swarmTasks,
  registryAgents,
  agentHeartbeats,
} from "../../stores/connection";
import { navigate, route } from "../../stores/router";
import { entityBelongsToProject } from "../../utils/project-match";

function relativeTime(timestamp: string | undefined): string {
  if (!timestamp) return "--";
  const diff = Date.now() - new Date(timestamp).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
}

function statusDotClass(status: string): string {
  switch (status) {
    case "active":
    case "online":
      return "bg-green-400";
    case "stale":
    case "warning":
      return "bg-yellow-400";
    case "dead":
    case "error":
    case "failed":
      return "bg-red-400";
    default:
      return "bg-gray-500";
  }
}

const AgentList: Component = () => {
  const projectId = () => (route() as any).projectId ?? "";

  // Find swarms belonging to this project
  const projectSwarms = createMemo(() =>
    swarms().filter(
      (s: any) => entityBelongsToProject(s, projectId()),
    ),
  );

  const projectSwarmIds = createMemo(() =>
    new Set(projectSwarms().map((s: any) => s.id ?? s.swarm_id ?? "")),
  );

  // Agents from swarms belonging to this project
  const projectSwarmAgents = createMemo(() =>
    swarmAgents().filter((a: any) =>
      projectSwarmIds().has(a.swarm_id ?? a.swarmId ?? ""),
    ),
  );

  // Global registry agents tied to this project (if any)
  const projectRegistryAgents = createMemo(() =>
    registryAgents().filter(
      (a: any) => entityBelongsToProject(a, projectId()),
    ),
  );

  // Merge and deduplicate by agent id
  const allAgents = createMemo(() => {
    const seen = new Set<string>();
    const result: any[] = [];
    for (const a of [...projectSwarmAgents(), ...projectRegistryAgents()]) {
      const id = a.id ?? a.agent_id ?? "";
      if (id && !seen.has(id)) {
        seen.add(id);
        result.push(a);
      }
    }
    return result;
  });

  // Count tasks assigned to a given agent
  function taskCount(agentId: string): number {
    return swarmTasks().filter(
      (t: any) => (t.assigned_to ?? t.agent_id ?? "") === agentId,
    ).length;
  }

  // Get heartbeat for an agent
  function getHeartbeat(agentId: string): any {
    return agentHeartbeats().find(
      (h: any) => (h.agent_id ?? "") === agentId,
    );
  }

  function handleAgentClick(agentId: string) {
    navigate({
      page: "project-agent-detail",
      projectId: projectId(),
      agentId,
    });
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      <div class="mb-4 flex items-center justify-between">
        <h3 class="text-sm font-semibold text-gray-100">Agents</h3>
        <span class="text-[10px] text-gray-400">
          {allAgents().length} agent{allAgents().length !== 1 ? "s" : ""}
        </span>
      </div>

      <Show
        when={allAgents().length > 0}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-400">No agents active for this project</p>
          </div>
        }
      >
        <div class="grid gap-3 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
          <For each={allAgents()}>
            {(agent) => {
              const id = agent.id ?? agent.agent_id ?? "";
              const name = agent.name ?? agent.agent_name ?? "unnamed";
              const role = agent.role ?? agent.agent_role ?? "";
              const status = agent.status ?? agent.state ?? "unknown";
              const worktree = agent.worktree ?? agent.worktree_path ?? "";
              const hb = () => getHeartbeat(id);

              return (
                <button
                  class="flex flex-col gap-2 rounded-lg border border-gray-800 bg-gray-900/50 p-3 text-left transition-colors hover:border-gray-600"
                  onClick={() => handleAgentClick(id)}
                >
                  {/* Top row: name + status */}
                  <div class="flex items-center gap-2">
                    <span class={`h-2.5 w-2.5 shrink-0 rounded-full ${statusDotClass(status)}`} />
                    <span class="truncate text-sm font-medium text-gray-100">
                      {name}
                    </span>
                    <Show when={role}>
                      <span class="ml-auto shrink-0 rounded-full bg-gray-800 px-2 py-0.5 text-[10px] font-semibold uppercase text-gray-300">
                        {role}
                      </span>
                    </Show>
                  </div>

                  {/* Stats row */}
                  <div class="flex items-center gap-4 text-[10px] text-gray-400">
                    <span>{taskCount(id)} task{taskCount(id) !== 1 ? "s" : ""}</span>
                    <span>
                      {relativeTime(hb()?.timestamp ?? hb()?.last_seen)}
                    </span>
                  </div>

                  {/* Worktree */}
                  <Show when={worktree}>
                    <p class="truncate font-mono text-[10px] text-gray-500">
                      {worktree}
                    </p>
                  </Show>
                </button>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default AgentList;
