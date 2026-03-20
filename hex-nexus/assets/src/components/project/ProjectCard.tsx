/**
 * ProjectCard.tsx — A card showing project health, agent count, and active swarm.
 *
 * Data comes from SpacetimeDB subscriptions (registryAgents, swarms) filtered
 * by project ID. Click opens the project in a new pane.
 */
import { Component, createMemo, Show } from "solid-js";
import { registryAgents, swarms, swarmTasks } from "../../stores/connection";
import { openPane } from "../../stores/panes";

export interface ProjectInfo {
  id: string;
  name: string;
  path: string;
  health?: "green" | "yellow" | "red";
  lastActivity?: string;
}

const healthColors = {
  green: "bg-green-500",
  yellow: "bg-yellow-500",
  red: "bg-red-500",
};

const ProjectCard: Component<{ project: ProjectInfo }> = (props) => {
  const agentCount = createMemo(
    () => registryAgents().filter((a: any) =>
      (a.project ?? a.project_id ?? "") === props.project.id
    ).length
  );

  const projectSwarms = createMemo(
    () => swarms().filter((s: any) =>
      (s.project ?? s.project_id ?? "") === props.project.id
    )
  );

  const totalTasks = createMemo(() => {
    const swarmIds = new Set(projectSwarms().map((s: any) => s.id ?? s.swarm_id));
    return swarmTasks().filter((t: any) => swarmIds.has(t.swarmId ?? t.swarm_id)).length;
  });

  function handleClick() {
    openPane("filetree", props.project.name, { projectId: props.project.id });
  }

  return (
    <button
      class="group flex flex-col gap-3 rounded-lg border border-gray-800 bg-gray-900/60 p-4 text-left transition-all hover:border-gray-700 hover:bg-gray-900 focus:outline-none focus:ring-1 focus:ring-cyan-500/50"
      onClick={handleClick}
    >
      {/* Header row */}
      <div class="flex items-center gap-2">
        <span
          class={`h-2.5 w-2.5 shrink-0 rounded-full ${healthColors[props.project.health ?? "green"]}`}
        />
        <span class="truncate text-sm font-semibold text-gray-100 group-hover:text-white">
          {props.project.name}
        </span>
      </div>

      {/* Path */}
      <p class="truncate font-mono text-[11px] text-gray-300">
        {props.project.path}
      </p>

      {/* Stats row */}
      <div class="flex items-center gap-3">
        <Stat label="agents" value={agentCount()} />
        <Stat label="swarms" value={projectSwarms().length} />
        <Stat label="tasks" value={totalTasks()} />
      </div>

      {/* Active swarm indicator */}
      <Show when={projectSwarms().length > 0}>
        <div class="flex items-center gap-2 rounded bg-cyan-900/20 px-2 py-1">
          <div class="h-1.5 w-1.5 animate-pulse rounded-full bg-cyan-400" />
          <span class="truncate text-[10px] text-cyan-300">
            {projectSwarms()[0]?.name ?? projectSwarms()[0]?.swarm_name ?? "active swarm"}
          </span>
        </div>
      </Show>

      {/* Last activity */}
      <Show when={props.project.lastActivity}>
        <p class="text-[10px] text-gray-300">
          Last: {props.project.lastActivity}
        </p>
      </Show>
    </button>
  );
};

const Stat: Component<{ label: string; value: number }> = (props) => (
  <div class="flex items-baseline gap-1">
    <span class="text-sm font-bold text-gray-100">{props.value}</span>
    <span class="text-[10px] text-gray-300">{props.label}</span>
  </div>
);

export default ProjectCard;
