/**
 * SwarmDetail.tsx — Detail view for a single swarm.
 *
 * Shows swarm metadata, task list with status badges, agent roster,
 * and an overall progress bar. Data from SpacetimeDB subscriptions.
 */
import { Component, For, Show, createMemo } from "solid-js";
import {
  swarms,
  swarmTasks,
  swarmAgents,
  agentHeartbeats,
} from "../../stores/connection";
import { navigate, route } from "../../stores/router";

function relativeTime(timestamp: string | undefined): string {
  if (!timestamp) return "--";
  const diff = Date.now() - new Date(timestamp).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
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

function topologyBadgeClass(topology: string): string {
  switch (topology) {
    case "hierarchical":
      return "bg-purple-900/40 text-purple-400";
    case "mesh":
      return "bg-cyan-900/40 text-cyan-400";
    case "pipeline":
      return "bg-amber-900/40 text-amber-400";
    case "star":
      return "bg-blue-900/40 text-blue-400";
    default:
      return "bg-gray-800 text-gray-400";
  }
}

function agentStatusDot(status: string): string {
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

const SwarmDetail: Component = () => {
  const projectId = () => (route() as any).projectId ?? "";
  const swarmId = () => (route() as any).swarmId ?? "";

  const swarm = createMemo(() =>
    swarms().find(
      (s: any) => (s.id ?? s.swarm_id ?? "") === swarmId(),
    ),
  );

  const tasks = createMemo(() =>
    swarmTasks().filter(
      (t: any) => (t.swarm_id ?? t.swarmId ?? "") === swarmId(),
    ),
  );

  const agents = createMemo(() =>
    swarmAgents().filter(
      (a: any) => (a.swarm_id ?? a.swarmId ?? "") === swarmId(),
    ),
  );

  // Progress
  const completedCount = createMemo(
    () => tasks().filter((t: any) => t.status === "completed").length,
  );
  const totalCount = createMemo(() => tasks().length);
  const progressPct = createMemo(() =>
    totalCount() > 0
      ? Math.round((completedCount() / totalCount()) * 100)
      : 0,
  );

  // Resolve agent name by id
  function agentName(agentId: string): string {
    if (!agentId) return "--";
    const a =
      swarmAgents().find(
        (ag: any) => (ag.id ?? ag.agent_id ?? "") === agentId,
      );
    return a?.name ?? a?.agent_name ?? agentId.slice(0, 8);
  }

  function getHeartbeat(agentId: string): any {
    return agentHeartbeats().find(
      (h: any) => (h.agent_id ?? "") === agentId,
    );
  }

  function handleBack() {
    navigate({ page: "project-swarms", projectId: projectId() });
  }

  function handleTaskClick(taskId: string) {
    navigate({
      page: "project-swarm-task",
      projectId: projectId(),
      swarmId: swarmId(),
      taskId,
    });
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
      {/* Back button */}
      <button
        class="mb-4 flex items-center gap-1 text-xs text-gray-400 transition-colors hover:text-gray-200"
        onClick={handleBack}
      >
        <span>&larr;</span>
        <span>Back to Swarms</span>
      </button>

      <Show
        when={swarm()}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-400">
              Swarm not found: {swarmId()}
            </p>
          </div>
        }
      >
        {(s) => {
          const name = () => s().name ?? "unnamed";
          const topology = () =>
            s().topology ?? s().swarm_topology ?? "unknown";
          const status = () => s().status ?? s().state ?? "unknown";
          const createdAt = () => s().created_at ?? s().createdAt ?? "";

          return (
            <>
              {/* Header */}
              <div class="mb-6">
                <div class="flex items-center gap-2">
                  <h2 class="text-lg font-semibold text-gray-100">
                    {name()}
                  </h2>
                  <span
                    class={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${topologyBadgeClass(topology())}`}
                  >
                    {topology()}
                  </span>
                  <span class="rounded-full bg-gray-800 px-2 py-0.5 text-[10px] font-semibold uppercase text-gray-300">
                    {status()}
                  </span>
                </div>
                <Show when={createdAt()}>
                  <p class="mt-1 text-[10px] text-gray-500">
                    Created {relativeTime(createdAt())}
                  </p>
                </Show>
              </div>

              {/* Progress Bar */}
              <div class="mb-6">
                <div class="mb-1 flex items-center justify-between text-[10px] text-gray-400">
                  <span>Progress</span>
                  <span>
                    {completedCount()}/{totalCount()} tasks ({progressPct()}%)
                  </span>
                </div>
                <div class="h-2 w-full overflow-hidden rounded-full bg-gray-800">
                  <div
                    class="h-full rounded-full bg-green-500 transition-all"
                    style={{ width: `${progressPct()}%` }}
                  />
                </div>
              </div>

              {/* Task List */}
              <SectionHeader title="Tasks" count={totalCount()} />
              <Show
                when={tasks().length > 0}
                fallback={
                  <p class="mb-6 text-xs text-gray-500">
                    No tasks in this swarm
                  </p>
                }
              >
                <div class="mb-6 space-y-1.5">
                  <For each={tasks()}>
                    {(task) => {
                      const tid = task.id ?? task.task_id ?? "";
                      const assignee =
                        task.assigned_to ?? task.agent_id ?? "";
                      const taskStatus = task.status ?? "pending";
                      const result = task.result ?? "";

                      return (
                        <button
                          class="flex w-full items-center gap-2 rounded-lg border border-gray-800 bg-gray-900/50 px-3 py-2 text-left text-xs transition-colors hover:border-gray-600"
                          onClick={() => handleTaskClick(tid)}
                        >
                          <span class="flex-1 truncate text-gray-100">
                            {task.title ?? "Untitled"}
                          </span>
                          <Show when={assignee}>
                            <button
                              class="shrink-0 text-[10px] text-cyan-400 hover:underline"
                              onClick={(e) => {
                                e.stopPropagation();
                                handleAgentClick(assignee);
                              }}
                            >
                              {agentName(assignee)}
                            </button>
                          </Show>
                          <span
                            class={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${taskStatusClass(taskStatus)}`}
                          >
                            {taskStatus}
                          </span>
                          <Show
                            when={taskStatus === "completed" && result}
                          >
                            <span
                              class="max-w-[120px] shrink-0 truncate text-[10px] text-gray-500"
                              title={result}
                            >
                              {result}
                            </span>
                          </Show>
                        </button>
                      );
                    }}
                  </For>
                </div>
              </Show>

              {/* Agent Roster */}
              <SectionHeader
                title="Agent Roster"
                count={agents().length}
              />
              <Show
                when={agents().length > 0}
                fallback={
                  <p class="mb-6 text-xs text-gray-500">
                    No agents in this swarm
                  </p>
                }
              >
                <div class="mb-6 space-y-1.5">
                  <For each={agents()}>
                    {(agent) => {
                      const aid = agent.id ?? agent.agent_id ?? "";
                      const aName =
                        agent.name ?? agent.agent_name ?? "unnamed";
                      const aStatus =
                        agent.status ?? agent.state ?? "unknown";
                      const aWorktree =
                        agent.worktree ?? agent.worktree_path ?? "";
                      const hb = () => getHeartbeat(aid);

                      return (
                        <button
                          class="flex w-full items-center gap-2 rounded-lg border border-gray-800 bg-gray-900/50 px-3 py-2 text-left text-xs transition-colors hover:border-gray-600"
                          onClick={() => handleAgentClick(aid)}
                        >
                          <span
                            class={`h-2 w-2 shrink-0 rounded-full ${agentStatusDot(aStatus)}`}
                          />
                          <span class="truncate text-gray-100">
                            {aName}
                          </span>
                          <span class="shrink-0 text-[10px] text-gray-500">
                            {relativeTime(
                              hb()?.timestamp ?? hb()?.last_seen,
                            )}
                          </span>
                          <Show when={aWorktree}>
                            <span class="ml-auto max-w-[200px] truncate font-mono text-[10px] text-gray-500">
                              {aWorktree}
                            </span>
                          </Show>
                        </button>
                      );
                    }}
                  </For>
                </div>
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

export default SwarmDetail;
