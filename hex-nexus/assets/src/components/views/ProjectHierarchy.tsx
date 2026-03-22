import { type Component, For, Show, createSignal } from "solid-js";
import type { WorktreeInfo, CommitInfo } from "../../stores/git";

// ── Status colors ───────────────────────────────────────

const badgeStyle: Record<string, { color: string; bg: string }> = {
  online: { color: "#34D399", bg: "#064E3B" },
  active: { color: "#34D399", bg: "#064E3B" },
  busy:   { color: "#FBBF24", bg: "#422006" },
  idle:   { color: "#FBBF24", bg: "#422006" },
  stale:  { color: "#FB923C", bg: "#431407" },
  dead:   { color: "#F87171", bg: "#7F1D1D" },
  offline:{ color: "#F87171", bg: "#7F1D1D" },
};

const dotColor: Record<string, string> = {
  online: "#10B981",
  active: "#10B981",
  busy:   "#FBBF24",
  idle:   "#FBBF24",
  stale:  "#FB923C",
  dead:   "#EF4444",
  offline:"#EF4444",
};

const modelColor: Record<string, string> = {
  qwen:    "#60A5FA",
  claude:  "#C084FC",
  ollama:  "#60A5FA",
  sonnet:  "#C084FC",
  opus:    "#F472B6",
  haiku:   "#22D3EE",
  default: "#9CA3AF",
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
  const [expanded, setExpanded] = createSignal<Set<string>>(
    new Set(props.agents.length > 0 ? [props.agents[0]?.name ?? "0"] : [])
  );

  const toggle = (key: string) =>
    setExpanded((s) => {
      const n = new Set(s);
      n.has(key) ? n.delete(key) : n.add(key);
      return n;
    });

  const commitsForBranch = (branch: string): CommitInfo[] => {
    if (!branch || branch === "(detached)") return [];
    return props.commits.slice(0, 5);
  };

  const agentWorktrees = (_agent: any, index: number): WorktreeInfo[] => {
    if (index === 0) return props.worktrees.filter((wt) => !wt.isBare);
    return [];
  };

  return (
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
          const badge = () => badgeStyle[status()] ?? { color: "var(--text-muted)", bg: "var(--bg-elevated)" };

          return (
            <div
              class="overflow-hidden rounded-[10px] bg-[var(--bg-surface)] border border-[var(--border-subtle)]"
            >
              {/* Agent header row */}
              <button
                class="flex w-full items-center gap-2.5 px-4 py-3 text-left transition-colors hover:bg-gray-800/40"
                onClick={() => toggle(key())}
              >
                <span
                  class="shrink-0 text-[10px] text-[var(--text-faint)]"
                >
                  {isOpen() ? "\u25BC" : "\u25B6"}
                </span>

                <span
                  class="h-2 w-2 shrink-0 rounded-full"
                  style={{ background: dotColor[status()] ?? "var(--text-faint)" }}
                />

                <span
                  class="font-mono text-[13px] font-semibold text-[var(--text-body)]"
                >
                  {name()}
                </span>

                <span
                  class="rounded-full px-2.5 py-0.5 text-[10px] font-semibold"
                  style={{ color: badge().color, background: badge().bg }}
                >
                  {status()}
                </span>

                {/* Spacer */}
                <div class="flex-1" />

                <span class="text-[11px] text-[var(--text-muted)]">
                  {host()}
                </span>
                <span
                  class="text-[11px]"
                  style={{ color: getModelColor(model()), "font-family": "'JetBrains Mono', monospace" }}
                >
                  {model()}
                </span>
              </button>

              {/* Expanded: worktrees + commits */}
              <Show when={isOpen()}>
                <div class="pb-3 pl-10 pr-4">
                  <Show
                    when={wts().length > 0}
                    fallback={
                      <p class="text-[11px] text-[var(--text-dim)]">
                        No worktrees
                      </p>
                    }
                  >
                    <div class="space-y-2">
                      <For each={wts()}>
                        {(wt) => (
                          <div
                            class="rounded-md bg-[var(--bg-base)] px-3 py-2"
                          >
                            {/* Worktree header */}
                            <div class="flex items-center gap-2">
                              <span
                                class="text-[12px]"
                                classList={{ "text-status-active": wt.isMain, "text-status-warning": !wt.isMain }}
                              >
                                &#x2387;
                              </span>
                              <span
                                class="font-mono text-[12px] text-[var(--text-body)]"
                                classList={{ "font-semibold": wt.isMain, "font-normal": !wt.isMain }}
                              >
                                {wt.branch || "(detached)"}
                              </span>
                              <Show when={wt.isMain}>
                                <span
                                  class="rounded-full bg-[#064E3B] px-2 py-0.5 text-[9px] font-semibold text-status-active"
                                >
                                  HEAD
                                </span>
                              </Show>
                              <Show when={wt.commitCount != null && wt.commitCount > 0}>
                                <span
                                  class="rounded-full bg-[var(--bg-elevated)] px-2 py-0.5 text-[9px] font-medium text-[var(--text-muted)]"
                                >
                                  {wt.commitCount} ahead
                                </span>
                              </Show>
                            </div>

                            {/* Commits under this worktree */}
                            <div class="mt-1 space-y-0.5 pl-5">
                              <For each={commitsForBranch(wt.branch)}>
                                {(c) => (
                                  <div class="flex items-center gap-2 py-0.5">
                                    <span
                                      class="shrink-0 font-mono text-[11px] text-hex-domain"
                                    >
                                      {c.shortSha}
                                    </span>
                                    <span
                                      class="min-w-0 flex-1 truncate text-[11px] text-[var(--text-secondary)]"
                                    >
                                      {truncate(c.message.split("\n")[0], 60)}
                                    </span>
                                    <span
                                      class="shrink-0 text-[10px] text-[var(--text-faint)]"
                                    >
                                      {relativeTime(c.timestamp)}
                                    </span>
                                  </div>
                                )}
                              </For>
                              <Show when={commitsForBranch(wt.branch).length === 0}>
                                <p class="text-[10px] italic text-[var(--text-dim)]">
                                  no recent commits
                                </p>
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
        <div
          class="rounded-[10px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-8 text-center text-sm text-[var(--text-faint)]"
        >
          No agents connected — start one with{" "}
          <code
            class="mx-1 rounded bg-[var(--bg-elevated)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--text-muted)]"
          >
            hex nexus start
          </code>
        </div>
      </Show>
    </div>
  );
};

export default ProjectHierarchy;
