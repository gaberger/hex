/**
 * ProjectOverview.tsx — Default center view showing up to 4 ProjectCards.
 *
 * Max 4 visible projects (matches the 4-pane limit). Additional projects
 * are hidden but accessible via "Show N more". Each card has a dismiss button.
 * Dismissed state stored in localStorage.
 */
import { Component, For, Show, createSignal, createResource, createMemo } from "solid-js";
import ProjectCard, { type ProjectInfo } from "./ProjectCard";
import { registryAgents, swarms, anyConnected } from "../../stores/connection";

const MAX_VISIBLE = 4;
const DISMISSED_KEY = "hex_dismissed_projects";

function loadDismissed(): Set<string> {
  try {
    const raw = localStorage.getItem(DISMISSED_KEY);
    return raw ? new Set(JSON.parse(raw)) : new Set();
  } catch {
    return new Set();
  }
}

function saveDismissed(ids: Set<string>) {
  localStorage.setItem(DISMISSED_KEY, JSON.stringify([...ids]));
}

async function fetchProjects(): Promise<ProjectInfo[]> {
  try {
    const res = await fetch("/api/projects");
    if (!res.ok) return [];
    const data = await res.json();
    return (data.projects ?? data ?? []).map((p: any) => ({
      id: p.id ?? p.project_id ?? p.name,
      name: p.name ?? p.project_name ?? "unnamed",
      path: p.path ?? p.project_path ?? "",
      health: p.health ?? "green",
      lastActivity: p.last_activity ?? undefined,
    }));
  } catch {
    return [];
  }
}

const ProjectOverview: Component = () => {
  const [projects, { refetch }] = createResource(fetchProjects);
  const [registering, setRegistering] = createSignal(false);
  const [newPath, setNewPath] = createSignal("");
  const [dismissed, setDismissed] = createSignal(loadDismissed());
  const [showAll, setShowAll] = createSignal(false);

  const visibleProjects = createMemo(() => {
    const all = projects() ?? [];
    const hidden = dismissed();
    const active = all.filter(p => !hidden.has(p.id));
    const extra = all.filter(p => hidden.has(p.id));
    return { active, extra, total: all.length };
  });

  const displayProjects = createMemo(() => {
    const { active, extra } = visibleProjects();
    if (showAll()) return [...active, ...extra];
    return active.slice(0, MAX_VISIBLE);
  });

  const hiddenCount = createMemo(() => {
    const { active, extra } = visibleProjects();
    if (showAll()) return 0;
    return Math.max(0, active.length - MAX_VISIBLE) + extra.length;
  });

  function dismissProject(id: string) {
    const next = new Set(dismissed());
    next.add(id);
    setDismissed(next);
    saveDismissed(next);
  }

  function restoreProject(id: string) {
    const next = new Set(dismissed());
    next.delete(id);
    setDismissed(next);
    saveDismissed(next);
  }

  function restoreAll() {
    setDismissed(new Set());
    saveDismissed(new Set());
    setShowAll(false);
  }

  async function registerProject(e: Event) {
    e.preventDefault();
    const path = newPath().trim();
    if (!path) return;
    setRegistering(true);
    try {
      await fetch("/api/projects/register", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path }),
      });
      setNewPath("");
      refetch();
    } finally {
      setRegistering(false);
    }
  }

  return (
    <div class="flex h-full flex-col overflow-auto p-6">
      {/* Header */}
      <div class="mb-6 flex items-center justify-between">
        <div>
          <h2 class="text-lg font-semibold text-gray-100">Projects</h2>
          <p class="text-xs text-gray-300">
            Multi-project agent control plane
          </p>
        </div>

        <div class="flex items-center gap-3">
          <Show when={dismissed().size > 0}>
            <button
              class="rounded border border-gray-700 px-2 py-1 text-[10px] text-gray-300 hover:text-gray-300 transition-colors"
              onClick={restoreAll}
            >
              Restore all ({dismissed().size})
            </button>
          </Show>
          <div class="flex items-center gap-2">
            <span
              class="h-2 w-2 rounded-full"
              classList={{
                "bg-green-500": anyConnected(),
                "bg-red-500": !anyConnected(),
              }}
            />
            <span class="text-[10px] text-gray-300">
              {anyConnected() ? "SpacetimeDB connected" : "Connecting..."}
            </span>
          </div>
        </div>
      </div>

      {/* Stats bar */}
      <div class="mb-6 flex gap-6 rounded-lg border border-gray-800 bg-gray-900/50 px-5 py-3">
        <StatBlock label="Projects" value={visibleProjects().total} />
        <StatBlock label="Agents" value={registryAgents().length} />
        <StatBlock label="Swarms" value={swarms().length} />
      </div>

      {/* Project grid */}
      <Show
        when={(projects()?.length ?? 0) > 0}
        fallback={<EmptyState onRegister={registerProject} path={newPath} setPath={setNewPath} registering={registering} />}
      >
        <div class="grid gap-4 grid-cols-[repeat(auto-fill,minmax(280px,1fr))]">
          <For each={displayProjects()}>
            {(project) => (
              <div class="relative group">
                <ProjectCard project={project} />
                {/* Dismiss button */}
                <button
                  class="absolute top-2 right-2 rounded p-1 text-gray-300 opacity-0 group-hover:opacity-100 hover:bg-gray-800 hover:text-gray-300 transition-all"
                  onClick={(e) => {
                    e.stopPropagation();
                    dismissProject(project.id);
                  }}
                  title="Dismiss project"
                >
                  <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <line x1="18" y1="6" x2="6" y2="18" />
                    <line x1="6" y1="6" x2="18" y2="18" />
                  </svg>
                </button>

                {/* Restore indicator for dismissed projects shown in "show all" mode */}
                <Show when={dismissed().has(project.id)}>
                  <div class="absolute inset-0 flex items-center justify-center rounded-lg bg-gray-950/60 backdrop-blur-sm">
                    <button
                      class="rounded bg-gray-800 px-3 py-1.5 text-xs text-gray-300 hover:bg-gray-700 transition-colors"
                      onClick={() => restoreProject(project.id)}
                    >
                      Restore
                    </button>
                  </div>
                </Show>
              </div>
            )}
          </For>

          {/* Add project card — only if under limit */}
          <Show when={displayProjects().length < MAX_VISIBLE && !showAll()}>
            <AddProjectCard
              onRegister={registerProject}
              path={newPath}
              setPath={setNewPath}
              registering={registering}
            />
          </Show>
        </div>

        {/* Show more / Show less */}
        <Show when={hiddenCount() > 0}>
          <button
            class="mt-4 w-full rounded border border-gray-800 py-2 text-xs text-gray-300 hover:border-gray-700 hover:text-gray-300 transition-colors"
            onClick={() => setShowAll(true)}
          >
            Show {hiddenCount()} more project{hiddenCount() > 1 ? "s" : ""}
          </button>
        </Show>
        <Show when={showAll()}>
          <button
            class="mt-4 w-full rounded border border-gray-800 py-2 text-xs text-gray-300 hover:border-gray-700 hover:text-gray-300 transition-colors"
            onClick={() => setShowAll(false)}
          >
            Show fewer
          </button>
        </Show>
      </Show>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

const StatBlock: Component<{ label: string; value: number }> = (props) => (
  <div class="flex items-baseline gap-2">
    <span class="text-xl font-bold text-gray-100">{props.value}</span>
    <span class="text-xs text-gray-300">{props.label}</span>
  </div>
);

const EmptyState: Component<{
  onRegister: (e: Event) => void;
  path: () => string;
  setPath: (v: string) => void;
  registering: () => boolean;
}> = (props) => (
  <div class="flex flex-1 flex-col items-center justify-center gap-6 text-center">
    <div class="rounded-full border border-gray-800 bg-gray-900 p-4">
      <svg class="h-8 w-8 text-gray-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
      </svg>
    </div>
    <div>
      <p class="text-lg font-semibold text-gray-100">Register a project to get started</p>
      <p class="mt-2 text-sm text-gray-300 max-w-md mx-auto">
        Enter the absolute path to your project directory below. hex-nexus will analyze its architecture, track agents, and coordinate swarms for it.
      </p>
    </div>
    <form class="w-full max-w-lg" onSubmit={props.onRegister}>
      <div class="flex gap-2">
        <input
          type="text"
          placeholder="e.g. /Users/gary/projects/my-app"
          value={props.path()}
          onInput={(e) => props.setPath(e.currentTarget.value)}
          class="flex-1 rounded border border-gray-700 bg-gray-800 px-3 py-2.5 text-sm text-gray-100 placeholder-gray-300 focus:border-cyan-600 focus:outline-none focus:ring-1 focus:ring-cyan-600"
        />
        <button
          type="submit"
          disabled={props.registering() || !props.path().trim()}
          class="rounded bg-cyan-600 px-5 py-2.5 text-sm font-medium text-white transition-colors hover:bg-cyan-500 disabled:opacity-50"
        >
          {props.registering() ? "Registering..." : "Register"}
        </button>
      </div>
      <p class="mt-2 text-[11px] text-gray-300">
        Or from CLI: <code class="rounded bg-gray-800 px-1.5 py-0.5 font-mono text-cyan-300">hex project register /path/to/project</code>
      </p>
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
    <div class="flex flex-col rounded-lg border border-dashed border-gray-700 bg-gray-900/30 p-4">
      <Show
        when={expanded()}
        fallback={
          <button
            class="flex flex-1 flex-col items-center justify-center gap-2 text-gray-300 hover:text-gray-300 transition-colors"
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
            class="rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 placeholder-gray-600 focus:border-cyan-600 focus:outline-none"
            autofocus
          />
          <div class="flex gap-2">
            <button
              type="submit"
              disabled={props.registering()}
              class="flex-1 rounded bg-cyan-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 disabled:opacity-50"
            >
              Register
            </button>
            <button
              type="button"
              class="rounded border border-gray-700 px-3 py-1.5 text-xs text-gray-300 hover:text-gray-300"
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

export default ProjectOverview;
