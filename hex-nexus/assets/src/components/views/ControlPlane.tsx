/**
 * ControlPlane.tsx — Main dashboard view showing all projects, active swarms,
 * and infrastructure status. Matches Pencil design spec exactly.
 *
 * Data sources: SpacetimeDB subscriptions via connection + projects stores.
 */
import { Component, For, Show, createMemo, createSignal, onMount } from "solid-js";
import {
  swarms,
  swarmTasks,
  registryAgents,
  inferenceProviders,
} from "../../stores/connection";
import { projects, registerProject } from "../../stores/projects";
import { navigate } from "../../stores/router";
import { setSpawnDialogOpen, setSwarmInitDialogOpen } from "../../stores/ui";

// Git worktree counts per project (fetched from /api/{id}/git/worktrees)
const [worktreeCounts, setWorktreeCounts] = createSignal<Record<string, number>>({});

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function healthScore(projectId: string): number {
  const agents = registryAgents().filter(
    (a: any) => (a.project ?? a.project_id ?? "") === projectId,
  );
  const tasks = swarmTasks().filter(
    (t: any) => (t.project ?? t.project_id ?? "") === projectId,
  );
  const completed = tasks.filter(
    (t: any) => t.status === "completed" || t.status === "done",
  );
  if (agents.length === 0 && tasks.length === 0) return 100;
  if (tasks.length === 0) return 85;
  return Math.round((completed.length / tasks.length) * 100) || 50;
}

function healthBadge(score: number): { bg: string; text: string } {
  if (score >= 80) return { bg: "#16532580", text: "#4ade80" };
  if (score >= 60) return { bg: "#eab30820", text: "#eab308" };
  return { bg: "#dc262620", text: "#f87171" };
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
// Folder icon (20x20)
// ---------------------------------------------------------------------------

const FolderIcon: Component<{ active?: boolean }> = (props) => (
  <svg
    class="shrink-0"
    width="20"
    height="20"
    viewBox="0 0 24 24"
    fill="none"
    stroke={props.active ? "#22d3ee" : "#6b7280"}
    stroke-width="1.5"
  >
    <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
  </svg>
);

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

const ControlPlane: Component = () => {
  const [showRegisterForm, setShowRegisterForm] = createSignal(false);
  const [registering, setRegistering] = createSignal(false);
  const [newPath, setNewPath] = createSignal("");

  // Fetch worktree counts for all registered projects
  onMount(() => {
    projects().forEach(async (p) => {
      // Ensure project is in REST registry (SpacetimeDB projects may not be)
      if (p.path) {
        await fetch("/api/projects/register", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ rootPath: p.path, name: p.name }),
        }).catch(() => {});
      }

      fetch(`/api/${p.id}/git/worktrees`)
        .then((r) => r.ok ? r.json() : null)
        .then((json) => {
          if (json?.ok) {
            const count = (json.data.worktrees ?? []).filter((w: any) => !w.isBare).length;
            setWorktreeCounts((prev) => ({ ...prev, [p.id]: count }));
          }
        })
        .catch(() => {});
    });
  });

  const projectList = createMemo(() =>
    projects().map((p) => {
      const score = healthScore(p.id);
      const agentCount = registryAgents().filter(
        (a: any) => (a.project ?? a.project_id ?? "") === p.id,
      ).length;
      const swarmCount = swarms().filter(
        (s: any) => (s.project ?? s.project_id ?? "") === p.id,
      ).length;
      const worktreeCount = worktreeCounts()[p.id] ?? 0;
      const taskCount = swarmTasks().filter(
        (t: any) => (t.project ?? t.project_id ?? "") === p.id,
      ).length;
      const completedTasks = swarmTasks().filter(
        (t: any) =>
          (t.project ?? t.project_id ?? "") === p.id &&
          (t.status === "completed" || t.status === "done"),
      ).length;
      return {
        ...p,
        score,
        agentCount,
        swarmCount,
        worktreeCount,
        taskCount,
        completedTasks,
      };
    }),
  );

  const activeSwarms = createMemo(() =>
    swarms().filter(
      (s: any) =>
        s.status === "active" || s.status === "running" || !s.status,
    ),
  );

  const subtitle = createMemo(() => {
    const parts: string[] = [];
    parts.push(
      `${projectList().length} project${projectList().length !== 1 ? "s" : ""}`,
    );
    parts.push(
      `${activeSwarms().length} active swarm${activeSwarms().length !== 1 ? "s" : ""}`,
    );
    parts.push(
      `${registryAgents().length} agent${registryAgents().length !== 1 ? "s" : ""}`,
    );
    parts.push(
      `${inferenceProviders().length} inference provider${inferenceProviders().length !== 1 ? "s" : ""}`,
    );
    return parts.join(" \u00b7 ");
  });

  function swarmProgress(swarmId: string): {
    percent: number;
    done: number;
    total: number;
  } {
    const tasks = swarmTasks().filter(
      (t: any) => (t.swarmId ?? t.swarm_id ?? "") === swarmId,
    );
    if (tasks.length === 0) return { percent: 0, done: 0, total: 0 };
    const done = tasks.filter(
      (t: any) => t.status === "completed" || t.status === "done",
    ).length;
    return {
      percent: Math.round((done / tasks.length) * 100),
      done,
      total: tasks.length,
    };
  }

  function swarmProjectId(swarm: any): string {
    return swarm.project ?? swarm.project_id ?? "";
  }

  function swarmProjectName(swarm: any): string {
    const pid = swarmProjectId(swarm);
    const proj = projects().find((p) => p.id === pid);
    return proj?.name ?? pid ?? "unassigned";
  }

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
    <div
      class="flex h-full flex-col overflow-auto"
      style={{ background: "var(--bg-base)" }}
    >
      {/* Padding container */}
      <div class="flex flex-col gap-6 p-6">
        {/* Header: Title + action buttons */}
        <div>
          <div class="flex items-start justify-between">
            <div>
              <h2
                style={{
                  "font-size": "22px",
                  "font-weight": "700",
                  color: "#e5e7eb",
                  "line-height": "1.3",
                }}
              >
                Control Plane
              </h2>
              <p
                style={{
                  "font-size": "13px",
                  color: "var(--text-faint)",
                  "margin-top": "4px",
                }}
              >
                {subtitle()}
              </p>
            </div>

            <div class="flex items-center gap-3">
              <button
                style={{
                  background: "#164e6380",
                  color: "var(--accent-hover)",
                  border: "none",
                  "border-radius": "8px",
                  padding: "6px 14px",
                  "font-size": "13px",
                  "font-weight": "600",
                  cursor: "pointer",
                }}
                onClick={() => setShowRegisterForm(true)}
              >
                + Add Project
              </button>
              <button
                style={{
                  background: "transparent",
                  color: "#e5e7eb",
                  border: "1px solid #374151",
                  "border-radius": "8px",
                  padding: "6px 14px",
                  "font-size": "13px",
                  "font-weight": "600",
                  cursor: "pointer",
                }}
                onClick={() => setSwarmInitDialogOpen(true)}
              >
                New Swarm
              </button>
            </div>
          </div>
        </div>

        {/* Inline register form (shown when + Add Project is clicked) */}
        <Show when={showRegisterForm() && projectList().length > 0}>
          <form
            class="flex items-center gap-3"
            onSubmit={handleRegister}
          >
            <input
              type="text"
              placeholder="/path/to/project"
              value={newPath()}
              onInput={(e) => setNewPath(e.currentTarget.value)}
              autofocus
              style={{
                flex: "1",
                background: "var(--bg-surface)",
                border: "1px solid #374151",
                "border-radius": "8px",
                padding: "8px 12px",
                "font-size": "13px",
                color: "#e5e7eb",
                outline: "none",
              }}
              onFocus={(e) =>
                (e.currentTarget.style.borderColor = "#22d3ee80")
              }
              onBlur={(e) =>
                (e.currentTarget.style.borderColor = "#374151")
              }
            />
            <button
              type="submit"
              disabled={registering() || !newPath().trim()}
              style={{
                background: "#164e6380",
                color: "var(--accent-hover)",
                border: "none",
                "border-radius": "8px",
                padding: "8px 16px",
                "font-size": "13px",
                "font-weight": "600",
                cursor: "pointer",
                opacity: registering() || !newPath().trim() ? "0.5" : "1",
              }}
            >
              {registering() ? "Registering..." : "Register"}
            </button>
            <button
              type="button"
              style={{
                background: "transparent",
                color: "var(--text-faint)",
                border: "1px solid #374151",
                "border-radius": "8px",
                padding: "8px 12px",
                "font-size": "13px",
                cursor: "pointer",
              }}
              onClick={() => {
                setShowRegisterForm(false);
                setNewPath("");
              }}
            >
              Cancel
            </button>
          </form>
        </Show>

        {/* Main content */}
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
          {/* Project cards grid */}
          <div class="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
            <For each={projectList()}>
              {(project) => {
                const badge = () => healthBadge(project.score);
                const hasSwarms = () => project.swarmCount > 0;
                return (
                  <button
                    class="group flex flex-col text-left transition-all"
                    style={{
                      background: "var(--bg-surface)",
                      border: hasSwarms()
                        ? "1px solid #22d3ee40"
                        : "1px solid var(--border-subtle)",
                      "border-radius": "12px",
                      padding: "16px",
                      cursor: "pointer",
                    }}
                    onMouseEnter={(e) => {
                      e.currentTarget.style.borderColor = "color-mix(in srgb, var(--accent-hover) 40%, transparent)";
                    }}
                    onMouseLeave={(e) => {
                      e.currentTarget.style.borderColor = hasSwarms()
                        ? "#22d3ee40"
                        : "#1f2937";
                    }}
                    onClick={() =>
                      navigate({ page: "project", projectId: project.id })
                    }
                  >
                    {/* Top row: folder icon + name + health badge */}
                    <div class="flex w-full items-center justify-between">
                      <div class="flex items-center gap-2.5 overflow-hidden">
                        <FolderIcon active={hasSwarms()} />
                        <span
                          style={{
                            "font-size": "16px",
                            "font-weight": "700",
                            color: "#e5e7eb",
                            "white-space": "nowrap",
                            overflow: "hidden",
                            "text-overflow": "ellipsis",
                          }}
                        >
                          {project.name}
                        </span>
                      </div>
                      <span
                        style={{
                          background: badge().bg,
                          color: badge().text,
                          "font-size": "12px",
                          "font-weight": "700",
                          "border-radius": "9999px",
                          padding: "2px 8px",
                          "flex-shrink": "0",
                          "margin-left": "8px",
                        }}
                      >
                        {project.score}
                      </span>
                    </div>

                    {/* Stats row */}
                    <div
                      class="mt-2 flex items-center gap-3"
                      style={{ "font-size": "12px" }}
                    >
                      <span style={{ color: "var(--text-faint)" }}>
                        {project.worktreeCount} worktree
                        {project.worktreeCount !== 1 ? "s" : ""}
                      </span>
                      <span style={{ color: "var(--accent-hover)" }}>
                        {project.swarmCount} swarm
                        {project.swarmCount !== 1 ? "s" : ""}
                      </span>
                    </div>

                    {/* Agent count */}
                    <Show when={project.agentCount > 0}>
                      <div
                        class="mt-1"
                        style={{ "font-size": "12px", color: "var(--text-muted)" }}
                      >
                        {project.agentCount} agent
                        {project.agentCount !== 1 ? "s" : ""}
                      </div>
                    </Show>

                    {/* Health bar */}
                    <div
                      class="mt-3 w-full overflow-hidden"
                      style={{
                        height: "6px",
                        "border-radius": "6px",
                        background: "var(--bg-elevated)",
                      }}
                    >
                      <div
                        style={{
                          height: "100%",
                          width: `${project.score}%`,
                          "border-radius": "6px",
                          background: badge().text,
                          transition: "width 500ms ease",
                        }}
                      />
                    </div>

                    {/* Last activity */}
                    <div
                      class="mt-2"
                      style={{ "font-size": "11px", color: "var(--text-dim)" }}
                    >
                      Last: {timeAgo((project as any).lastActivity)}
                    </div>
                  </button>
                );
              }}
            </For>
          </div>

          {/* Active Swarms section */}
          <Show when={activeSwarms().length > 0}>
            <div class="mt-2">
              <h3
                style={{
                  "font-size": "14px",
                  "font-weight": "700",
                  color: "#e5e7eb",
                  "margin-bottom": "12px",
                  "text-transform": "uppercase",
                  "letter-spacing": "0.05em",
                }}
              >
                Active Swarms
              </h3>
              <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
                <For each={activeSwarms()}>
                  {(swarm) => {
                    const prog = () =>
                      swarmProgress(swarm.id ?? swarm.swarm_id ?? "");
                    const topology = () =>
                      swarm.topology ?? swarm.swarm_topology ?? "hier";
                    const topoShort = () => {
                      const t = topology();
                      if (t.startsWith("hier")) return "hier";
                      if (t.startsWith("mesh")) return "mesh";
                      if (t.startsWith("star")) return "star";
                      return t.slice(0, 4);
                    };
                    return (
                      <button
                        class="flex flex-col text-left transition-all"
                        style={{
                          background: "var(--bg-surface)",
                          border: "1px solid var(--border-subtle)",
                          "border-radius": "12px",
                          padding: "16px",
                          cursor: "pointer",
                        }}
                        onMouseEnter={(e) => {
                          e.currentTarget.style.borderColor = "#22d3ee40";
                        }}
                        onMouseLeave={(e) => {
                          e.currentTarget.style.borderColor = "#1f2937";
                        }}
                        onClick={() => {
                          const pid = swarmProjectId(swarm);
                          if (pid) {
                            navigate({ page: "project", projectId: pid });
                          }
                        }}
                      >
                        {/* Swarm name + progress% + topology */}
                        <div class="flex w-full items-center justify-between">
                          <span
                            style={{
                              "font-family": "'JetBrains Mono', monospace",
                              "font-size": "14px",
                              "font-weight": "700",
                              color: "#e5e7eb",
                              "white-space": "nowrap",
                              overflow: "hidden",
                              "text-overflow": "ellipsis",
                            }}
                          >
                            {swarm.name ?? swarm.swarm_name ?? "unnamed"}
                          </span>
                          <div class="flex items-center gap-2">
                            <span
                              style={{
                                "font-size": "12px",
                                "font-weight": "600",
                                color: "var(--accent-hover)",
                              }}
                            >
                              {prog().percent}%
                            </span>
                            <span
                              style={{
                                "font-size": "11px",
                                color: "var(--text-faint)",
                                "font-family":
                                  "'JetBrains Mono', monospace",
                              }}
                            >
                              {topoShort()}
                            </span>
                          </div>
                        </div>

                        {/* Project name + task count */}
                        <div
                          class="mt-1"
                          style={{ "font-size": "12px", color: "var(--text-faint)" }}
                        >
                          {swarmProjectName(swarm)} &middot;{" "}
                          {prog().done}/{prog().total} tasks
                        </div>

                        {/* Progress bar */}
                        <div
                          class="mt-3 w-full overflow-hidden"
                          style={{
                            height: "6px",
                            "border-radius": "6px",
                            background: "var(--bg-elevated)",
                          }}
                        >
                          <div
                            style={{
                              height: "100%",
                              width: `${prog().percent}%`,
                              "border-radius": "6px",
                              background: "#22d3ee",
                              transition: "width 500ms ease",
                            }}
                          />
                        </div>
                      </button>
                    );
                  }}
                </For>
              </div>
            </div>
          </Show>
        </Show>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Empty state — shown when no projects are registered
// ---------------------------------------------------------------------------

const EmptyState: Component<{
  onRegister: (e: Event) => void;
  path: () => string;
  setPath: (v: string) => void;
  registering: () => boolean;
}> = (props) => (
  <div class="flex flex-1 flex-col items-center justify-center gap-6 text-center">
    <div
      style={{
        "border-radius": "9999px",
        border: "1px solid var(--border-subtle)",
        background: "var(--bg-surface)",
        padding: "16px",
      }}
    >
      <svg
        width="32"
        height="32"
        viewBox="0 0 24 24"
        fill="none"
        stroke="#6b7280"
        stroke-width="1.5"
      >
        <path d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
      </svg>
    </div>
    <div>
      <p
        style={{
          "font-size": "18px",
          "font-weight": "700",
          color: "#e5e7eb",
        }}
      >
        No projects registered
      </p>
      <p
        style={{
          "font-size": "13px",
          color: "var(--text-faint)",
          "margin-top": "8px",
          "max-width": "28rem",
          "margin-left": "auto",
          "margin-right": "auto",
        }}
      >
        Register a project directory to start tracking its architecture,
        agents, and swarms.
      </p>
    </div>
    <form
      class="flex w-full max-w-lg gap-2"
      onSubmit={props.onRegister}
    >
      <input
        type="text"
        placeholder="/path/to/project"
        value={props.path()}
        onInput={(e) => props.setPath(e.currentTarget.value)}
        style={{
          flex: "1",
          background: "var(--bg-surface)",
          border: "1px solid #374151",
          "border-radius": "8px",
          padding: "10px 14px",
          "font-size": "13px",
          color: "#e5e7eb",
          outline: "none",
        }}
        onFocus={(e) =>
          (e.currentTarget.style.borderColor = "#22d3ee80")
        }
        onBlur={(e) =>
          (e.currentTarget.style.borderColor = "#374151")
        }
      />
      <button
        type="submit"
        disabled={props.registering() || !props.path().trim()}
        style={{
          background: "#164e6380",
          color: "var(--accent-hover)",
          border: "none",
          "border-radius": "8px",
          padding: "10px 20px",
          "font-size": "13px",
          "font-weight": "600",
          cursor: "pointer",
          opacity:
            props.registering() || !props.path().trim() ? "0.5" : "1",
        }}
      >
        {props.registering() ? "Registering..." : "Register"}
      </button>
    </form>
  </div>
);

export default ControlPlane;
