import { type Component, Show, createMemo, createEffect, createSignal, onCleanup } from "solid-js";
import ProjectHierarchy from "./ProjectHierarchy";
// TODO: ProjectChatWidget for inline project chat
import BranchPicker from "../project/BranchPicker";
import DiffViewer from "../code/DiffViewer";
import { route } from "../../stores/router";
import { projects } from "../../stores/projects";
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
    console.log(`[DEBUG] registryAgents count: ${allAgents.length}, projectId: ${pid}`, allAgents);
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
        </div>

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
