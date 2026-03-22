/**
 * ProjectOverview.tsx — Default center view showing up to 4 ProjectCards.
 *
 * Max 4 visible projects (matches the 4-pane limit). Additional projects
 * are hidden but accessible via "Show N more". Each card has a dismiss button.
 * Dismissed state stored in localStorage.
 */
import { Component, For, Show, createSignal, createMemo } from "solid-js";
import ProjectCard, { type ProjectInfo, type ProjectAction } from "./ProjectCard";
import { registryAgents, swarms, anyConnected } from "../../stores/connection";
import {
  projects as sharedProjects,
  registerProject as sharedRegisterProject,
  unregisterProject,
  archiveProject,
  deleteProject,
} from "../../stores/projects";

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

const ProjectOverview: Component = () => {
  // Map shared projects to ProjectInfo shape
  const projects = () => sharedProjects().map((p): ProjectInfo => ({
    id: p.id,
    name: p.name,
    path: p.path,
    health: p.health ?? "green",
    lastActivity: p.lastActivity,
  }));

  const [registering, setRegistering] = createSignal(false);
  const [newPath, setNewPath] = createSignal("");
  const [dismissed, setDismissed] = createSignal(loadDismissed());
  const [showAll, setShowAll] = createSignal(false);
  const [confirmDelete, setConfirmDelete] = createSignal<ProjectInfo | null>(null);

  async function handleProjectAction(action: ProjectAction, id: string) {
    const project = projects().find((p) => p.id === id);
    if (!project) return;

    if (action === "hide") {
      dismissProject(id);
    } else if (action === "unregister") {
      await unregisterProject(id);
    } else if (action === "archive") {
      await archiveProject(id);
    } else if (action === "delete") {
      // Show confirmation dialog — this is destructive
      setConfirmDelete(project);
    }
  }

  async function confirmDeleteProject() {
    const project = confirmDelete();
    if (!project) return;
    setConfirmDelete(null);
    await deleteProject(project.id);
  }

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

  async function handleRegisterProject(e: Event) {
    e.preventDefault();
    const path = newPath().trim();
    if (!path) return;
    setRegistering(true);
    try {
      await sharedRegisterProject(path);
      setNewPath("");
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
        when={projects().length > 0}
        fallback={<EmptyState onRegister={handleRegisterProject} path={newPath} setPath={setNewPath} registering={registering} />}
      >
        <div class="grid gap-4 grid-cols-[repeat(auto-fill,minmax(280px,1fr))]">
          <For each={displayProjects()}>
            {(project) => (
              <div class="relative group">
                <ProjectCard project={project} onAction={handleProjectAction} />

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
              onRegister={handleRegisterProject}
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

      {/* Delete confirmation dialog */}
      <Show when={confirmDelete()}>
        {(project) => (
          <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
            <div class="mx-4 max-w-md rounded-xl border border-red-900/50 bg-gray-950 p-6 shadow-2xl">
              <div class="mb-4 flex items-center gap-3">
                <div class="flex h-10 w-10 items-center justify-center rounded-full bg-red-900/30">
                  <svg class="h-5 w-5 text-red-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M12 9v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                </div>
                <div>
                  <h3 class="text-sm font-semibold text-gray-100">
                    Delete project permanently?
                  </h3>
                  <p class="text-xs text-gray-400">This cannot be undone</p>
                </div>
              </div>

              <div class="mb-5 rounded-lg border border-gray-800 bg-gray-900 p-3">
                <p class="text-sm font-medium text-gray-200">{project().name}</p>
                <p class="mt-1 truncate font-mono text-[11px] text-red-300">
                  {project().path}
                </p>
                <p class="mt-2 text-[11px] text-gray-500">
                  All files at this path will be permanently deleted.
                </p>
              </div>

              <div class="flex justify-end gap-2">
                <button
                  class="rounded-lg border border-gray-700 px-4 py-2 text-xs text-gray-300 transition-colors hover:bg-gray-800"
                  onClick={() => setConfirmDelete(null)}
                >
                  Cancel
                </button>
                <button
                  class="rounded-lg bg-red-600 px-4 py-2 text-xs font-medium text-white transition-colors hover:bg-red-500"
                  onClick={confirmDeleteProject}
                >
                  Delete permanently
                </button>
              </div>
            </div>
          </div>
        )}
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
