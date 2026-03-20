/**
 * TaskBoard.tsx — Kanban-style task board for a swarm.
 *
 * Shows tasks grouped by status (pending/in_progress/completed/failed)
 * from SpacetimeDB swarm_task subscription.
 */
import { Component, For, Show, createMemo } from "solid-js";
import { swarmTasks, swarmAgents } from "../../stores/connection";

const STATUS_COLUMNS = [
  { key: "pending", label: "Pending", color: "border-gray-600" },
  { key: "in_progress", label: "In Progress", color: "border-cyan-500" },
  { key: "completed", label: "Done", color: "border-green-500" },
  { key: "failed", label: "Failed", color: "border-red-500" },
] as const;

const TaskBoard: Component<{ swarmId: string }> = (props) => {
  const tasks = createMemo(() =>
    swarmTasks().filter((t: any) =>
      (t.swarmId ?? t.swarm_id ?? "") === props.swarmId
    )
  );

  const agents = createMemo(() =>
    swarmAgents().filter((a: any) =>
      (a.swarmId ?? a.swarm_id ?? "") === props.swarmId
    )
  );

  const tasksByStatus = (status: string) =>
    tasks().filter((t: any) => (t.status ?? "pending") === status);

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      {/* Header */}
      <div class="mb-4 flex items-center justify-between">
        <h3 class="text-sm font-semibold text-gray-100">Task Board</h3>
        <div class="flex items-center gap-3 text-[10px] text-gray-300">
          <span>{tasks().length} tasks</span>
          <span>{agents().length} agents</span>
        </div>
      </div>

      {/* Columns */}
      <div class="grid flex-1 grid-cols-4 gap-3">
        <For each={STATUS_COLUMNS}>
          {(col) => (
            <div class={`flex flex-col rounded-lg border-t-2 ${col.color} bg-gray-900/40 p-2`}>
              <div class="mb-2 flex items-center justify-between px-1">
                <span class="text-[10px] font-semibold uppercase tracking-wider text-gray-300">
                  {col.label}
                </span>
                <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300">
                  {tasksByStatus(col.key).length}
                </span>
              </div>
              <div class="space-y-1.5 overflow-auto">
                <For each={tasksByStatus(col.key)}>
                  {(task) => <TaskCard task={task} />}
                </For>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

const TaskCard: Component<{ task: any }> = (props) => (
  <div class="rounded border border-gray-800 bg-gray-900/80 p-2 text-xs">
    <p class="font-medium text-gray-100 truncate">
      {props.task.title ?? props.task.name ?? "Untitled"}
    </p>
    <Show when={props.task.assigned_to ?? props.task.agent_id}>
      <p class="mt-1 text-[10px] text-gray-300 truncate">
        Agent: {props.task.assigned_to ?? props.task.agent_id}
      </p>
    </Show>
  </div>
);

export default TaskBoard;
