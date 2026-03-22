/**
 * ProjectHome.tsx — Main project landing page with four quadrants + governance pipeline.
 *
 * Layout:
 *   [GovernancePipeline banner]
 *   [Health Ring      ] [Active Swarms   ]
 *   [Recent Agents    ] [Recent Commits  ]
 *
 * Data sources: health store, SpacetimeDB subscriptions, REST API.
 */
import {
  Component,
  For,
  Show,
  createMemo,
  createResource,
  onMount,
} from "solid-js";
import {
  swarms,
  swarmTasks,
  swarmAgents,
} from "../../stores/connection";
import { healthData, healthLoading, fetchHealth } from "../../stores/health";
import { navigate } from "../../stores/router";
import { restClient } from "../../services/rest-client";
import GovernancePipeline from "./GovernancePipeline";
import { entityBelongsToProject } from "../../utils/project-match";
import { route } from "../../stores/router";

interface ProjectHomeProps {
  projectId: string;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function scoreColor(score: number): string {
  if (score >= 80) return "text-green-400";
  if (score >= 60) return "text-yellow-400";
  return "text-red-400";
}

function scoreBorderColor(score: number): string {
  if (score >= 80) return "border-green-500/40";
  if (score >= 60) return "border-yellow-500/40";
  return "border-red-500/40";
}

function statusDotClass(status: string | undefined): string {
  const s = (status ?? "").toLowerCase();
  if (s === "active" || s === "running" || s === "online") return "bg-green-400";
  if (s === "stale" || s === "idle") return "bg-yellow-400";
  return "bg-red-400";
}

function timeAgo(iso: string | undefined): string {
  if (!iso) return "";
  const diff = Date.now() - new Date(iso).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function shortHash(hash: string | undefined): string {
  return (hash ?? "").slice(0, 7);
}

function truncate(str: string, max: number): string {
  if (str.length <= max) return str;
  return str.slice(0, max - 1) + "\u2026";
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const ProjectHome: Component<ProjectHomeProps> = (props) => {
  const pid = () => props.projectId || (route() as any).projectId || "";

  // Fetch health data on mount
  onMount(() => {
    fetchHealth();
  });

  // ---- Top-right: Active Swarms ----
  const projectSwarms = createMemo(() =>
    swarms().filter(
      (s: any) => entityBelongsToProject(s, pid()),
    ),
  );

  const activeSwarms = createMemo(() =>
    projectSwarms().filter(
      (s: any) => s.status === "active" || s.status === "running" || !s.status,
    ),
  );

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

  // ---- Bottom-left: Recent Agents ----
  const projectAgents = createMemo(() =>
    swarmAgents().filter(
      (a: any) => entityBelongsToProject(a, pid()),
    ),
  );

  const recentAgents = createMemo(() =>
    projectAgents()
      .slice()
      .sort((a: any, b: any) => {
        const ta = a.registered_at ?? a.created_at ?? "";
        const tb = b.registered_at ?? b.created_at ?? "";
        return tb.localeCompare(ta);
      })
      .slice(0, 8),
  );

  // ---- Bottom-right: Recent Commits ----
  const [commits] = createResource(
    () => props.projectId,
    async (pid) => {
      try {
        const data = await restClient.get<any>(
          `/api/${pid}/git/log?limit=10`,
        );
        return Array.isArray(data) ? data : data?.commits ?? data?.data ?? [];
      } catch {
        return [];
      }
    },
  );

  return (
    <div class="flex h-full flex-col gap-4 overflow-auto p-6">
      {/* Governance Pipeline banner */}
      <GovernancePipeline projectId={props.projectId} />

      {/* Four quadrants */}
      <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
        {/* ── Top-left: Health Ring ────────────────────────────────────── */}
        <button
          class="flex flex-col items-center justify-center rounded-lg border border-gray-800 bg-gray-900 p-6 text-center cursor-pointer transition-colors hover:border-[var(--accent)]"
          onClick={() =>
            navigate({
              page: "project-health",
              projectId: props.projectId,
            })
          }
        >
          <Show
            when={!healthLoading() && healthData()}
            fallback={
              <div class="text-[var(--text-faint)] text-sm">
                {healthLoading() ? "Analyzing..." : "No health data"}
              </div>
            }
          >
            {(() => {
              const hd = healthData()!;
              const score = hd.health_score;
              return (
                <>
                  {/* Score ring — SVG circle */}
                  <div class="relative mb-3">
                    <svg width="96" height="96" viewBox="0 0 96 96">
                      {/* Background ring */}
                      <circle
                        cx="48"
                        cy="48"
                        r="40"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="6"
                        class="text-gray-800"
                      />
                      {/* Score arc */}
                      <circle
                        cx="48"
                        cy="48"
                        r="40"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="6"
                        stroke-linecap="round"
                        stroke-dasharray={`${(score / 100) * 251.3} 251.3`}
                        stroke-dashoffset="0"
                        transform="rotate(-90 48 48)"
                        class={scoreColor(score)}
                      />
                    </svg>
                    <span
                      class={`absolute inset-0 flex items-center justify-center text-2xl font-bold ${scoreColor(score)}`}
                    >
                      {score}
                    </span>
                  </div>
                  <span class="text-sm font-semibold text-[var(--text-body)]">
                    Architecture Health
                  </span>
                  <Show when={hd.violation_count > 0}>
                    <span class="mt-1 text-xs text-red-400">
                      {hd.violation_count} violation
                      {hd.violation_count !== 1 ? "s" : ""}
                    </span>
                  </Show>
                </>
              );
            })()}
          </Show>
        </button>

        {/* ── Top-right: Active Swarms ────────────────────────────────── */}
        <div class="flex flex-col rounded-lg border border-gray-800 bg-gray-900 p-4">
          <h3 class="mb-3 text-sm font-bold uppercase tracking-wide text-[var(--text-body)]">
            Active Swarms
          </h3>
          <Show
            when={activeSwarms().length > 0}
            fallback={
              <p class="text-sm text-[var(--text-faint)]">No active swarms</p>
            }
          >
            <div class="flex flex-col gap-3">
              <For each={activeSwarms()}>
                {(swarm) => {
                  const prog = () =>
                    swarmProgress(swarm.id ?? swarm.swarm_id ?? "");
                  const topology = () =>
                    swarm.topology ?? swarm.swarm_topology ?? "hier";
                  return (
                    <button
                      class="flex flex-col rounded-md border border-gray-700 bg-gray-950 p-3 text-left cursor-pointer transition-colors hover:border-[var(--accent)]"
                      onClick={() =>
                        navigate({
                          page: "project-swarm-detail",
                          projectId: props.projectId,
                          swarmId: swarm.id ?? swarm.swarm_id ?? "",
                        })
                      }
                    >
                      <div class="flex items-center justify-between">
                        <span class="truncate font-mono text-sm font-bold text-[var(--text-body)]">
                          {swarm.name ?? swarm.swarm_name ?? "unnamed"}
                        </span>
                        <span class="ml-2 shrink-0 rounded-full bg-gray-800 px-2 py-0.5 font-mono text-[11px] text-[var(--text-faint)]">
                          {topology()}
                        </span>
                      </div>
                      {/* Progress bar */}
                      <div class="mt-2 flex items-center gap-2">
                        <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-gray-800">
                          <div
                            class="h-full rounded-full bg-[var(--accent-hover)] transition-[width] duration-500"
                            style={{ width: `${prog().percent}%` }}
                          />
                        </div>
                        <span class="shrink-0 text-[11px] text-[var(--text-faint)]">
                          {prog().done}/{prog().total}
                        </span>
                      </div>
                    </button>
                  );
                }}
              </For>
            </div>
          </Show>
        </div>

        {/* ── Bottom-left: Recent Agents ──────────────────────────────── */}
        <div class="flex flex-col rounded-lg border border-gray-800 bg-gray-900 p-4">
          <h3 class="mb-3 text-sm font-bold uppercase tracking-wide text-[var(--text-body)]">
            Recent Agents
          </h3>
          <Show
            when={recentAgents().length > 0}
            fallback={
              <p class="text-sm text-[var(--text-faint)]">No agents</p>
            }
          >
            <div class="flex flex-col gap-2">
              <For each={recentAgents()}>
                {(agent) => (
                  <button
                    class="flex items-center gap-3 rounded-md border border-gray-700 bg-gray-950 px-3 py-2 text-left cursor-pointer transition-colors hover:border-[var(--accent)]"
                    onClick={() =>
                      navigate({
                        page: "project-agent-detail",
                        projectId: props.projectId,
                        agentId: agent.id ?? agent.agent_id ?? "",
                      })
                    }
                  >
                    {/* Status dot */}
                    <span
                      class={`h-2 w-2 shrink-0 rounded-full ${statusDotClass(agent.status)}`}
                    />
                    <div class="flex flex-1 flex-col overflow-hidden">
                      <span class="truncate text-sm font-semibold text-[var(--text-body)]">
                        {agent.name ?? agent.agent_name ?? "agent"}
                      </span>
                      <div class="flex items-center gap-2 text-[11px] text-[var(--text-faint)]">
                        <Show when={agent.role}>
                          <span>{agent.role}</span>
                        </Show>
                        <Show when={agent.worktree ?? agent.worktree_path}>
                          <span class="truncate font-mono">
                            {agent.worktree ?? agent.worktree_path}
                          </span>
                        </Show>
                      </div>
                    </div>
                  </button>
                )}
              </For>
            </div>
          </Show>
        </div>

        {/* ── Bottom-right: Recent Commits ────────────────────────────── */}
        <div class="flex flex-col rounded-lg border border-gray-800 bg-gray-900 p-4">
          <h3 class="mb-3 text-sm font-bold uppercase tracking-wide text-[var(--text-body)]">
            Recent Commits
          </h3>
          <Show
            when={!commits.loading}
            fallback={
              <p class="text-sm text-[var(--text-faint)]">Loading...</p>
            }
          >
            <Show
              when={(commits() ?? []).length > 0}
              fallback={
                <p class="text-sm text-[var(--text-faint)]">No commits</p>
              }
            >
              <div class="flex flex-col gap-1.5">
                <For each={commits() ?? []}>
                  {(commit: any) => (
                    <div class="flex items-start gap-2 rounded-md px-2 py-1.5 text-[13px]">
                      <span class="shrink-0 font-mono text-[11px] text-[var(--accent-hover)]">
                        {shortHash(commit.hash ?? commit.sha ?? commit.id)}
                      </span>
                      <span class="flex-1 truncate text-[var(--text-body)]">
                        {truncate(
                          commit.message ?? commit.subject ?? "",
                          60,
                        )}
                      </span>
                      <span class="shrink-0 text-[11px] text-[var(--text-faint)]">
                        {timeAgo(
                          commit.date ?? commit.timestamp ?? commit.authored_at,
                        )}
                      </span>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
};

export default ProjectHome;
