/**
 * ControlPlane.tsx — Aggregated landing page across ALL projects.
 *
 * Shows connection status, project cards with actions (archive/delete),
 * swarm/agent counts, and global activity summary.
 */
import { Component, For, Show, createMemo, createSignal } from "solid-js";
import {
  swarms,
  swarmTasks,
  swarmAgents,
  registryAgents,
  hexfloConnected,
  agentRegistryConnected,
  inferenceConnected,
  fleetConnected,
} from "../../stores/connection";
import {
  projects,
  registerProject,
  archiveProject,
  deleteProject,
} from "../../stores/projects";
import { navigate } from "../../stores/router";
import { setSwarmInitDialogOpen } from "../../stores/ui";
import { addToast } from "../../stores/toast";
import { entityBelongsToProject } from "../../utils/project-match";

// ---------------------------------------------------------------------------
// Connection Status Banner
// ---------------------------------------------------------------------------

const ConnectionStatus: Component = () => {
  const allConnected = createMemo(
    () => hexfloConnected() && agentRegistryConnected(),
  );

  return (
    <div
      class="flex items-center gap-4 rounded-lg border px-4 py-2.5 text-xs"
      classList={{
        "border-green-800/50 bg-green-950/30": allConnected(),
        "border-yellow-800/50 bg-yellow-950/30": !allConnected(),
      }}
    >
      <StatusDot label="Nexus" connected={true} />
      <StatusDot label="SpacetimeDB" connected={hexfloConnected()} />
      <StatusDot label="Agent Registry" connected={agentRegistryConnected()} />
      <StatusDot label="Inference" connected={inferenceConnected()} />
      <StatusDot label="Fleet" connected={fleetConnected()} />
      <span class="ml-auto text-gray-500">
        {swarms().length} swarm{swarms().length !== 1 ? "s" : ""} ·{" "}
        {registryAgents().length} agent{registryAgents().length !== 1 ? "s" : ""}
      </span>
    </div>
  );
};

const StatusDot: Component<{ label: string; connected: boolean }> = (props) => (
  <div class="flex items-center gap-1.5">
    <span
      class="h-2 w-2 rounded-full"
      classList={{
        "bg-green-400": props.connected,
        "bg-red-400 animate-pulse": !props.connected,
      }}
    />
    <span
      classList={{
        "text-gray-300": props.connected,
        "text-yellow-400": !props.connected,
      }}
    >
      {props.label}
    </span>
  </div>
);

// ---------------------------------------------------------------------------
// Health ring (48x48)
// ---------------------------------------------------------------------------

function scoreColor(score: number | null): string {
  if (score === null) return "text-gray-500";
  if (score >= 80) return "text-green-400";
  if (score >= 60) return "text-yellow-400";
  return "text-red-400";
}

const MiniHealthRing: Component<{ score: number | null }> = (props) => {
  const pct = () => props.score ?? 0;
  const circumference = 2 * Math.PI * 18;

  return (
    <div class="relative shrink-0">
      <svg width="48" height="48" viewBox="0 0 48 48">
        <circle cx="24" cy="24" r="18" fill="none" stroke="currentColor" stroke-width="4" class="text-gray-800" />
        <Show when={props.score !== null}>
          <circle
            cx="24" cy="24" r="18" fill="none" stroke="currentColor" stroke-width="4"
            stroke-linecap="round"
            stroke-dasharray={`${(pct() / 100) * circumference} ${circumference}`}
            transform="rotate(-90 24 24)"
            class={scoreColor(props.score)}
          />
        </Show>
      </svg>
      <span class={`absolute inset-0 flex items-center justify-center text-xs font-bold ${scoreColor(props.score)}`}>
        {props.score !== null ? props.score : "--"}
      </span>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Project Card Action Menu
// ---------------------------------------------------------------------------

const ProjectActions: Component<{ projectId: string; projectName: string }> = (props) => {
  const [open, setOpen] = createSignal(false);
  const [confirming, setConfirming] = createSignal<string | null>(null);

  const handleArchive = async () => {
    if (confirming() !== "archive") {
      setConfirming("archive");
      setTimeout(() => setConfirming((v) => (v === "archive" ? null : v)), 3000);
      return;
    }
    try {
      await archiveProject(props.projectId);
      addToast("success", `Archived ${props.projectName}`);
    } catch (e: any) {
      addToast("error", `Archive failed: ${e.message}`);
    }
    setOpen(false);
    setConfirming(null);
  };

  const handleDelete = async () => {
    if (confirming() !== "delete") {
      setConfirming("delete");
      setTimeout(() => setConfirming((v) => (v === "delete" ? null : v)), 3000);
      return;
    }
    try {
      await deleteProject(props.projectId);
      addToast("success", `Deleted ${props.projectName}`);
    } catch (e: any) {
      addToast("error", `Delete failed: ${e.message}`);
    }
    setOpen(false);
    setConfirming(null);
  };

  return (
    <div class="relative">
      <button
        class="rounded p-1 text-gray-500 opacity-0 transition-all hover:bg-gray-800 hover:text-gray-300 group-hover:opacity-100"
        onClick={(e) => { e.stopPropagation(); setOpen(!open()); }}
        aria-label="Project actions"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
          <circle cx="12" cy="5" r="2" /><circle cx="12" cy="12" r="2" /><circle cx="12" cy="19" r="2" />
        </svg>
      </button>
      <Show when={open()}>
        <div class="absolute right-0 top-8 z-50 w-40 rounded-lg border border-gray-700 bg-gray-900 py-1 shadow-xl">
          <button
            class="flex w-full items-center gap-2 px-3 py-2 text-left text-xs transition-colors hover:bg-gray-800"
            classList={{
              "text-yellow-400": confirming() === "archive",
              "text-gray-300": confirming() !== "archive",
            }}
            onClick={(e) => { e.stopPropagation(); handleArchive(); }}
          >
            {confirming() === "archive" ? "Click again to confirm" : "Archive"}
          </button>
          <button
            class="flex w-full items-center gap-2 px-3 py-2 text-left text-xs transition-colors hover:bg-gray-800"
            classList={{
              "text-red-400": confirming() === "delete",
              "text-gray-300": confirming() !== "delete",
            }}
            onClick={(e) => { e.stopPropagation(); handleDelete(); }}
          >
            {confirming() === "delete" ? "Click again to DELETE" : "Delete"}
          </button>
        </div>
      </Show>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

const ControlPlane: Component = () => {
  const [showRegisterForm, setShowRegisterForm] = createSignal(false);
  const [registering, setRegistering] = createSignal(false);
  const [newPath, setNewPath] = createSignal("");

  // Project list with computed stats
  // Match swarms/agents by project_id OR projectDir containing project name
  const projectList = createMemo(() =>
    projects().map((p) => {
      const pid = p.id;
      const ppath = (p as any).rootPath || (p as any).path || "";

      // Count swarms: use shared project matching utility
      const projectSwarms = swarms().filter((s: any) =>
        entityBelongsToProject(s, pid),
      );

      // Count agents: use shared project matching utility
      const projectAgents = registryAgents().filter((a: any) =>
        entityBelongsToProject(a, pid),
      );

      // Count swarm agents too
      const swarmAgentCount = swarmAgents().filter((sa: any) => {
        const saSwarm = sa.swarm_id ?? sa.swarmId ?? "";
        return projectSwarms.some((s: any) => (s.id ?? s.swarm_id) === saSwarm);
      }).length;

      const totalAgents = projectAgents.length + swarmAgentCount;

      // Active tasks for this project's swarms
      const projectTasks = swarmTasks().filter((t: any) => {
        const tSwarm = t.swarm_id ?? t.swarmId ?? "";
        return projectSwarms.some((s: any) => (s.id ?? s.swarm_id) === tSwarm);
      });
      const activeTasks = projectTasks.filter(
        (t: any) => t.status === "in_progress" || t.status === "running" || t.status === "assigned",
      ).length;

      return { ...p, swarmCount: projectSwarms.length, agentCount: totalAgents, activeTasks, score: null as number | null };
    }),
  );

  // Global totals (including unscoped swarms)
  const totalSwarms = createMemo(() => swarms().filter((s: any) => s.status === "active").length);
  const totalTasksInProgress = createMemo(() =>
    swarmTasks().filter(
      (t: any) => t.status === "in_progress" || t.status === "running" || t.status === "assigned",
    ).length,
  );
  const totalActiveAgents = createMemo(() =>
    registryAgents().filter(
      (a: any) => a.status === "active" || a.status === "running" || a.status === "registered",
    ).length,
  );

  async function handleRegister(e: Event) {
    e.preventDefault();
    const path = newPath().trim();
    if (!path) return;
    setRegistering(true);
    try {
      await registerProject(path);
      setNewPath("");
      setShowRegisterForm(false);
      addToast("success", `Registered ${path}`);
    } catch (e: any) {
      addToast("error", `Registration failed: ${e.message}`);
    } finally {
      setRegistering(false);
    }
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950">
      <div class="flex flex-col gap-5 p-6">
        {/* Connection Status */}
        <ConnectionStatus />

        {/* Header */}
        <div class="flex items-start justify-between">
          <div>
            <h2 class="text-xl font-bold text-gray-100">Control Plane</h2>
            <p class="mt-1 text-xs text-gray-500">
              {projectList().length} project{projectList().length !== 1 ? "s" : ""}
              {" · "}{totalSwarms()} active swarm{totalSwarms() !== 1 ? "s" : ""}
              {" · "}{totalActiveAgents()} agent{totalActiveAgents() !== 1 ? "s" : ""}
              {" · "}{totalTasksInProgress()} task{totalTasksInProgress() !== 1 ? "s" : ""} in progress
            </p>
          </div>
          <div class="flex items-center gap-3">
            <button
              class="rounded-lg bg-cyan-500/20 px-3.5 py-1.5 text-xs font-semibold text-cyan-400 hover:bg-cyan-500/30 transition-colors"
              onClick={() => setShowRegisterForm(true)}
            >
              + Add Project
            </button>
            <button
              class="rounded-lg border border-gray-700 px-3.5 py-1.5 text-xs font-semibold text-gray-300 hover:bg-gray-800 transition-colors"
              onClick={() => setSwarmInitDialogOpen(true)}
            >
              New Swarm
            </button>
          </div>
        </div>

        {/* Inline register form */}
        <Show when={showRegisterForm()}>
          <form class="flex items-center gap-3" onSubmit={handleRegister}>
            <input
              type="text"
              placeholder="/path/to/project"
              value={newPath()}
              onInput={(e) => setNewPath(e.currentTarget.value)}
              autofocus
              class="flex-1 rounded-lg border border-gray-700 bg-gray-900 px-3 py-2 text-xs text-gray-200 outline-none focus:border-cyan-500/50"
            />
            <button
              type="submit"
              disabled={registering() || !newPath().trim()}
              class="rounded-lg bg-cyan-500/20 px-4 py-2 text-xs font-semibold text-cyan-400 disabled:opacity-50"
            >
              {registering() ? "Registering..." : "Register"}
            </button>
            <button
              type="button"
              class="rounded-lg border border-gray-700 px-3 py-2 text-xs text-gray-500"
              onClick={() => { setShowRegisterForm(false); setNewPath(""); }}
            >
              Cancel
            </button>
          </form>
        </Show>

        {/* Project cards grid */}
        <Show
          when={projectList().length > 0}
          fallback={
            <div class="flex flex-1 flex-col items-center justify-center gap-6 py-20 text-center">
              <p class="text-lg font-bold text-gray-300">No projects registered</p>
              <p class="max-w-md text-xs text-gray-500">
                Register a project directory to start tracking its architecture, agents, and swarms.
              </p>
            </div>
          }
        >
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
            <For each={projectList()}>
              {(project) => (
                <div
                  class="group relative flex items-start gap-4 rounded-xl border border-gray-800 bg-gray-900/50 p-4 cursor-pointer transition-all hover:border-cyan-500/30 hover:bg-gray-900"
                  onClick={() => navigate({ page: "project", projectId: project.id })}
                >
                  {/* Mini health ring */}
                  <MiniHealthRing score={project.score} />

                  {/* Info */}
                  <div class="flex flex-1 flex-col overflow-hidden">
                    <span class="truncate text-sm font-bold text-gray-200">
                      {project.name}
                    </span>
                    <div class="mt-1 flex items-center gap-3 text-[11px] text-gray-500">
                      <span classList={{ "text-green-400 font-medium": project.swarmCount > 0 }}>
                        {project.swarmCount} swarm{project.swarmCount !== 1 ? "s" : ""}
                      </span>
                      <span classList={{ "text-cyan-400 font-medium": project.agentCount > 0 }}>
                        {project.agentCount} agent{project.agentCount !== 1 ? "s" : ""}
                      </span>
                      <Show when={project.activeTasks > 0}>
                        <span class="text-yellow-400 font-medium">
                          {project.activeTasks} task{project.activeTasks !== 1 ? "s" : ""} active
                        </span>
                      </Show>
                    </div>
                    <Show when={(project as any).rootPath || (project as any).path}>
                      <span class="mt-1 truncate text-[10px] font-mono text-gray-600">
                        {(project as any).rootPath || (project as any).path}
                      </span>
                    </Show>
                  </div>

                  {/* Action menu */}
                  <ProjectActions projectId={project.id} projectName={project.name} />
                </div>
              )}
            </For>

            {/* Add Project card */}
            <button
              class="flex items-center justify-center gap-2 rounded-xl border border-dashed border-gray-700 bg-transparent p-4 text-sm text-gray-500 cursor-pointer transition-colors hover:border-cyan-500 hover:text-cyan-400"
              onClick={() => setShowRegisterForm(true)}
            >
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
              </svg>
              Add Project
            </button>
          </div>

          {/* Global activity summary */}
          <div class="flex items-center gap-6 rounded-lg border border-gray-800 bg-gray-900 px-5 py-3">
            <div class="flex items-center gap-2">
              <span class="h-2 w-2 rounded-full bg-green-400" classList={{ "animate-pulse": totalSwarms() > 0 }} />
              <span class="text-xs text-gray-300">
                {totalSwarms()} active swarm{totalSwarms() !== 1 ? "s" : ""}
              </span>
            </div>
            <div class="flex items-center gap-2">
              <span class="h-2 w-2 rounded-full bg-cyan-400" classList={{ "animate-pulse": totalTasksInProgress() > 0 }} />
              <span class="text-xs text-gray-300">
                {totalTasksInProgress()} task{totalTasksInProgress() !== 1 ? "s" : ""} in progress
              </span>
            </div>
            <div class="flex items-center gap-2">
              <span class="h-2 w-2 rounded-full bg-blue-400" />
              <span class="text-xs text-gray-300">
                {totalActiveAgents()} active agent{totalActiveAgents() !== 1 ? "s" : ""}
              </span>
            </div>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default ControlPlane;
