import { type Component, For, Show, createMemo, createSignal } from "solid-js";
import type { WorktreeInfo, CommitInfo } from "../../stores/git";

// ── Status badge colors ─────────────────────────────────

const statusColors: Record<string, string> = {
  online: "bg-emerald-900/30 text-emerald-400 border border-emerald-500/30",
  active: "bg-emerald-900/30 text-emerald-400 border border-emerald-500/30",
  busy: "bg-yellow-900/30 text-yellow-400 border border-yellow-500/30",
  stale: "bg-orange-900/30 text-orange-400 border border-orange-500/30",
  dead: "bg-red-900/30 text-red-400 border border-red-500/30",
  offline: "bg-red-900/30 text-red-400 border border-red-500/30",
};

const statusDot: Record<string, string> = {
  online: "bg-emerald-500",
  active: "bg-emerald-500",
  busy: "bg-yellow-500",
  stale: "bg-orange-500",
  dead: "bg-red-500",
  offline: "bg-red-500",
};

// ── Icons ────────────────────────────────────────────────

const ChevronIcon: Component<{ open: boolean }> = (props) => (
  <svg
    class={`h-4 w-4 shrink-0 text-gray-500 transition-transform ${props.open ? "rotate-90" : ""}`}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <polyline points="9 18 15 12 9 6" />
  </svg>
);

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

const GitBranchIcon: Component = () => (
  <svg
    class="h-3.5 w-3.5 shrink-0"
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

// ── Helpers ──────────────────────────────────────────────

const relativeTime = (epoch: number): string => {
  const diff = Math.floor(Date.now() / 1000) - epoch;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
};

const truncate = (s: string, max: number): string =>
  s.length > max ? s.slice(0, max - 1) + "\u2026" : s;

// ── Props ────────────────────────────────────────────────

interface ProjectHierarchyProps {
  projectId: string;
  agents: any[];
  worktrees: WorktreeInfo[];
  commits: CommitInfo[];
}

// ── Component ────────────────────────────────────────────

const ProjectHierarchy: Component<ProjectHierarchyProps> = (props) => {
  const [expandedAgents, setExpandedAgents] = createSignal<Set<string>>(new Set());

  const toggleAgent = (id: string) => {
    setExpandedAgents((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  /** Match worktrees to an agent by branch name or path containing the agent name. */
  const worktreesForAgent = (agent: any): WorktreeInfo[] => {
    const name = (agent.name ?? agent.agent_name ?? "").toLowerCase();
    if (!name) return [];
    return props.worktrees.filter((wt) => {
      const branch = (wt.branch ?? "").toLowerCase();
      const path = (wt.path ?? "").toLowerCase();
      return branch.includes(name) || path.includes(name);
    });
  };

  /** Match commits to a worktree by branch name appearing in the commit message or by headSha prefix. */
  const commitsForWorktree = (wt: WorktreeInfo): CommitInfo[] => {
    return props.commits.filter((c) => {
      if (wt.headSha && c.sha.startsWith(wt.headSha.slice(0, 7))) return true;
      // Show commits whose SHA prefix matches the worktree head lineage (simple heuristic)
      return false;
    }).slice(0, 5);
  };

  /** Unmatched worktrees (not claimed by any agent). */
  const unmatchedWorktrees = createMemo(() => {
    const claimed = new Set<string>();
    for (const agent of props.agents) {
      for (const wt of worktreesForAgent(agent)) {
        claimed.add(wt.path);
      }
    }
    return props.worktrees.filter((wt) => !claimed.has(wt.path));
  });

  return (
    <div class="rounded-xl border border-gray-800 bg-[#111827] p-5">
      {/* Project header */}
      <h2
        class="mb-4 text-lg font-bold text-gray-100"
        style={{ "font-family": "'JetBrains Mono', monospace" }}
      >
        {props.projectId}
      </h2>

      {/* Agents */}
      <Show
        when={props.agents.length > 0}
        fallback={<p class="text-sm text-gray-500">No agents assigned</p>}
      >
        <div class="space-y-2">
          <For each={props.agents}>
            {(agent) => {
              const id = () => agent.id ?? agent.name ?? agent.agent_name ?? "";
              const name = () => agent.name ?? agent.agent_name ?? "unnamed";
              const host = () => agent.host ?? agent.hostname ?? "--";
              const status = () => agent.status ?? "offline";
              const model = () => agent.model ?? "--";
              const isOpen = () => expandedAgents().has(id());
              const agentWts = () => worktreesForAgent(agent);

              return (
                <div class="rounded-lg border border-gray-700/50">
                  {/* Agent row */}
                  <button
                    class="flex w-full items-center gap-2 px-3 py-2.5 text-left hover:bg-gray-800/50 transition-colors rounded-lg"
                    onClick={() => toggleAgent(id())}
                  >
                    <ChevronIcon open={isOpen()} />
                    <span class={`h-2 w-2 shrink-0 rounded-full ${statusDot[status()] ?? "bg-gray-500"}`} />
                    <span
                      class="text-xs font-semibold text-gray-200"
                      style={{ "font-family": "'JetBrains Mono', monospace" }}
                    >
                      {name()}
                    </span>
                    <span class={`ml-1 rounded-full px-2 py-0.5 text-[10px] font-medium ${statusColors[status()] ?? "bg-gray-800 text-gray-400"}`}>
                      {status()}
                    </span>
                    <span class="ml-auto text-[10px] text-gray-500">
                      {host()} &middot; {model()}
                    </span>
                  </button>

                  {/* Expanded: worktrees */}
                  <Show when={isOpen()}>
                    <div class="border-t border-gray-800 px-3 pb-3 pt-2">
                      <Show
                        when={agentWts().length > 0}
                        fallback={<p class="pl-6 text-[11px] text-gray-600">No matched worktrees</p>}
                      >
                        <div class="space-y-2 pl-6">
                          <For each={agentWts()}>
                            {(wt) => {
                              const wtCommits = () => commitsForWorktree(wt);
                              return (
                                <div>
                                  <div class="flex items-center gap-1.5 text-gray-400">
                                    <GitBranchIcon />
                                    <span class="text-xs font-medium text-gray-300" style={{ "font-family": "'JetBrains Mono', monospace" }}>
                                      {wt.branch || "(detached)"}
                                    </span>
                                    <span class="ml-1 text-[10px] text-gray-600 font-mono">
                                      {wt.path.length > 40 ? "\u2026" + wt.path.slice(-37) : wt.path}
                                    </span>
                                  </div>
                                  <Show when={wtCommits().length > 0}>
                                    <div class="mt-1 space-y-0.5 pl-5">
                                      <For each={wtCommits()}>
                                        {(c) => (
                                          <div class="flex items-center gap-2 text-[11px]">
                                            <GitCommitIcon />
                                            <span class="font-mono text-blue-400">{c.shortSha}</span>
                                            <span class="text-gray-300">{truncate(c.message.split("\n")[0], 60)}</span>
                                            <span class="ml-auto text-gray-500">{relativeTime(c.timestamp)}</span>
                                          </div>
                                        )}
                                      </For>
                                    </div>
                                  </Show>
                                </div>
                              );
                            }}
                          </For>
                        </div>
                      </Show>
                    </div>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>

      {/* Unmatched worktrees */}
      <Show when={unmatchedWorktrees().length > 0}>
        <h3 class="mt-4 mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
          Unassigned Worktrees
        </h3>
        <div class="space-y-1 pl-2">
          <For each={unmatchedWorktrees()}>
            {(wt) => (
              <div class="flex items-center gap-1.5 text-gray-500">
                <GitBranchIcon />
                <span class="text-xs text-gray-400 font-mono">{wt.branch || "(detached)"}</span>
                <span class="text-[10px] text-gray-600 font-mono">{wt.headSha?.slice(0, 7)}</span>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default ProjectHierarchy;
