/**
 * SwarmMonitor.tsx — Main swarm visualization pane.
 *
 * Composes SwarmHeader (phase + progress), TaskDAG (dependency graph),
 * and SwarmTimeline (event log) into a single view.
 * All data from SpacetimeDB subscriptions via connection store.
 */
import { Component, Show, createMemo } from "solid-js";
import { swarms, swarmTasks, swarmAgents } from "../../stores/connection";
import SwarmHeader from "./SwarmHeader";
import TaskDAG, { type TaskNode } from "./TaskDAG";
import SwarmTimeline, { tasksToTimeline } from "./SwarmTimeline";

const SwarmMonitor: Component<{ swarmId: string }> = (props) => {
  const swarm = createMemo(() =>
    swarms().find((s: any) => (s.id ?? s.swarm_id ?? "") === props.swarmId)
  );

  const tasks = createMemo(() =>
    swarmTasks().filter((t: any) => (t.swarmId ?? t.swarm_id ?? "") === props.swarmId)
  );

  const agents = createMemo(() =>
    swarmAgents().filter((a: any) => (a.swarmId ?? a.swarm_id ?? "") === props.swarmId)
  );

  // Map tasks to DAG nodes
  const dagNodes = createMemo<TaskNode[]>(() =>
    tasks().map((t: any) => ({
      id: t.id ?? t.task_id ?? "",
      title: t.title ?? "Untitled",
      status: t.status ?? "pending",
      agentId: t.assigned_to ?? t.agent_id ?? undefined,
      agentName: t.agent_name ?? undefined,
      dependsOn: t.depends_on ?? t.dependsOn ?? [],
    }))
  );

  const timelineEvents = createMemo(() => tasksToTimeline(tasks()));

  const swarmInfo = createMemo(() => ({
    name: swarm()?.name ?? swarm()?.swarm_name ?? props.swarmId,
    topology: swarm()?.topology ?? "mesh",
    status: swarm()?.status ?? "active",
    tasks: tasks().map((t: any) => ({ status: t.status ?? "pending" })),
    agents: agents().length,
  }));

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      <Show
        when={swarm()}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-300">Swarm not found: {props.swarmId}</p>
          </div>
        }
      >
        {/* Header — phase + progress */}
        <SwarmHeader swarm={swarmInfo()} />

        {/* Main content: DAG left, Timeline right */}
        <div class="flex flex-1 gap-4 overflow-hidden">
          {/* Task DAG */}
          <div class="flex-1 overflow-auto rounded-lg border border-gray-800 bg-gray-900/30 p-3">
            <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300">
              Task Graph
            </h4>
            <Show
              when={dagNodes().length > 0}
              fallback={<p class="text-xs text-gray-300">No tasks in this swarm</p>}
            >
              <div class="overflow-auto">
                <TaskDAG tasks={dagNodes()} />
              </div>
            </Show>
          </div>

          {/* Timeline sidebar */}
          <div class="w-64 shrink-0 overflow-auto rounded-lg border border-gray-800 bg-gray-900/30 p-3">
            <h4 class="mb-3 text-[11px] font-semibold uppercase tracking-wider text-gray-300">
              Timeline
            </h4>
            <SwarmTimeline events={timelineEvents()} />
          </div>
        </div>
      </Show>
    </div>
  );
};

export default SwarmMonitor;
