import { type Component, createMemo, onMount, Show, For } from "solid-js";
import { route } from "../../stores/router";
import { projects } from "../../stores/projects";
import { healthData, healthLoading, fetchHealth } from "../../stores/health";

// TODO: Replace mock worktrees with SpacetimeDB `worktree` table subscription
// when the table is available (ADR TBD).
interface Worktree {
  branch: string;
  status: "active" | "in-progress" | "merged" | "stale";
  layer?: string;
  agent?: string;
  commits?: number;
}

const MOCK_WORKTREES: Worktree[] = [
  {
    branch: "feat/auth/secondary-adapter",
    status: "active",
    layer: "secondary",
    agent: "hex-coder",
    commits: 3,
  },
  {
    branch: "feat/auth/primary-adapter",
    status: "in-progress",
    layer: "primary",
    agent: "hex-coder",
    commits: 1,
  },
  {
    branch: "feat/auth/domain-ports",
    status: "merged",
  },
];

const statusColors: Record<string, string> = {
  active: "bg-green-900/30 text-green-400 border border-green-500/30",
  "in-progress": "bg-cyan-900/30 text-cyan-400 border border-cyan-500/30",
  merged: "bg-gray-800 text-gray-400 border border-gray-700",
  stale: "bg-yellow-900/30 text-yellow-400 border border-yellow-500/30",
};

const cardBorderColors: Record<string, string> = {
  active: "border-[#4ade8040]",
  "in-progress": "border-[#22d3ee40]",
  merged: "border-gray-800",
  stale: "border-yellow-800/40",
};

/** Git branch icon (lucide git-branch) */
const GitBranchIcon: Component = () => (
  <svg
    class="h-4 w-4 shrink-0"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <line x1="6" y1="3" x2="6" y2="15" />
    <circle cx="18" cy="6" r="3" />
    <circle cx="6" cy="18" r="3" />
    <path d="M18 9a9 9 0 0 1-9 9" />
  </svg>
);

const ProjectDetail: Component = () => {
  const projectId = createMemo(() => {
    const r = route();
    return (r as any).projectId ?? "";
  });

  const project = createMemo(() =>
    projects().find((p) => p.id === projectId())
  );

  const health = healthData;
  const loading = healthLoading;

  // TODO: Replace with real worktree data from SpacetimeDB
  const worktrees = () => MOCK_WORKTREES;

  const handleAnalyze = () => {
    const p = project();
    if (p?.path) fetchHealth(p.path);
  };

  onMount(() => {
    // Auto-fetch health on mount if we have a project path and no data yet
    const p = project();
    if (p?.path && !health()) {
      fetchHealth(p.path);
    }
  });

  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="mb-6 flex items-start justify-between">
        <div>
          <h1
            class="text-2xl font-bold text-gray-100"
            style={{ "font-family": "'JetBrains Mono', monospace" }}
          >
            {project()?.name ?? projectId()}
          </h1>
          <p class="mt-1 text-xs text-gray-500" style={{ "font-family": "'JetBrains Mono', monospace" }}>
            {project()?.path ?? ""}
          </p>
        </div>
        <div class="flex items-center gap-2">
          <button
            class="rounded-lg border border-gray-700 bg-gray-800 px-3.5 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:bg-gray-700 hover:text-gray-100"
            onClick={handleAnalyze}
            disabled={loading()}
          >
            {loading() ? "Analyzing..." : "Analyze"}
          </button>
          <button class="rounded-lg border border-cyan-800/50 bg-cyan-900/20 px-3.5 py-1.5 text-xs font-medium text-cyan-400 transition-colors hover:bg-cyan-900/40">
            New Worktree
          </button>
        </div>
      </div>

      {/* Stats bar */}
      <div class="mb-8 rounded-xl border border-gray-800 bg-[#111827] p-5">
        <div class="grid grid-cols-4 gap-4 text-center">
          <div>
            <div
              class="text-2xl font-bold"
              style={{ color: "#4ade80", "font-family": "'JetBrains Mono', monospace" }}
            >
              {health()?.health_score ?? "--"}
            </div>
            <div class="mt-1 text-[11px] font-medium uppercase tracking-wider text-gray-500">
              Health
            </div>
          </div>
          <div>
            <div
              class="text-2xl font-bold"
              style={{ color: "#e5e7eb", "font-family": "'JetBrains Mono', monospace" }}
            >
              {health()?.file_count ?? "--"}
            </div>
            <div class="mt-1 text-[11px] font-medium uppercase tracking-wider text-gray-500">
              Files
            </div>
          </div>
          <div>
            <div
              class="text-2xl font-bold"
              style={{ color: "#22d3ee", "font-family": "'JetBrains Mono', monospace" }}
            >
              {worktrees().length}
            </div>
            <div class="mt-1 text-[11px] font-medium uppercase tracking-wider text-gray-500">
              Worktrees
            </div>
          </div>
          <div>
            <div
              class="text-2xl font-bold"
              style={{ color: "#f87171", "font-family": "'JetBrains Mono', monospace" }}
            >
              {health()?.violation_count ?? "--"}
            </div>
            <div class="mt-1 text-[11px] font-medium uppercase tracking-wider text-gray-500">
              Violations
            </div>
          </div>
        </div>
      </div>

      {/* Active Worktrees */}
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-gray-400">
        Active Worktrees
      </h2>
      <div class="space-y-3">
        <For each={worktrees()}>
          {(wt) => (
            <div
              class={`rounded-xl border bg-[#111827] px-4 py-3.5 ${cardBorderColors[wt.status] ?? "border-gray-800"}`}
            >
              <div class="flex items-center justify-between">
                <div class="flex items-center gap-2 text-gray-300">
                  <GitBranchIcon />
                  <span
                    class="text-sm font-bold text-gray-200"
                    style={{ "font-family": "'JetBrains Mono', monospace" }}
                  >
                    {wt.branch}
                  </span>
                </div>
                <span
                  class={`rounded-full px-2.5 py-0.5 text-[10px] font-semibold ${statusColors[wt.status] ?? ""}`}
                >
                  {wt.status}
                </span>
              </div>
              <Show when={wt.layer || wt.agent || wt.commits}>
                <div class="mt-2 flex items-center gap-4 text-xs text-gray-500">
                  <Show when={wt.layer}>
                    <span>
                      Layer: <span class="text-gray-400">{wt.layer}</span>
                    </span>
                  </Show>
                  <Show when={wt.agent}>
                    <span>
                      Agent:{" "}
                      <span style={{ color: "#22d3ee" }}>{wt.agent}</span>
                    </span>
                  </Show>
                  <Show when={wt.commits}>
                    <span>
                      <span class="text-gray-400">{wt.commits}</span>{" "}
                      {wt.commits === 1 ? "commit" : "commits"}
                    </span>
                  </Show>
                </div>
              </Show>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default ProjectDetail;
