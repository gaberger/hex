import { type Component, Show, createMemo, createEffect, createSignal, onCleanup } from "solid-js";
import ProjectHierarchy from "./ProjectHierarchy";
// TODO: ProjectChatWidget for inline project chat
import BranchPicker from "../project/BranchPicker";
import DiffViewer from "../code/DiffViewer";
import { route, navigate } from "../../stores/router";
import { projects, unregisterProject, archiveProject, deleteProject } from "../../stores/projects";
import { registryAgents } from "../../stores/connection";
import { healthData } from "../../stores/health";
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

type DetailTab = "overview" | "changes";

const ProjectDetail: Component = () => {
  // const [chatOpen, setChatOpen] = createSignal(false); // TODO: inline chat
  const [activeTab, setActiveTab] = createSignal<DetailTab>("overview");
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [confirmDelete, setConfirmDelete] = createSignal(false);

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
    }
  });

  // Health is fetched on-demand (e.g. from Health page), not on every project nav

  onCleanup(() => {
    unsubscribeGitEvents();
  });

  const grade = createMemo(() => healthGrade(health()?.health_score));

  return (
    <div class="flex-1 overflow-auto p-6">
        {/* Header — project name + path + BranchPicker + grade badge */}
        <div class="mb-5 flex items-center gap-3">
          <h1
            class="text-[22px] font-bold"
            style={{ color: "var(--text-primary)", "font-family": "Inter, sans-serif" }}
          >
            {project()?.name ?? projectId()}
          </h1>
          <span
            class="text-[11px]"
            style={{ color: "var(--text-faint)", "font-family": "'JetBrains Mono', monospace" }}
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
          >
            {grade().letter}
          </span>

          {/* Project actions menu */}
          <div class="relative">
            <button
              class="rounded-md p-2 transition-colors hover:bg-gray-800"
              style={{ color: "var(--text-muted)" }}
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
        <div class="mb-4 flex items-center gap-0 border-b" style={{ "border-color": "var(--border-subtle)" }}>
          <button
            class="px-4 py-2 text-[11px] font-semibold uppercase transition-colors"
            style={{
              color: activeTab() === "overview" ? "var(--accent-hover)" : "var(--text-faint)",
              "border-bottom": activeTab() === "overview" ? "2px solid #0E7490" : "2px solid transparent",
              "letter-spacing": "1px",
            }}
            onClick={() => setActiveTab("overview")}
          >
            Overview
          </button>
          <button
            class="px-4 py-2 text-[11px] font-semibold uppercase transition-colors"
            style={{
              color: activeTab() === "changes" ? "var(--accent-hover)" : "var(--text-faint)",
              "border-bottom": activeTab() === "changes" ? "2px solid #0E7490" : "2px solid transparent",
              "letter-spacing": "1px",
            }}
            onClick={() => setActiveTab("changes")}
          >
            Changes
          </button>
        </div>

        {/* Tab content: Overview */}
        <Show when={activeTab() === "overview"}>
          {/* Section label */}
          <h2
            class="mb-4 text-[10px] font-semibold uppercase"
            style={{ color: "var(--text-faint)", "letter-spacing": "1.2px" }}
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
        </Show>

        {/* Tab content: Changes (DiffViewer) */}
        <Show when={activeTab() === "changes"}>
          <DiffViewer
            projectId={projectId()}
            projectPath={project()?.path}
          />
        </Show>
      </div>
  );
};

export default ProjectDetail;
