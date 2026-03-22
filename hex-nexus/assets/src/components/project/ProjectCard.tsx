/**
 * ProjectCard.tsx — A card showing project health, agent count, and active swarm.
 *
 * Data comes from SpacetimeDB subscriptions (registryAgents, swarms) filtered
 * by project ID. Click opens the project in a new pane.
 */
import { Component, createMemo, createSignal, Show } from "solid-js";
import { registryAgents, swarms, swarmTasks } from "../../stores/connection";
import { openPane } from "../../stores/panes";

export interface ProjectInfo {
  id: string;
  name: string;
  path: string;
  health?: "green" | "yellow" | "red";
  lastActivity?: string;
}

export type ProjectAction = "hide" | "unregister" | "archive" | "delete";

const healthColors = {
  green: "bg-green-500",
  yellow: "bg-yellow-500",
  red: "bg-red-500",
};

const ProjectCard: Component<{
  project: ProjectInfo;
  onAction?: (action: ProjectAction, id: string) => void;
}> = (props) => {
  const [menuOpen, setMenuOpen] = createSignal(false);

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

  function fireAction(action: ProjectAction) {
    setMenuOpen(false);
    props.onAction?.(action, props.project.id);
  }

  return (
    <div class="relative">
      <button
        class="group flex w-full flex-col gap-3 rounded-lg border border-gray-800 bg-gray-900/60 p-4 text-left transition-all hover:border-gray-700 hover:bg-gray-900 focus:outline-none focus:ring-1 focus:ring-cyan-500/50"
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

      {/* Actions menu trigger */}
      <Show when={props.onAction}>
        <button
          class="absolute top-2 right-2 rounded p-1 text-gray-500 opacity-0 transition-all hover:bg-gray-800 hover:text-gray-300 group-hover:opacity-100"
          onClick={(e) => {
            e.stopPropagation();
            setMenuOpen(!menuOpen());
          }}
          title="Project actions"
        >
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="currentColor">
            <circle cx="12" cy="5" r="1.5" />
            <circle cx="12" cy="12" r="1.5" />
            <circle cx="12" cy="19" r="1.5" />
          </svg>
        </button>

        {/* Dropdown menu */}
        <Show when={menuOpen()}>
          <div
            class="absolute top-8 right-2 z-50 min-w-[160px] rounded-lg border border-gray-700 bg-gray-900 py-1 shadow-xl"
            onClick={(e) => e.stopPropagation()}
          >
            <MenuItem
              label="Hide"
              description="Hide from dashboard view"
              onClick={() => fireAction("hide")}
            />
            <MenuItem
              label="Unregister"
              description="Remove from nexus registry"
              onClick={() => fireAction("unregister")}
            />
            <div class="my-1 border-t border-gray-800" />
            <MenuItem
              label="Archive"
              description="Remove config, keep source files"
              onClick={() => fireAction("archive")}
              class="text-yellow-400"
            />
            <MenuItem
              label="Delete from disk"
              description="Permanently remove all files"
              onClick={() => fireAction("delete")}
              class="text-red-400"
            />
          </div>
          {/* Backdrop to close menu */}
          <div
            class="fixed inset-0 z-40"
            onClick={() => setMenuOpen(false)}
          />
        </Show>
      </Show>
    </div>
  );
};

const Stat: Component<{ label: string; value: number }> = (props) => (
  <div class="flex items-baseline gap-1">
    <span class="text-sm font-bold text-gray-100">{props.value}</span>
    <span class="text-[10px] text-gray-300">{props.label}</span>
  </div>
);

const MenuItem: Component<{
  label: string;
  description: string;
  onClick: () => void;
  class?: string;
}> = (props) => (
  <button
    class="flex w-full flex-col gap-0.5 px-3 py-2 text-left transition-colors hover:bg-gray-800"
    onClick={props.onClick}
  >
    <span class={`text-xs font-medium ${props.class ?? "text-gray-200"}`}>
      {props.label}
    </span>
    <span class="text-[10px] text-gray-500">{props.description}</span>
  </button>
);

export default ProjectCard;
