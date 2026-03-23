/**
 * SwarmDetail.tsx — Detail view for a single swarm.
 *
 * Shows swarm metadata, task list with status badges, agent roster,
 * and an overall progress bar. Data from SpacetimeDB subscriptions.
 */
import { Component, For, Show, createMemo, createSignal } from "solid-js";
import {
  swarms,
  swarmTasks,
  swarmAgents,
  agentHeartbeats,
  registryAgents,
  getHexfloConn,
} from "../../stores/connection";
import { navigate, route } from "../../stores/router";
import { addToast } from "../../stores/toast";
import QualityGatePanel from "../fleet/QualityGatePanel";

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

              {/* Task Create Form */}
              <TaskCreateForm swarmId={swarmId()} />

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
                      const assignee = task.assigned_to ?? task.agent_id ?? "";
                      const taskStatus = task.status ?? "pending";
                      const result = task.result ?? "";
                      const assignedAgent = () => {
                        if (!assignee) return null;
                        return registryAgents().find((a: any) => (a.agent_id ?? a.id ?? "") === assignee)
                          ?? swarmAgents().find((a: any) => (a.id ?? a.agent_id ?? "") === assignee);
                      };
                      const worktreePath = () => assignedAgent()?.worktree_path ?? assignedAgent()?.worktree ?? "";
                      const commitHash = () => {
                        const match = (result ?? "").match(/\b([0-9a-f]{7,40})\b/);
                        return match ? match[1] : "";
                      };

                      return (
                        <div class="rounded-lg border border-gray-800 bg-gray-900/50 transition-colors hover:border-gray-600">
                          <button
                            class="flex w-full items-center gap-2 px-3 py-2 text-left text-xs"
                            onClick={() => handleTaskClick(tid)}
                          >
                            <span class="flex-1 truncate text-gray-100">{task.title ?? "Untitled"}</span>
                            <Show when={assignee}>
                              <button
                                class="shrink-0 text-[10px] text-cyan-400 hover:underline"
                                onClick={(e) => { e.stopPropagation(); handleAgentClick(assignee); }}
                              >
                                {agentName(assignee)}
                              </button>
                            </Show>
                            <span class={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${taskStatusClass(taskStatus)}`}>
                              {taskStatus}
                            </span>
                          </button>
                          <Show when={worktreePath() || (taskStatus === "completed" && result)}>
                            <div class="flex items-center gap-3 border-t border-gray-800/50 px-3 py-1.5 text-[10px]">
                              <Show when={worktreePath()}>
                                <span class="flex items-center gap-1 text-gray-500">
                                  <svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                                    <path d="M6 3v12" /><circle cx="18" cy="6" r="3" /><circle cx="6" cy="18" r="3" />
                                    <path d="M18 9a9 9 0 0 1-9 9" />
                                  </svg>
                                  <span class="max-w-[250px] truncate font-mono" title={worktreePath()}>{worktreePath()}</span>
                                </span>
                              </Show>
                              <Show when={commitHash()}>
                                <span class="flex items-center gap-1 text-green-400/70">
                                  <svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                                    <circle cx="12" cy="12" r="4" /><line x1="1.05" y1="12" x2="7" y2="12" /><line x1="17.01" y1="12" x2="22.96" y2="12" />
                                  </svg>
                                  <span class="font-mono">{commitHash().slice(0, 7)}</span>
                                </span>
                              </Show>
                              <Show when={taskStatus === "completed" && result && !commitHash()}>
                                <span class="max-w-[200px] truncate text-gray-500" title={result}>{result}</span>
                              </Show>
                            </div>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </Show>

              {/* Quality Gates */}
              <div class="mb-6">
                <QualityGatePanel />
              </div>

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

/** Inline task creation form — calls SpacetimeDB taskCreate reducer. */
const TaskCreateForm: Component<{ swarmId: string }> = (props) => {
  const [title, setTitle] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);

  async function handleSubmit(e: Event) {
    e.preventDefault();
    const t = title().trim();
    if (!t) return;
    const conn = getHexfloConn();
    if (!conn) { addToast("error", "SpacetimeDB not connected"); return; }
    setSubmitting(true);
    try {
      conn.reducers.taskCreate(crypto.randomUUID(), props.swarmId, t, new Date().toISOString());
      addToast("success", `Task created: ${t}`);
      setTitle("");
    } catch (err: any) {
      addToast("error", `Failed: ${err.message}`);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form class="mb-4 flex items-center gap-2" onSubmit={handleSubmit}>
      <input
        type="text"
        placeholder="New task title..."
        value={title()}
        onInput={(e) => setTitle(e.currentTarget.value)}
        class="flex-1 rounded-lg border border-gray-700 bg-gray-900 px-3 py-1.5 text-xs text-gray-200 placeholder-gray-500 outline-none focus:border-cyan-600 transition-colors"
        disabled={submitting()}
      />
      <button
        type="submit"
        class="shrink-0 rounded-lg border border-cyan-700 bg-cyan-900/30 px-3 py-1.5 text-xs font-medium text-cyan-400 transition-colors hover:bg-cyan-800/40 disabled:opacity-50"
        disabled={submitting() || !title().trim()}
      >
        {submitting() ? "Creating..." : "Add Task"}
      </button>
    </form>
  );
};

export default SwarmDetail;
