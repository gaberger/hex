/**
 * ControlPlane.tsx — Main dashboard view showing all projects, active swarms,
 * and infrastructure status. Intended as the default center pane.
 *
 * Data sources: SpacetimeDB subscriptions via connection + projects stores.
 */
import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { swarms, swarmTasks, registryAgents, anyConnected } from "../../stores/connection";
import { projects, registerProject } from "../../stores/projects";
import { openPane } from "../../stores/panes";
import { setSwarmInitDialogOpen } from "../../stores/ui";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function healthScore(projectId: string): number {
  // Derive a simple health score from agent activity + task completion
  const agents = registryAgents().filter(
    (a: any) => (a.project ?? a.project_id ?? "") === projectId
  );
  const tasks = swarmTasks().filter(
    (t: any) => (t.project ?? t.project_id ?? "") === projectId
  );
  const completed = tasks.filter((t: any) => t.status === "completed" || t.status === "done");
  if (agents.length === 0 && tasks.length === 0) return 100;
  if (tasks.length === 0) return 85;
  return Math.round((completed.length / tasks.length) * 100) || 50;
}

function healthColor(score: number): string {
  if (score >= 80) return "bg-green-500";
  if (score >= 50) return "bg-yellow-500";
  return "bg-red-500";
}

function healthTextColor(score: number): string {
  if (score >= 80) return "text-green-400";
  if (score >= 50) return "text-yellow-400";
  return "text-red-400";
}

function healthBgColor(score: number): string {
  if (score >= 80) return "bg-green-900/30";
  if (score >= 50) return "bg-yellow-900/30";
  return "bg-red-900/30";
}

function timeAgo(iso: string | undefined): string {
  if (!iso) return "never";
  const diff = Date.now() - new Date(iso).getTime();
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

const ControlPlane: Component = () => {
  const [registering, setRegistering] = createSignal(false);
  const [newPath, setNewPath] = createSignal("");

  const projectList = createMemo(() =>
    projects().map((p) => {
      const score = healthScore(p.id);
      const agentCount = registryAgents().filter(
        (a: any) => (a.project ?? a.project_id ?? "") === p.id
      ).length;
      const swarmCount = swarms().filter(
        (s: any) => (s.project ?? s.project_id ?? "") === p.id
      ).length;
      const worktreeCount = 0; // Not yet tracked in SpacetimeDB
      return { ...p, score, agentCount, swarmCount, worktreeCount };
    })
  );

  const activeSwarms = createMemo(() =>
    swarms().filter(
      (s: any) => s.status === "active" || s.status === "running" || !s.status
    )
  );

  function swarmProgress(swarmId: string): number {
    const tasks = swarmTasks().filter(
      (t: any) => (t.swarmId ?? t.swarm_id ?? "") === swarmId
    );
    if (tasks.length === 0) return 0;
    const done = tasks.filter(
      (t: any) => t.status === "completed" || t.status === "done"
    ).length;
    return Math.round((done / tasks.length) * 100);
  }

  function swarmProjectName(swarm: any): string {
    const pid = swarm.project ?? swarm.project_id ?? "";
    const proj = projects().find((p) => p.id === pid);
    return proj?.name ?? pid ?? "unassigned";
  }

  function handleProjectClick(projectId: string, projectName: string) {
    openPane("filetree", projectName, { projectId });
  }

  async function handleRegister(e: Event) {
    e.preventDefault();
    const path = newPath().trim();
    if (!path) return;
    setRegistering(true);
    try {
      await registerProject(path);
      setNewPath("");
    } finally {
      setRegistering(false);
    }
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-6">
      {/* Header */}
      <div class="mb-6 flex items-center justify-between">
        <div>
          <h2 class="text-[22px] font-semibold text-gray-100">Control Plane</h2>
          <p class="mt-0.5 text-xs text-gray-400">
            {projectList().length} project{projectList().length !== 1 ? "s" : ""}
            {" / "}
            {activeSwarms().length} active swarm{activeSwarms().length !== 1 ? "s" : ""}
            {" / "}
            {registryAgents().length} agent{registryAgents().length !== 1 ? "s" : ""}
          </p>
        </div>

        <div class="flex items-center gap-3">
          {/* Connection indicator */}
          <div class="flex items-center gap-1.5">
            <span
              class="h-2 w-2 rounded-full"
              classList={{
                "bg-green-500": anyConnected(),
                "bg-red-500": !anyConnected(),
              }}
            />
            <span class="text-[10px] text-gray-400">
              {anyConnected() ? "Connected" : "Offline"}
            </span>
          </div>

          {/* Action buttons */}
          <button
            class="rounded-lg border border-gray-700 bg-gray-900 px-3 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:border-gray-600 hover:text-gray-100"
            onClick={() => setSwarmInitDialogOpen(true)}
          >
            New Swarm
          </button>
        </div>
      </div>

      {/* Project grid */}
      <Show
        when={projectList().length > 0}
        fallback={
          <EmptyProjects
            onRegister={handleRegister}
            path={newPath}
            setPath={setNewPath}
            registering={registering}
          />
        }
      >
        <div class="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
          <For each={projectList()}>
            {(project) => (
              <button
                class="group flex flex-col gap-3 rounded-xl border border-gray-800 bg-gray-900 p-4 text-left transition-all hover:border-gray-700 hover:bg-[#111827] focus:outline-none focus:ring-1 focus:ring-cyan-500/50"
                onClick={() => handleProjectClick(project.id, project.name)}
              >
                {/* Top row: icon + name + health badge */}
                <div class="flex items-center justify-between">
                  <div class="flex items-center gap-2.5">
                    <svg class="h-4 w-4 shrink-0 text-gray-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                      <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                    </svg>
                    <span class="truncate text-sm font-bold text-gray-100 group-hover:text-white">
                      {project.name}
                    </span>
                  </div>
                  <span
                    class={`rounded-full px-2 py-0.5 text-[10px] font-semibold ${healthBgColor(project.score)} ${healthTextColor(project.score)}`}
                  >
                    {project.score}%
                  </span>
                </div>

                {/* Stats */}
                <div class="flex items-center gap-4 text-[12px] text-gray-400">
                  <span>{project.worktreeCount} worktrees</span>
                  <span>{project.swarmCount} swarms</span>
                  <span>{project.agentCount} agents</span>
                </div>

                {/* Last activity */}
                <p class="text-[11px] text-gray-500">
                  Last activity: {timeAgo((project as any).lastActivity)}
                </p>
              </button>
            )}
          </For>

          {/* Add project card */}
          <AddProjectCard
            onRegister={handleRegister}
            path={newPath}
            setPath={setNewPath}
            registering={registering}
          />
        </div>
      </Show>

      {/* Active swarms section */}
      <Show when={activeSwarms().length > 0}>
        <div class="mt-8">
          <h3 class="mb-3 text-sm font-semibold text-gray-200">Active Swarms</h3>
          <div class="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
            <For each={activeSwarms()}>
              {(swarm) => {
                const progress = () => swarmProgress(swarm.id ?? swarm.swarm_id ?? "");
                const topology = () => swarm.topology ?? swarm.swarm_topology ?? "hierarchical";
                return (
                  <div class="flex flex-col gap-2.5 rounded-xl border border-gray-800 bg-gray-900 p-4">
                    {/* Name + badges */}
                    <div class="flex items-center justify-between">
                      <span class="truncate font-mono text-xs font-semibold text-gray-100">
                        {swarm.name ?? swarm.swarm_name ?? "unnamed"}
                      </span>
                      <div class="flex items-center gap-2">
                        <span class="rounded bg-cyan-900/30 px-1.5 py-0.5 text-[10px] text-cyan-300">
                          {progress()}%
                        </span>
                        <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] uppercase text-gray-400">
                          {topology()}
                        </span>
                      </div>
                    </div>

                    {/* Project */}
                    <p class="text-[11px] text-gray-400">
                      Project: {swarmProjectName(swarm)}
                    </p>

                    {/* Progress bar */}
                    <div class="h-1.5 w-full overflow-hidden rounded-full bg-gray-800">
                      <div
                        class="h-full rounded-full bg-cyan-500 transition-all duration-500"
                        style={{ width: `${progress()}%` }}
                      />
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

const EmptyProjects: Component<{
  onRegister: (e: Event) => void;
  path: () => string;
  setPath: (v: string) => void;
  registering: () => boolean;
}> = (props) => (
  <div class="flex flex-1 flex-col items-center justify-center gap-6 text-center">
    <div class="rounded-full border border-gray-800 bg-gray-900 p-4">
      <svg class="h-8 w-8 text-gray-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
      </svg>
    </div>
    <div>
      <p class="text-lg font-semibold text-gray-100">No projects registered</p>
      <p class="mx-auto mt-2 max-w-md text-sm text-gray-400">
        Register a project directory to start tracking its architecture, agents, and swarms.
      </p>
    </div>
    <form class="w-full max-w-lg" onSubmit={props.onRegister}>
      <div class="flex gap-2">
        <input
          type="text"
          placeholder="/path/to/project"
          value={props.path()}
          onInput={(e) => props.setPath(e.currentTarget.value)}
          class="flex-1 rounded-lg border border-gray-700 bg-gray-800 px-3 py-2.5 text-sm text-gray-100 placeholder-gray-500 focus:border-cyan-600 focus:outline-none focus:ring-1 focus:ring-cyan-600"
        />
        <button
          type="submit"
          disabled={props.registering() || !props.path().trim()}
          class="rounded-lg bg-cyan-600 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-cyan-500 disabled:opacity-50"
        >
          {props.registering() ? "Registering..." : "Register"}
        </button>
      </div>
    </form>
  </div>
);

const AddProjectCard: Component<{
  onRegister: (e: Event) => void;
  path: () => string;
  setPath: (v: string) => void;
  registering: () => boolean;
}> = (props) => {
  const [expanded, setExpanded] = createSignal(false);

  return (
    <div class="flex flex-col rounded-xl border border-dashed border-gray-700 bg-gray-900/30 p-4">
      <Show
        when={expanded()}
        fallback={
          <button
            class="flex flex-1 flex-col items-center justify-center gap-2 py-4 text-gray-400 transition-colors hover:text-gray-200"
            onClick={() => setExpanded(true)}
          >
            <svg class="h-6 w-6" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
            <span class="text-xs">Add Project</span>
          </button>
        }
      >
        <form class="flex flex-col gap-3" onSubmit={props.onRegister}>
          <input
            type="text"
            placeholder="/path/to/project"
            value={props.path()}
            onInput={(e) => props.setPath(e.currentTarget.value)}
            class="rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-200 placeholder-gray-500 focus:border-cyan-600 focus:outline-none"
            autofocus
          />
          <div class="flex gap-2">
            <button
              type="submit"
              disabled={props.registering()}
              class="flex-1 rounded-lg bg-cyan-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 disabled:opacity-50"
            >
              Register
            </button>
            <button
              type="button"
              class="rounded-lg border border-gray-700 px-3 py-1.5 text-xs text-gray-400 hover:text-gray-200"
              onClick={() => setExpanded(false)}
            >
              Cancel
            </button>
          </div>
        </form>
      </Show>
    </div>
  );
};

export default ControlPlane;
