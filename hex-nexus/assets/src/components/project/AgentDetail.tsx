/**
 * AgentDetail.tsx — Detail view for a single agent.
 *
 * Shows agent metadata, assigned tasks, worktree info, and recent commits.
 * Data from SpacetimeDB subscriptions + REST for git log.
 */
import { Component, For, Show, createMemo, createResource } from "solid-js";
import {
  swarmAgents,
  swarmTasks,
  swarms,
  registryAgents,
  agentHeartbeats,
} from "../../stores/connection";
import { navigate, route } from "../../stores/router";
import { restClient } from "../../services/rest-client";

function relativeTime(timestamp: string | undefined): string {
  if (!timestamp) return "--";
  const diff = Date.now() - new Date(timestamp).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
}

function statusBadgeClass(status: string): string {
  switch (status) {
    case "active":
    case "online":
      return "bg-green-900/40 text-green-400";
    case "stale":
    case "warning":
      return "bg-yellow-900/40 text-yellow-400";
    case "dead":
    case "error":
    case "failed":
      return "bg-red-900/40 text-red-400";
    default:
      return "bg-gray-800 text-gray-400";
  }
}

function taskStatusClass(status: string): string {
  switch (status) {
    case "in_progress":
      return "bg-cyan-900/40 text-cyan-400";
    case "completed":
      return "bg-green-900/40 text-green-400";
    case "failed":
      return "bg-red-900/40 text-red-400";
    default:
      return "bg-gray-800 text-gray-400";
  }
}

const AgentDetail: Component = () => {
  const projectId = () => (route() as any).projectId ?? "";
  const agentId = () => (route() as any).agentId ?? "";

  // Find agent in swarm agents or registry agents
  const agent = createMemo(() => {
    const id = agentId();
    return (
      swarmAgents().find(
        (a: any) => (a.id ?? a.agent_id ?? "") === id,
      ) ??
      registryAgents().find(
        (a: any) => (a.id ?? a.agent_id ?? "") === id,
      )
    );
  });

  const heartbeat = createMemo(() =>
    agentHeartbeats().find(
      (h: any) => (h.agent_id ?? "") === agentId(),
    ),
  );

  // Tasks assigned to this agent
  const assignedTasks = createMemo(() =>
    swarmTasks().filter(
      (t: any) => (t.assigned_to ?? t.agent_id ?? "") === agentId(),
    ),
  );

  // Resolve swarm name for a task
  function swarmName(swarmId: string): string {
    const s = swarms().find(
      (sw: any) => (sw.id ?? sw.swarm_id ?? "") === swarmId,
    );
    return s?.name ?? swarmId;
  }

  // Fetch recent commits via REST (filesystem op)
  const [commits] = createResource(
    () => projectId(),
    async (pid) => {
      if (!pid) return [];
      try {
        const data = await restClient.get<any>(
          `/api/${encodeURIComponent(pid)}/git/log?limit=10`,
        );
        return Array.isArray(data) ? data : data?.commits ?? [];
      } catch {
        return [];
      }
    },
  );

  function handleBack() {
    navigate({ page: "project-agents", projectId: projectId() });
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      {/* Back button */}
      <button
        class="mb-4 flex items-center gap-1 text-xs text-gray-400 transition-colors hover:text-gray-200"
        onClick={handleBack}
      >
        <span>&larr;</span>
        <span>Back to Agents</span>
      </button>

      <Show
        when={agent()}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-400">
              Agent not found: {agentId()}
            </p>
          </div>
        }
      >
        {(a) => {
          const name = () => a().name ?? a().agent_name ?? "unnamed";
          const role = () => a().role ?? a().agent_role ?? "";
          const model = () => a().model ?? a().model_name ?? "";
          const status = () => a().status ?? a().state ?? "unknown";
          const worktree = () => a().worktree ?? a().worktree_path ?? "";
          const branch = () => a().branch ?? a().worktree_branch ?? "";

          return (
            <>
              {/* Header */}
              <div class="mb-6 flex items-center gap-3">
                <div class="flex-1">
                  <div class="flex items-center gap-2">
                    <h2 class="text-lg font-semibold text-gray-100">
                      {name()}
                    </h2>
                    <Show when={role()}>
                      <span class="rounded-full bg-gray-800 px-2 py-0.5 text-[10px] font-semibold uppercase text-gray-300">
                        {role()}
                      </span>
                    </Show>
                    <span
                      class={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${statusBadgeClass(status())}`}
                    >
                      {status()}
                    </span>
                  </div>
                  <div class="mt-1 flex items-center gap-4 text-[10px] text-gray-400">
                    <Show when={model()}>
                      <span>Model: {model()}</span>
                    </Show>
                    <span>
                      Heartbeat:{" "}
                      {relativeTime(
                        heartbeat()?.timestamp ?? heartbeat()?.last_seen,
                      )}
                    </span>
                  </div>
                </div>
              </div>

              {/* Assigned Tasks */}
              <SectionHeader
                title="Assigned Tasks"
                count={assignedTasks().length}
              />
              <Show
                when={assignedTasks().length > 0}
                fallback={
                  <p class="mb-6 text-xs text-gray-500">
                    No tasks assigned to this agent
                  </p>
                }
              >
                <div class="mb-6 space-y-1.5">
                  <For each={assignedTasks()}>
                    {(task) => {
                      const sid = task.swarm_id ?? task.swarmId ?? "";
                      return (
                        <div class="flex items-center justify-between rounded-lg border border-gray-800 bg-gray-900/50 px-3 py-2 text-xs">
                          <div class="flex-1 truncate">
                            <span class="text-gray-100">
                              {task.title ?? "Untitled"}
                            </span>
                            <Show when={sid}>
                              <span class="ml-2 text-[10px] text-gray-500">
                                {swarmName(sid)}
                              </span>
                            </Show>
                          </div>
                          <span
                            class={`ml-2 shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${taskStatusClass(task.status ?? "pending")}`}
                          >
                            {task.status ?? "pending"}
                          </span>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </Show>

              {/* Worktree */}
              <SectionHeader title="Worktree" count={worktree() ? 1 : 0} />
              <Show
                when={worktree()}
                fallback={
                  <p class="mb-6 text-xs text-gray-500">
                    No worktree assigned
                  </p>
                }
              >
                <div class="mb-6 rounded-lg border border-gray-800 bg-gray-900/50 px-3 py-2">
                  <p class="truncate font-mono text-xs text-gray-300">
                    {worktree()}
                  </p>
                  <Show when={branch()}>
                    <p class="mt-1 text-[10px] text-gray-500">
                      Branch: {branch()}
                    </p>
                  </Show>
                </div>
              </Show>

              {/* Recent Commits */}
              <SectionHeader
                title="Recent Commits"
                count={commits()?.length ?? 0}
              />
              <Show
                when={!commits.loading}
                fallback={
                  <p class="text-xs text-gray-500">Loading commits...</p>
                }
              >
                <Show
                  when={(commits() ?? []).length > 0}
                  fallback={
                    <p class="text-xs text-gray-500">No recent commits</p>
                  }
                >
                  <div class="space-y-1">
                    <For each={commits() ?? []}>
                      {(commit: any) => (
                        <div class="flex items-center gap-2 rounded-lg border border-gray-800 bg-gray-900/50 px-3 py-2 text-xs">
                          <span class="shrink-0 font-mono text-cyan-400">
                            {(commit.hash ?? commit.sha ?? "").slice(0, 7)}
                          </span>
                          <span class="flex-1 truncate text-gray-300">
                            {commit.message ?? commit.subject ?? ""}
                          </span>
                          <Show when={commit.date ?? commit.timestamp}>
                            <span class="shrink-0 text-[10px] text-gray-500">
                              {relativeTime(commit.date ?? commit.timestamp)}
                            </span>
                          </Show>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </Show>
            </>
          );
        }}
      </Show>
    </div>
  );
};

const SectionHeader: Component<{ title: string; count: number }> = (
  props,
) => (
  <div class="mb-2 flex items-center gap-2">
    <h4 class="text-[11px] font-semibold uppercase tracking-wider text-gray-400">
      {props.title}
    </h4>
    <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-400">
      {props.count}
    </span>
  </div>
);

export default AgentDetail;
