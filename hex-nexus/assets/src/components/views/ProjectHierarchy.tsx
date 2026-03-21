import { type Component, For, Show, createMemo, createSignal } from "solid-js";
import type { WorktreeInfo, CommitInfo } from "../../stores/git";

// ── Status colors ───────────────────────────────────────

const badgeStyle: Record<string, string> = {
  online: "bg-emerald-900/40 text-emerald-400",
  active: "bg-emerald-900/40 text-emerald-400",
  busy:   "bg-yellow-900/40 text-yellow-400",
  idle:   "bg-yellow-900/40 text-yellow-400",
  stale:  "bg-orange-900/40 text-orange-400",
  dead:   "bg-red-900/40 text-red-400",
  offline:"bg-red-900/40 text-red-400",
};

const dotColor: Record<string, string> = {
  online: "bg-emerald-500",
  active: "bg-emerald-500",
  busy:   "bg-yellow-500",
  idle:   "bg-yellow-500",
  stale:  "bg-orange-500",
  dead:   "bg-red-500",
  offline:"bg-red-500",
};

const modelColor: Record<string, string> = {
  qwen:    "text-blue-400",
  claude:  "text-purple-400",
  ollama:  "text-blue-400",
  sonnet:  "text-purple-400",
  opus:    "text-pink-400",
  haiku:   "text-cyan-400",
  default: "text-gray-400",
};

const getModelColor = (model: string): string => {
  const m = model.toLowerCase();
  for (const [key, color] of Object.entries(modelColor)) {
    if (m.includes(key)) return color;
  }
  return modelColor.default;
};

// ── Helpers ─────────────────────────────────────────────

const relativeTime = (epoch: number): string => {
  const diff = Math.floor(Date.now() / 1000) - epoch;
  if (diff < 60) return "now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
};

const truncate = (s: string, max: number): string =>
  s.length > max ? s.slice(0, max - 1) + "\u2026" : s;

// ── Props ───────────────────────────────────────────────

interface ProjectHierarchyProps {
  projectId: string;
  agents: any[];
  worktrees: WorktreeInfo[];
  commits: CommitInfo[];
}

// ── Component ───────────────────────────────────────────

const ProjectHierarchy: Component<ProjectHierarchyProps> = (props) => {
  // First agent starts expanded
  const [expanded, setExpanded] = createSignal<Set<string>>(
    new Set(props.agents.length > 0 ? [props.agents[0]?.name ?? "0"] : [])
  );

  const toggle = (key: string) =>
    setExpanded((s) => {
      const n = new Set(s);
      n.has(key) ? n.delete(key) : n.add(key);
      return n;
    });

  // Group commits by branch — match commit to worktree branch
  const commitsForBranch = (branch: string): CommitInfo[] => {
    if (!branch || branch === "(detached)") return [];
    // Show all commits — in a real impl we'd filter by branch ancestry
    // For now show the most recent commits (they're from the active branch)
    return props.commits.slice(0, 5);
  };

  // Each agent "owns" worktrees — for now all worktrees go under the primary agent
  // In future: match by agent.worktree_path or SpacetimeDB assignment
  const agentWorktrees = (_agent: any, index: number): WorktreeInfo[] => {
    if (index === 0) return props.worktrees.filter((wt) => !wt.isBare);
    return [];
  };

  return (
    <>
      <h2 class="mb-3 mt-8 text-[10px] font-semibold uppercase tracking-wider text-gray-500">
        Agents · Worktrees · Commits
      </h2>

      <div class="space-y-3">
        <For each={props.agents}>
          {(agent, idx) => {
            const name = () => agent.name ?? agent.agent_name ?? "unnamed";
            const host = () => agent.host ?? agent.hostname ?? "local";
            const status = () => agent.status ?? "idle";
            const model = () => agent.model ?? "--";
            const key = () => name();
            const isOpen = () => expanded().has(key());
            const wts = () => agentWorktrees(agent, idx());

            return (
              <div class="rounded-xl border border-gray-800 bg-[#111827] overflow-hidden">
                {/* Agent header */}
                <button
                  class="flex w-full items-center gap-2.5 px-4 py-3 text-left transition-colors hover:bg-gray-800/40"
                  onClick={() => toggle(key())}
                >
                  <svg
                    class={`h-3 w-3 shrink-0 text-gray-500 transition-transform duration-150 ${isOpen() ? "rotate-90" : ""}`}
                    viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"
                    stroke-linecap="round" stroke-linejoin="round"
                  >
                    <polyline points="9 18 15 12 9 6" />
                  </svg>

                  <span class={`h-2 w-2 shrink-0 rounded-full ${dotColor[status()] ?? "bg-gray-500"}`} />

                  <span class="text-[13px] font-semibold text-gray-200" style={{"font-family": "'JetBrains Mono', monospace"}}>
                    {name()}
                  </span>

                  <span class={`rounded-full px-2.5 py-0.5 text-[10px] font-semibold ${badgeStyle[status()] ?? "bg-gray-800 text-gray-400"}`}>
                    {status()}
                  </span>

                  <span class="ml-auto flex items-center gap-3 text-[11px]">
                    <span class="text-gray-500">{host()}</span>
                    <span class={`${getModelColor(model())}`} style={{"font-family": "'JetBrains Mono', monospace"}}>
                      {model()}
                    </span>
                  </span>
                </button>

                {/* Expanded: worktrees + commits */}
                <Show when={isOpen()}>
                  <div class="border-t border-gray-800/60 px-4 pb-3 pt-2">
                    <Show
                      when={wts().length > 0}
                      fallback={<p class="pl-7 text-[11px] text-gray-600">No worktrees</p>}
                    >
                      <div class="space-y-3 pl-7">
                        <For each={wts()}>
                          {(wt) => (
                            <div>
                              {/* Worktree header */}
                              <div class="flex items-center gap-2">
                                <svg class="h-3.5 w-3.5 shrink-0 text-yellow-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                  <line x1="6" y1="3" x2="6" y2="15" />
                                  <circle cx="18" cy="6" r="3" />
                                  <circle cx="6" cy="18" r="3" />
                                  <path d="M18 9a9 9 0 0 1-9 9" />
                                </svg>
                                <span class="text-[12px] font-medium text-gray-200" style={{"font-family": "'JetBrains Mono', monospace"}}>
                                  {wt.branch || "(detached)"}
                                </span>
                                <Show when={wt.isMain}>
                                  <span class="rounded-full bg-emerald-900/40 px-2 py-0.5 text-[9px] font-semibold text-emerald-400">
                                    HEAD
                                  </span>
                                </Show>
                                <Show when={wt.commitCount != null && wt.commitCount > 0}>
                                  <span class="rounded-full bg-gray-800 px-2 py-0.5 text-[9px] font-medium text-gray-400">
                                    {wt.commitCount} ahead
                                  </span>
                                </Show>
                              </div>

                              {/* Commits under this worktree */}
                              <div class="mt-1.5 space-y-1 pl-5">
                                <For each={commitsForBranch(wt.branch)}>
                                  {(c) => (
                                    <div class="flex items-center gap-2 text-[11px]">
                                      <span class="font-mono text-blue-400">{c.shortSha}</span>
                                      <span class="flex-1 truncate text-gray-300">
                                        {truncate(c.message.split("\n")[0], 60)}
                                      </span>
                                      <span class="shrink-0 text-gray-600">{relativeTime(c.timestamp)}</span>
                                    </div>
                                  )}
                                </For>
                                <Show when={commitsForBranch(wt.branch).length === 0}>
                                  <p class="text-[10px] text-gray-600 italic">no recent commits</p>
                                </Show>
                              </div>
                            </div>
                          )}
                        </For>
                      </div>
                    </Show>
                  </div>
                </Show>
              </div>
            );
          }}
        </For>

        {/* Empty state */}
        <Show when={props.agents.length === 0}>
          <div class="rounded-xl border border-gray-800 bg-[#111827] px-4 py-8 text-center text-sm text-gray-500">
            No agents connected — start one with <code class="mx-1 rounded bg-gray-800 px-1.5 py-0.5 font-mono text-[11px] text-gray-400">hex nexus start</code>
          </div>
        </Show>
      </div>
    </>
  );
};

export default ProjectHierarchy;
