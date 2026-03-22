/**
 * ControlPlane.tsx — Aggregated landing page across ALL projects.
 *
 * Shows a grid of project cards with health badges, swarm/agent counts,
 * plus a global activity summary row. Click a card to navigate into that project.
 *
 * Data sources: SpacetimeDB subscriptions via connection + projects stores.
 */
import { Component, For, Show, createMemo, createSignal } from "solid-js";
import {
  swarms,
  swarmTasks,
  registryAgents,
} from "../../stores/connection";
import { projects, registerProject } from "../../stores/projects";
import { navigate } from "../../stores/router";
import { setSwarmInitDialogOpen } from "../../stores/ui";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function healthScore(projectId: string): number | null {
  const tasks = swarmTasks().filter(
    (t: any) => (t.project ?? t.project_id ?? "") === projectId,
  );
  if (tasks.length === 0) return null;
  const completed = tasks.filter(
    (t: any) => t.status === "completed" || t.status === "done",
  );
  return Math.round((completed.length / tasks.length) * 100);
}

function scoreColor(score: number | null): string {
  if (score === null) return "text-gray-500";
  if (score >= 80) return "text-green-400";
  if (score >= 60) return "text-yellow-400";
  return "text-red-400";
}

function scoreRingBg(score: number | null): string {
  if (score === null) return "text-gray-800";
  if (score >= 80) return "text-green-500/20";
  if (score >= 60) return "text-yellow-500/20";
  return "text-red-500/20";
}

// ---------------------------------------------------------------------------
// Small health ring (48x48)
// ---------------------------------------------------------------------------

const MiniHealthRing: Component<{ score: number | null }> = (props) => {
  const pct = () => props.score ?? 0;
  const circumference = 2 * Math.PI * 18; // r=18

  return (
    <div class="relative">
      <svg width="48" height="48" viewBox="0 0 48 48">
        <circle
          cx="24"
          cy="24"
          r="18"
          fill="none"
          stroke="currentColor"
          stroke-width="4"
          class="text-gray-800"
        />
        <Show when={props.score !== null}>
          <circle
            cx="24"
            cy="24"
            r="18"
            fill="none"
            stroke="currentColor"
            stroke-width="4"
            stroke-linecap="round"
            stroke-dasharray={`${(pct() / 100) * circumference} ${circumference}`}
            transform="rotate(-90 24 24)"
            class={scoreColor(props.score)}
          />
        </Show>
      </svg>
      <span
        class={`absolute inset-0 flex items-center justify-center text-xs font-bold ${scoreColor(props.score)}`}
      >
        {props.score !== null ? props.score : "--"}
      </span>
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
  const projectList = createMemo(() =>
    projects().map((p) => {
      const score = healthScore(p.id);
      const agentCount = registryAgents().filter(
        (a: any) => (a.project ?? a.project_id ?? "") === p.id,
      ).length;
      const swarmCount = swarms().filter(
        (s: any) => (s.project ?? s.project_id ?? "") === p.id,
      ).length;
      return { ...p, score, agentCount, swarmCount };
    }),
  );

  // Global activity summary
  const totalTasksInProgress = createMemo(() =>
    swarmTasks().filter(
      (t: any) =>
        t.status === "in_progress" ||
        t.status === "running" ||
        t.status === "assigned",
    ).length,
  );

  const totalActiveAgents = createMemo(() =>
    registryAgents().filter(
      (a: any) =>
        a.status === "active" || a.status === "running" || a.status === "online",
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
    } finally {
      setRegistering(false);
    }
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-[var(--bg-base)]">
      <div class="flex flex-col gap-6 p-6">
        {/* Header */}
        <div class="flex items-start justify-between">
          <div>
            <h2 class="text-[22px] font-bold leading-tight text-[var(--text-body)]">
              Control Plane
            </h2>
            <p class="mt-1 text-[13px] text-[var(--text-faint)]">
              {projectList().length} project
              {projectList().length !== 1 ? "s" : ""}
              {" \u00b7 "}
              {totalActiveAgents()} active agent
              {totalActiveAgents() !== 1 ? "s" : ""}
              {" \u00b7 "}
              {totalTasksInProgress()} task
              {totalTasksInProgress() !== 1 ? "s" : ""} in progress
            </p>
          </div>
          <div class="flex items-center gap-3">
            <button
              class="rounded-lg border-none bg-[color-mix(in_srgb,var(--accent)_30%,transparent)] px-3.5 py-1.5 text-[13px] font-semibold text-[var(--accent-hover)] cursor-pointer"
              onClick={() => setShowRegisterForm(true)}
            >
              + Add Project
            </button>
            <button
              class="rounded-lg border border-[var(--border)] bg-transparent px-3.5 py-1.5 text-[13px] font-semibold text-[var(--text-body)] cursor-pointer"
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
              class="flex-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-2 text-[13px] text-[var(--text-body)] outline-none focus:border-[color-mix(in_srgb,var(--accent-hover)_50%,transparent)]"
            />
            <button
              type="submit"
              disabled={registering() || !newPath().trim()}
              class="rounded-lg border-none bg-[color-mix(in_srgb,var(--accent)_30%,transparent)] px-4 py-2 text-[13px] font-semibold text-[var(--accent-hover)] cursor-pointer disabled:opacity-50"
            >
              {registering() ? "Registering..." : "Register"}
            </button>
            <button
              type="button"
              class="rounded-lg border border-[var(--border)] bg-transparent px-3 py-2 text-[13px] text-[var(--text-faint)] cursor-pointer"
              onClick={() => {
                setShowRegisterForm(false);
                setNewPath("");
              }}
            >
              Cancel
            </button>
          </form>
        </Show>

        {/* Project cards grid */}
        <Show
          when={projectList().length > 0}
          fallback={
            <EmptyState
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
                  class="group flex items-start gap-4 rounded-xl border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 text-left cursor-pointer transition-all hover:border-[color-mix(in_srgb,var(--accent-hover)_40%,transparent)]"
                  onClick={() =>
                    navigate({ page: "project", projectId: project.id })
                  }
                >
                  {/* Mini health ring */}
                  <MiniHealthRing score={project.score} />

                  {/* Info */}
                  <div class="flex flex-1 flex-col overflow-hidden">
                    <span class="truncate text-[15px] font-bold text-[var(--text-body)]">
                      {project.name}
                    </span>
                    <div class="mt-1 flex items-center gap-3 text-[12px] text-[var(--text-faint)]">
                      <span>
                        {project.swarmCount} swarm
                        {project.swarmCount !== 1 ? "s" : ""}
                      </span>
                      <span>
                        {project.agentCount} agent
                        {project.agentCount !== 1 ? "s" : ""}
                      </span>
                    </div>
                  </div>
                </button>
              )}
            </For>

            {/* Add Project card */}
            <button
              class="flex items-center justify-center gap-2 rounded-xl border border-dashed border-gray-700 bg-transparent p-4 text-[14px] text-[var(--text-faint)] cursor-pointer transition-colors hover:border-[var(--accent)] hover:text-[var(--accent-hover)]"
              onClick={() => setShowRegisterForm(true)}
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
              >
                <line x1="12" y1="5" x2="12" y2="19" />
                <line x1="5" y1="12" x2="19" y2="12" />
              </svg>
              Add Project
            </button>
          </div>

          {/* Global activity summary */}
          <div class="flex items-center gap-6 rounded-lg border border-gray-800 bg-gray-900 px-5 py-3">
            <div class="flex items-center gap-2">
              <span class="h-2 w-2 rounded-full bg-[var(--accent-hover)]" />
              <span class="text-[13px] text-[var(--text-body)]">
                {totalTasksInProgress()} task
                {totalTasksInProgress() !== 1 ? "s" : ""} in progress
              </span>
            </div>
            <div class="flex items-center gap-2">
              <span class="h-2 w-2 rounded-full bg-green-400" />
              <span class="text-[13px] text-[var(--text-body)]">
                {totalActiveAgents()} active agent
                {totalActiveAgents() !== 1 ? "s" : ""}
              </span>
            </div>
          </div>
        </Show>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

const EmptyState: Component<{
  onRegister: (e: Event) => void;
  path: () => string;
  setPath: (v: string) => void;
  registering: () => boolean;
}> = (props) => (
  <div class="flex flex-1 flex-col items-center justify-center gap-6 text-center">
    <div class="rounded-full border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4">
      <svg
        width="32"
        height="32"
        viewBox="0 0 24 24"
        fill="none"
        stroke="var(--text-faint)"
        stroke-width="1.5"
      >
        <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
      </svg>
    </div>
    <div>
      <p class="text-[18px] font-bold text-[var(--text-body)]">
        No projects registered
      </p>
      <p class="mx-auto mt-2 max-w-md text-[13px] text-[var(--text-faint)]">
        Register a project directory to start tracking its architecture, agents,
        and swarms.
      </p>
    </div>
    <form class="flex w-full max-w-lg gap-2" onSubmit={props.onRegister}>
      <input
        type="text"
        placeholder="/path/to/project"
        value={props.path()}
        onInput={(e) => props.setPath(e.currentTarget.value)}
        class="flex-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3.5 py-2.5 text-[13px] text-[var(--text-body)] outline-none focus:border-[color-mix(in_srgb,var(--accent-hover)_50%,transparent)]"
      />
      <button
        type="submit"
        disabled={props.registering() || !props.path().trim()}
        class="rounded-lg border-none bg-[color-mix(in_srgb,var(--accent)_30%,transparent)] px-5 py-2.5 text-[13px] font-semibold text-[var(--accent-hover)] cursor-pointer disabled:opacity-50"
      >
        {props.registering() ? "Registering..." : "Register"}
      </button>
    </form>
  </div>
);

export default ControlPlane;
