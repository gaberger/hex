import { type Component, createMemo, onMount, Show, For } from "solid-js";
import { route } from "../../stores/router";
import { projects } from "../../stores/projects";
import { healthData, healthLoading, fetchHealth } from "../../stores/health";
import {
  gitStatus,
  gitWorktrees,
  gitLog,
  gitLoading,
  fetchAllGitData,
  type WorktreeInfo,
  type CommitInfo,
} from "../../stores/git";

// ── Status colors ─────────────────────────────────────

const worktreeStatusColor = (wt: WorktreeInfo): string => {
  if (wt.isMain) return "bg-gray-800 text-gray-400 border border-gray-700";
  if (wt.commitCount && wt.commitCount > 0)
    return "bg-green-900/30 text-green-400 border border-green-500/30";
  return "bg-cyan-900/30 text-cyan-400 border border-cyan-500/30";
};

const worktreeBorderColor = (wt: WorktreeInfo): string => {
  if (wt.isMain) return "border-gray-800";
  if (wt.commitCount && wt.commitCount > 0) return "border-[#4ade8040]";
  return "border-[#22d3ee40]";
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

/** Git commit icon (lucide git-commit-horizontal) */
const GitCommitIcon: Component = () => (
  <svg
    class="h-3.5 w-3.5 shrink-0"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <circle cx="12" cy="12" r="3" />
    <line x1="3" y1="12" x2="9" y2="12" />
    <line x1="15" y1="12" x2="21" y2="12" />
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

  // Real worktree data from git store (replaces MOCK_WORKTREES)
  const worktrees = createMemo(() => {
    const wts = gitWorktrees();
    // Filter out bare worktrees
    return wts.filter((wt) => !wt.isBare);
  });

  const recentCommits = createMemo(() => {
    const log = gitLog();
    return log?.commits ?? [];
  });

  const status = gitStatus;

  const handleAnalyze = () => {
    const p = project();
    if (p?.path) fetchHealth(p.path);
  };

  onMount(() => {
    const pid = projectId();
    if (pid) {
      // Fetch git data on mount
      fetchAllGitData(pid);
    }

    // Auto-fetch health on mount if we have a project path and no data yet
    const p = project();
    if (p?.path && !health()) {
      fetchHealth(p.path);
    }
  });

  /** Format epoch seconds to relative time */
  const relativeTime = (epoch: number): string => {
    const now = Math.floor(Date.now() / 1000);
    const diff = now - epoch;
    if (diff < 60) return "just now";
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  };

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
          <div class="mt-1 flex items-center gap-3">
            <p class="text-xs text-gray-500" style={{ "font-family": "'JetBrains Mono', monospace" }}>
              {project()?.path ?? ""}
            </p>
            {/* Git status badge */}
            <Show when={status()}>
              <span class="inline-flex items-center gap-1.5 rounded-full bg-gray-800 px-2.5 py-0.5 text-[10px] font-medium text-gray-300 border border-gray-700">
                <GitBranchIcon />
                <span style={{ color: "#22d3ee" }}>{status()!.branch}</span>
                <Show when={status()!.dirtyCount + status()!.stagedCount + status()!.untrackedCount > 0}>
                  <span class="text-yellow-400">
                    {status()!.dirtyCount + status()!.stagedCount + status()!.untrackedCount} changed
                  </span>
                </Show>
                <Show when={status()!.ahead > 0}>
                  <span class="text-green-400">{"\u2191"}{status()!.ahead}</span>
                </Show>
                <Show when={status()!.behind > 0}>
                  <span class="text-red-400">{"\u2193"}{status()!.behind}</span>
                </Show>
              </span>
            </Show>
          </div>
        </div>
        <div class="flex items-center gap-2">
          <button
            class="rounded-lg border border-gray-700 bg-gray-800 px-3.5 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:bg-gray-700 hover:text-gray-100"
            onClick={handleAnalyze}
            disabled={loading()}
          >
            {loading() ? "Analyzing..." : "Analyze"}
          </button>
          <button
            class="rounded-lg border border-gray-700 bg-gray-800 px-3.5 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:bg-gray-700 hover:text-gray-100"
            onClick={() => { const pid = projectId(); if (pid) fetchAllGitData(pid); }}
            disabled={gitLoading()}
          >
            {gitLoading() ? "Refreshing..." : "Refresh Git"}
          </button>
        </div>
      </div>

      {/* Stats bar */}
      <div class="mb-8 rounded-xl border border-gray-800 bg-[#111827] p-5">
        <div class="grid grid-cols-5 gap-4 text-center">
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
              {worktrees().length || "--"}
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
          <div>
            <div
              class="text-2xl font-bold"
              style={{ color: "#a78bfa", "font-family": "'JetBrains Mono', monospace" }}
            >
              {status()?.branch ?? "--"}
            </div>
            <div class="mt-1 text-[11px] font-medium uppercase tracking-wider text-gray-500">
              Branch
            </div>
          </div>
        </div>
      </div>

      {/* Active Worktrees */}
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-gray-400">
        Worktrees
      </h2>
      <Show
        when={worktrees().length > 0}
        fallback={
          <div class="mb-8 rounded-xl border border-gray-800 bg-[#111827] px-4 py-6 text-center text-sm text-gray-500">
            {gitLoading() ? "Loading worktrees..." : "No worktrees found — is this a git repository?"}
          </div>
        }
      >
        <div class="mb-8 space-y-3">
          <For each={worktrees()}>
            {(wt) => (
              <div
                class={`rounded-xl border bg-[#111827] px-4 py-3.5 ${worktreeBorderColor(wt)}`}
              >
                <div class="flex items-center justify-between">
                  <div class="flex items-center gap-2 text-gray-300">
                    <GitBranchIcon />
                    <span
                      class="text-sm font-bold text-gray-200"
                      style={{ "font-family": "'JetBrains Mono', monospace" }}
                    >
                      {wt.branch || "(detached)"}
                    </span>
                  </div>
                  <span
                    class={`rounded-full px-2.5 py-0.5 text-[10px] font-semibold ${worktreeStatusColor(wt)}`}
                  >
                    {wt.isMain ? "main" : wt.commitCount ? "active" : "clean"}
                  </span>
                </div>
                <div class="mt-2 flex items-center gap-4 text-xs text-gray-500">
                  <span
                    class="font-mono text-[10px] text-gray-600"
                    title={wt.path}
                  >
                    {wt.path.length > 50 ? "..." + wt.path.slice(-47) : wt.path}
                  </span>
                  <Show when={wt.commitCount != null && wt.commitCount > 0}>
                    <span>
                      <span class="text-gray-400">{wt.commitCount}</span>{" "}
                      {wt.commitCount === 1 ? "commit ahead" : "commits ahead"}
                    </span>
                  </Show>
                  <Show when={wt.headSha}>
                    <span class="font-mono text-gray-600">
                      {wt.headSha.slice(0, 7)}
                    </span>
                  </Show>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* Recent Commits */}
      <h2 class="mb-3 text-sm font-semibold uppercase tracking-wider text-gray-400">
        Recent Commits
      </h2>
      <Show
        when={recentCommits().length > 0}
        fallback={
          <div class="rounded-xl border border-gray-800 bg-[#111827] px-4 py-6 text-center text-sm text-gray-500">
            {gitLoading() ? "Loading commits..." : "No commits found"}
          </div>
        }
      >
        <div class="space-y-1">
          <For each={recentCommits()}>
            {(commit: CommitInfo) => (
              <div class="flex items-start gap-3 rounded-lg border border-gray-800/50 bg-[#111827] px-4 py-2.5 hover:border-gray-700 transition-colors">
                <div class="mt-0.5 text-gray-600">
                  <GitCommitIcon />
                </div>
                <div class="min-w-0 flex-1">
                  <div class="flex items-baseline gap-2">
                    <span
                      class="text-xs font-mono text-cyan-500"
                      style={{ "font-family": "'JetBrains Mono', monospace" }}
                    >
                      {commit.shortSha}
                    </span>
                    <span class="truncate text-sm text-gray-300">
                      {commit.message.split("\n")[0]}
                    </span>
                  </div>
                  <div class="mt-0.5 text-[10px] text-gray-600">
                    {commit.authorName} · {relativeTime(commit.timestamp)}
                  </div>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default ProjectDetail;
