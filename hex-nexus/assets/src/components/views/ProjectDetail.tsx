import { type Component, Show, createMemo, createEffect, createSignal, onCleanup, For } from "solid-js";
import ProjectHierarchy from "./ProjectHierarchy";
// TODO: ProjectChatWidget for inline project chat
import BranchPicker from "../project/BranchPicker";
import FingerprintPane from "../project/FingerprintPane";
import DiffViewer from "../code/DiffViewer";
import { route, navigate } from "../../stores/router";
import { projects, unregisterProject, archiveProject, deleteProject } from "../../stores/projects";
import { registryAgents, swarmTasks, swarms } from "../../stores/connection";
import { healthData } from "../../stores/health";
import { workplans, fetchWorkplans } from "../../stores/workplan";
import {
  gitWorktrees,
  gitLog,
  fetchAllGitData,
  fetchGitLog,
  subscribeGitEvents,
  unsubscribeGitEvents,
} from "../../stores/git";

/** Health grade from numeric score */
const healthGrade = (score: number | undefined): { letter: string; color: string; bg: string } => {
  if (score == null) return { letter: "--", color: "var(--text-muted)", bg: "var(--bg-elevated)" };
  if (score >= 90) return { letter: "Grade A", color: "#34D399", bg: "#065F46" };
  if (score >= 75) return { letter: "Grade B", color: "#34D399", bg: "#065F46" };
  if (score >= 60) return { letter: "Grade C", color: "#FBBF24", bg: "#422006" };
  return { letter: "Grade D", color: "#F87171", bg: "#7F1D1D" };
};

type DetailTab = "overview" | "changes" | "fingerprint";

// ── Inbox types ──────────────────────────────────────────────────────────────

interface InboxItem {
  id: number;
  priority: number;
  message: string;
  from_agent: string;
  created_at: string;
  acknowledged: boolean;
}

function priorityBadgeClass(priority: number): string {
  if (priority === 0) return "bg-red-900/50 text-red-300";
  if (priority === 1) return "bg-orange-900/50 text-orange-300";
  if (priority === 2) return "bg-yellow-900/50 text-yellow-300";
  return "bg-gray-800 text-gray-400";
}

function relativeTime(isoString: string): string {
  try {
    const diff = Date.now() - new Date(isoString).getTime();
    const secs = Math.floor(diff / 1000);
    if (secs < 60) return `${secs}s ago`;
    const mins = Math.floor(secs / 60);
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
  } catch {
    return isoString;
  }
}

// ── Component ────────────────────────────────────────────────────────────────

const ProjectDetail: Component = () => {
  // const [chatOpen, setChatOpen] = createSignal(false); // TODO: inline chat
  const [activeTab, setActiveTab] = createSignal<DetailTab>("overview");
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [confirmDelete, setConfirmDelete] = createSignal(false);
  const [expandedTaskId, setExpandedTaskId] = createSignal<string | null>(null);

  // ── Inbox state ────────────────────────────────────────
  const [inboxItems, setInboxItems] = createSignal<InboxItem[]>([]);

  async function fetchInbox(pid: string) {
    try {
      const url = pid
        ? `/api/inbox?project_id=${encodeURIComponent(pid)}`
        : "/api/inbox";
      const res = await fetch(url);
      if (!res.ok) return;
      const data = await res.json();
      const items: InboxItem[] = Array.isArray(data) ? data : [];
      setInboxItems(items.filter((n) => !n.acknowledged));
    } catch {
      // silently ignore fetch errors
    }
  }

  async function handleAck(id: number) {
    // Optimistic update — remove from list immediately
    setInboxItems((prev) => prev.filter((n) => n.id !== id));
    try {
      await fetch(`/api/inbox/${id}/ack`, { method: "POST" });
    } catch {
      // ignore — item already removed from local list
    }
  }

  async function handleUnregister() {
    setMenuOpen(false);
    const pid = projectId();
    if (pid && await unregisterProject(pid)) {
      navigate({ page: "control-plane" });
    }
  }

  async function handleArchive() {
    setMenuOpen(false);
    const pid = projectId();
    if (pid && await archiveProject(pid)) {
      navigate({ page: "control-plane" });
    }
  }

  async function handleDelete() {
    setConfirmDelete(false);
    setMenuOpen(false);
    const pid = projectId();
    if (pid && await deleteProject(pid)) {
      navigate({ page: "control-plane" });
    }
  }

  const projectId = createMemo(() => {
    const r = route();
    return (r as any).projectId ?? "";
  });

  const project = createMemo(() =>
    projects().find((p) => p.id === projectId())
  );

  const health = healthData;

  // Real worktree data from git store
  const worktrees = createMemo(() => {
    const wts = gitWorktrees();
    return wts.filter((wt) => !wt.isBare);
  });

  const recentCommits = createMemo(() => {
    const log = gitLog();
    return log?.commits ?? [];
  });

  const projectAgents = createMemo(() => {
    const pid = projectId();
    const allAgents = registryAgents();
    if (!pid) return [];
    return allAgents.filter((a: any) => {
      // Primary: match by project_id (SpacetimeDB project ID like "hex-intf-1xq8wun")
      const agentProjId = a.projectId ?? a.project_id ?? "";
      if (agentProjId && agentProjId === pid) return true;
      // Fallback: match by project_dir path suffix
      const agentDir = a.projectDir ?? a.project_dir ?? "";
      return agentDir && (agentDir === pid || agentDir.endsWith("/" + pid));
    });
  });

  // Swarm tasks for this project (via swarms scoped to project)
  const projectSwarmIds = createMemo(() => {
    const pid = projectId();
    return swarms()
      .filter((s: any) => (s.project_id ?? s.projectId ?? "") === pid)
      .map((s: any) => s.id ?? s.swarm_id ?? "");
  });

  const projectTasks = createMemo(() => {
    const ids = new Set(projectSwarmIds());
    if (ids.size === 0) return swarmTasks(); // fallback: show all if no project-scoped swarms
    return swarmTasks().filter((t: any) => ids.has(t.swarm_id ?? t.swarmId ?? ""));
  });

  // Active workplan for this project
  const activeWorkplanForProject = createMemo(() => {
    const pid = projectId();
    const all = workplans();
    // Prefer active/pending, fall back to most recent
    return (
      all.find((w) => (w as any).project_id === pid && (w.status === "active" || w.status === "pending")) ??
      all.find((w) => w.status === "active" || w.status === "pending") ??
      all[0] ??
      null
    );
  });

  // Fetch workplans when overview tab is active
  createEffect(() => {
    if (activeTab() === "overview") {
      fetchWorkplans();
    }
  });

  function handleBranchChange(branch: string) {
    const pid = projectId();
    if (pid) {
      fetchGitLog(pid, project()?.path, branch, undefined, 10);
    }
  }

  // Re-fetch git data whenever the active project changes.
  // Must be a createEffect (not onMount) because Solid's Switch/Match
  // does NOT remount ProjectDetail when navigating between projects —
  // the Match condition (route().page === "project") stays true.
  createEffect(() => {
    const pid = projectId();
    const p = project();

    if (pid) {
      fetchAllGitData(pid, p?.path);
      subscribeGitEvents(pid);
      fetchInbox(pid);
    }
  });

  // Poll inbox every 10 seconds
  const inboxPollInterval = setInterval(() => {
    const pid = projectId();
    if (pid) fetchInbox(pid);
  }, 10_000);

  // Health is fetched on-demand (e.g. from Health page), not on every project nav

  onCleanup(() => {
    unsubscribeGitEvents();
    clearInterval(inboxPollInterval);
  });

  const isHealthStub = createMemo(() => {
    const d = health();
    if (!d) return false;
    const raw = d as any;
    return raw.ast_is_stub || raw.astIsStub || (d.health_score === 100 && (raw.file_count ?? 0) === 0);
  });
  const grade = createMemo(() => isHealthStub() ? { letter: "No Analysis", color: "#9ca3af", bg: "#1f2937" } : healthGrade(health()?.health_score));

  return (
    <div class="flex-1 overflow-auto p-6">
        {/* Header — project name + path + BranchPicker + grade badge */}
        <div class="mb-5 flex items-center gap-3">
          <h1
            class="text-[22px] font-bold text-[var(--text-primary)] font-[Inter,sans-serif]"
          >
            {project()?.name ?? projectId()}
          </h1>
          <span
            class="font-mono text-[11px] text-[var(--text-faint)]"
          >
            {project()?.path ?? ""}
          </span>
          <div class="flex-1" />
          <BranchPicker
            projectId={projectId()}
            projectPath={project()?.path}
            onBranchChange={handleBranchChange}
          />
          <span
            class="rounded-md px-3.5 py-1.5 text-[11px] font-semibold"
            style={{ color: grade().color, background: grade().bg }}
            title={isHealthStub() ? "Run `hex analyze .` to get real scores" : undefined}
          >
            {grade().letter}
          </span>

          {/* Project actions menu */}
          <div class="relative">
            <button
              class="rounded-md p-2 text-[var(--text-muted)] transition-colors hover:bg-gray-800"
              onClick={() => setMenuOpen(!menuOpen())}
              title="Project actions"
            >
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="currentColor">
                <circle cx="12" cy="5" r="1.5" />
                <circle cx="12" cy="12" r="1.5" />
                <circle cx="12" cy="19" r="1.5" />
              </svg>
            </button>

            <Show when={menuOpen()}>
              <div class="absolute right-0 top-10 z-50 min-w-[180px] rounded-lg border border-gray-700 bg-gray-900 py-1 shadow-xl">
                <button
                  class="flex w-full flex-col gap-0.5 px-3 py-2 text-left transition-colors hover:bg-gray-800"
                  onClick={handleUnregister}
                >
                  <span class="text-xs font-medium text-gray-200">Unregister</span>
                  <span class="text-[10px] text-gray-500">Remove from nexus registry</span>
                </button>
                <div class="my-1 border-t border-gray-800" />
                <button
                  class="flex w-full flex-col gap-0.5 px-3 py-2 text-left transition-colors hover:bg-gray-800"
                  onClick={handleArchive}
                >
                  <span class="text-xs font-medium text-yellow-400">Archive</span>
                  <span class="text-[10px] text-gray-500">Remove config, keep source files</span>
                </button>
                <button
                  class="flex w-full flex-col gap-0.5 px-3 py-2 text-left transition-colors hover:bg-gray-800"
                  onClick={() => { setMenuOpen(false); setConfirmDelete(true); }}
                >
                  <span class="text-xs font-medium text-red-400">Delete from disk</span>
                  <span class="text-[10px] text-gray-500">Permanently remove all files</span>
                </button>
              </div>
              <div class="fixed inset-0 z-40" onClick={() => setMenuOpen(false)} />
            </Show>
          </div>
        </div>

        {/* Delete confirmation dialog */}
        <Show when={confirmDelete()}>
          <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
            <div class="mx-4 max-w-md rounded-xl border border-red-900/50 bg-gray-950 p-6 shadow-2xl">
              <div class="mb-4 flex items-center gap-3">
                <div class="flex h-10 w-10 items-center justify-center rounded-full bg-red-900/30">
                  <svg class="h-5 w-5 text-red-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M12 9v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                </div>
                <div>
                  <h3 class="text-sm font-semibold text-gray-100">Delete project permanently?</h3>
                  <p class="text-xs text-gray-400">This cannot be undone</p>
                </div>
              </div>
              <div class="mb-5 rounded-lg border border-gray-800 bg-gray-900 p-3">
                <p class="text-sm font-medium text-gray-200">{project()?.name}</p>
                <p class="mt-1 truncate font-mono text-[11px] text-red-300">{project()?.path}</p>
                <p class="mt-2 text-[11px] text-gray-500">All files at this path will be permanently deleted.</p>
              </div>
              <div class="flex justify-end gap-2">
                <button
                  class="rounded-lg border border-gray-700 px-4 py-2 text-xs text-gray-300 transition-colors hover:bg-gray-800"
                  onClick={() => setConfirmDelete(false)}
                >
                  Cancel
                </button>
                <button
                  class="rounded-lg bg-red-600 px-4 py-2 text-xs font-medium text-white transition-colors hover:bg-red-500"
                  onClick={handleDelete}
                >
                  Delete permanently
                </button>
              </div>
            </div>
          </div>
        </Show>

        {/* Tab bar: Overview | Changes */}
        <div class="mb-4 flex items-center gap-0 border-b border-[var(--border-subtle)]">
          <button
            class="px-4 py-2 text-[11px] font-semibold uppercase tracking-wide transition-colors border-b-2"
            classList={{
              "border-cyan-700 text-[var(--accent-hover)]": activeTab() === "overview",
              "border-transparent text-[var(--text-faint)]": activeTab() !== "overview",
            }}
            onClick={() => setActiveTab("overview")}
          >
            Overview
          </button>
          <button
            class="px-4 py-2 text-[11px] font-semibold uppercase tracking-wide transition-colors border-b-2"
            classList={{
              "border-cyan-700 text-[var(--accent-hover)]": activeTab() === "changes",
              "border-transparent text-[var(--text-faint)]": activeTab() !== "changes",
            }}
            onClick={() => setActiveTab("changes")}
          >
            Changes
          </button>
          <button
            class="px-4 py-2 text-[11px] font-semibold uppercase tracking-wide transition-colors border-b-2"
            classList={{
              "border-cyan-700 text-[var(--accent-hover)]": activeTab() === "fingerprint",
              "border-transparent text-[var(--text-faint)]": activeTab() !== "fingerprint",
            }}
            onClick={() => setActiveTab("fingerprint")}
          >
            Fingerprint
          </button>
        </div>

        {/* Tab content: Overview */}
        <Show when={activeTab() === "overview"}>
          {/* Active Workplan banner */}
          <Show when={activeWorkplanForProject()}>
            {(wp) => {
              const adrId = () => (wp() as any).adr ?? (wp() as any).related_adrs?.[0] ?? null;
              return (
                <div class="mb-5 rounded-lg border border-gray-800 bg-gray-900/60 px-4 py-3">
                  <div class="flex items-center gap-2 flex-wrap">
                    <span class="text-[10px] font-semibold uppercase tracking-wider text-gray-500">Active Workplan</span>
                    <span class="text-xs font-medium text-gray-200 truncate flex-1">{wp().feature}</span>
                    <Show when={adrId()}>
                      <button
                        class="shrink-0 rounded bg-indigo-900/50 px-1.5 py-0.5 font-mono text-xs text-indigo-300 cursor-pointer hover:bg-indigo-800/50 transition-colors"
                        onClick={() => navigate({ page: "project-adr-detail", projectId: projectId(), adrId: adrId()! })}
                        title={`View ${adrId()}`}
                      >
                        {adrId()}
                      </button>
                    </Show>
                    <span class={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${
                      wp().status === "active" ? "bg-cyan-900/40 text-cyan-400"
                      : wp().status === "completed" ? "bg-green-900/40 text-green-400"
                      : wp().status === "failed" ? "bg-red-900/40 text-red-400"
                      : "bg-gray-800 text-gray-400"
                    }`}>{wp().status}</span>
                  </div>
                  <Show when={wp().currentPhase}>
                    <p class="mt-1 text-[10px] text-gray-500">Phase: {wp().currentPhase}</p>
                  </Show>
                </div>
              );
            }}
          </Show>

          {/* Swarm Tasks section */}
          <Show when={projectTasks().length > 0}>
            <h2 class="mb-2 text-[10px] font-semibold uppercase tracking-[1.2px] text-[var(--text-faint)]">
              Swarm Tasks
            </h2>
            <div class="mb-5 space-y-1">
              <For each={projectTasks()}>
                {(task) => {
                  const tid = () => task.id ?? task.task_id ?? "";
                  const taskStatus = () => task.status ?? "pending";
                  const assignee = () => task.assigned_to ?? task.agent_id ?? "";
                  const isExpanded = () => expandedTaskId() === tid();

                  const statusBadgeClass = () => {
                    switch (taskStatus()) {
                      case "in_progress": return "bg-blue-900/40 text-blue-400";
                      case "completed":   return "bg-green-900/40 text-green-400";
                      case "failed":      return "bg-red-900/40 text-red-400";
                      default:            return "bg-gray-800 text-gray-400";
                    }
                  };

                  return (
                    <div class="rounded-lg border border-gray-800 bg-gray-900/50 overflow-hidden">
                      {/* Row */}
                      <button
                        class="flex w-full items-center gap-2 px-3 py-2 text-left text-xs transition-colors hover:bg-gray-800/40"
                        onClick={() => setExpandedTaskId(isExpanded() ? null : tid())}
                      >
                        <span class="shrink-0 text-[10px] text-gray-500 w-3">
                          {isExpanded() ? "▼" : "▶"}
                        </span>
                        <span class="flex-1 truncate text-gray-100">{task.title ?? "Untitled"}</span>
                        <span class={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${statusBadgeClass()}`}>
                          {taskStatus()}
                        </span>
                      </button>

                      {/* Expanded detail panel */}
                      <Show when={isExpanded()}>
                        <div class="border-t border-gray-800/70 bg-gray-950/60 px-4 py-3 space-y-1.5 text-[11px]">
                          <Show when={task.description ?? task.title}>
                            <p class="text-gray-300 leading-relaxed">{task.description ?? task.title}</p>
                          </Show>
                          <div class="flex flex-wrap gap-x-6 gap-y-1 text-[10px] text-gray-500 pt-1">
                            <Show when={assignee()}>
                              <span>Agent: <span class="text-cyan-400 font-mono">{assignee()}</span></span>
                            </Show>
                            <Show when={task.created_at ?? task.createdAt}>
                              <span>Created: <span class="text-gray-400">{new Date(task.created_at ?? task.createdAt).toLocaleString()}</span></span>
                            </Show>
                            <Show when={task.completed_at ?? task.completedAt}>
                              <span>Completed: <span class="text-green-400">{new Date(task.completed_at ?? task.completedAt).toLocaleString()}</span></span>
                            </Show>
                          </div>
                          <Show when={task.result}>
                            <div class="mt-1 rounded bg-gray-900 px-2 py-1.5 font-mono text-[10px] text-gray-400 break-all">
                              {task.result}
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

          {/* Section label */}
          <h2
            class="mb-4 text-[10px] font-semibold uppercase tracking-[1.2px] text-[var(--text-faint)]"
          >
            Agents &middot; Worktrees &middot; Commits
          </h2>

          {/* Agent → Worktree → Commit hierarchy */}
          <ProjectHierarchy
            projectId={projectId()}
            agents={projectAgents()}
            worktrees={worktrees()}
            commits={recentCommits()}
          />

          {/* Agent Inbox panel (ADR-060) */}
          <div class="mt-6 rounded-xl border border-gray-700/50 bg-gray-900/60 p-4">
            <div class="mb-3 flex items-center justify-between">
              <h2 class="text-[10px] font-semibold uppercase tracking-[1.2px] text-[var(--text-faint)]">
                Agent Inbox
              </h2>
              <Show when={inboxItems().length > 0}>
                <span class="rounded-full bg-yellow-900/40 px-2 py-0.5 text-[10px] font-semibold text-yellow-300">
                  {inboxItems().length} pending
                </span>
              </Show>
            </div>

            <Show
              when={inboxItems().length > 0}
              fallback={
                <p class="py-4 text-center text-[12px] text-gray-500">
                  No pending notifications
                </p>
              }
            >
              <div class="divide-y divide-gray-800">
                <For each={inboxItems()}>
                  {(item) => (
                    <div class="flex items-start gap-3 py-2.5">
                      {/* Priority badge */}
                      <span
                        class={`mt-0.5 shrink-0 rounded px-1.5 py-0.5 text-[10px] font-bold ${priorityBadgeClass(item.priority)}`}
                      >
                        P{item.priority}
                      </span>

                      {/* Message + meta */}
                      <div class="min-w-0 flex-1">
                        <p class="truncate text-[12px] text-gray-200">{item.message}</p>
                        <p class="mt-0.5 text-[10px] text-gray-500">
                          <span class="font-mono">{item.from_agent}</span>
                          <span class="mx-1 opacity-40">&middot;</span>
                          {relativeTime(item.created_at)}
                        </p>
                      </div>

                      {/* Ack button */}
                      <button
                        class="shrink-0 rounded border border-gray-700 px-2 py-1 text-[10px] text-gray-400 transition-colors hover:border-gray-500 hover:bg-gray-800 hover:text-gray-200"
                        onClick={() => handleAck(item.id)}
                        title="Acknowledge notification"
                      >
                        Ack
                      </button>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </Show>

        {/* Tab content: Changes (DiffViewer) */}
        <Show when={activeTab() === "changes"}>
          <DiffViewer
            projectId={projectId()}
            projectPath={project()?.path}
          />
        </Show>

        {/* Tab content: Architecture Fingerprint */}
        <Show when={activeTab() === "fingerprint"}>
          <FingerprintPane
            projectId={projectId()}
            projectRoot={project()?.path}
          />
        </Show>
      </div>
  );
};

export default ProjectDetail;
