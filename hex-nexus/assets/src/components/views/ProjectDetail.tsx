import { type Component, Show, createMemo, createEffect, createSignal, onMount, onCleanup } from "solid-js";
import ProjectHierarchy from "./ProjectHierarchy";
import ProjectChatWidget from "../chat/ProjectChatWidget";
import BranchPicker from "../project/BranchPicker";
import DiffViewer from "../code/DiffViewer";
import { route } from "../../stores/router";
import { projects } from "../../stores/projects";
import { registryAgents } from "../../stores/connection";
import { healthData, fetchHealth } from "../../stores/health";
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
  if (score == null) return { letter: "--", color: "#9CA3AF", bg: "#1F2937" };
  if (score >= 90) return { letter: "Grade A", color: "#34D399", bg: "#065F46" };
  if (score >= 75) return { letter: "Grade B", color: "#34D399", bg: "#065F46" };
  if (score >= 60) return { letter: "Grade C", color: "#FBBF24", bg: "#422006" };
  return { letter: "Grade D", color: "#F87171", bg: "#7F1D1D" };
};

type DetailTab = "overview" | "changes";

const ProjectDetail: Component = () => {
  const [chatOpen, setChatOpen] = createSignal(false);
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
    if (!pid) return [];
    return registryAgents().filter((a: any) => {
      const agentProj = a.project ?? a.projectId ?? a.project_id ?? "";
      return agentProj === pid;
    });
  });

  function handleBranchChange(branch: string) {
    const pid = projectId();
    if (pid) {
      fetchGitLog(pid, project()?.path, branch, undefined, 10);
    }
  }

  onMount(async () => {
    const pid = projectId();
    const p = project();

    if (pid) {
      fetchAllGitData(pid, p?.path);
      subscribeGitEvents(pid);
    }
  });

  // Auto-fetch health whenever the active project changes
  createEffect(() => {
    const proj = project();
    if (proj?.path) {
      fetchHealth(proj.path);
    }
  });

  onCleanup(() => {
    unsubscribeGitEvents();
  });

  const grade = createMemo(() => healthGrade(health()?.health_score));

  return (
    <div class="flex flex-1 overflow-hidden">
      <div class="flex-1 overflow-auto" style={{ padding: "24px 32px" }}>
        {/* Header — project name + path + BranchPicker + grade badge */}
        <div class="mb-5 flex items-center gap-3">
          <h1
            class="text-[22px] font-bold"
            style={{ color: "#F3F4F6", "font-family": "Inter, sans-serif" }}
          >
            {project()?.name ?? projectId()}
          </h1>
          <span
            class="text-[11px]"
            style={{ color: "#6B7280", "font-family": "'JetBrains Mono', monospace" }}
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
        <div class="mb-4 flex items-center gap-0 border-b" style={{ "border-color": "#1F2937" }}>
          <button
            class="px-4 py-2 text-[11px] font-semibold uppercase transition-colors"
            style={{
              color: activeTab() === "overview" ? "#67E8F9" : "#6B7280",
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
              color: activeTab() === "changes" ? "#67E8F9" : "#6B7280",
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
            style={{ color: "#6B7280", "letter-spacing": "1.2px" }}
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

      {/* Chat panel (right side) */}
      <Show when={chatOpen()}>
        <ProjectChatWidget
          projectId={projectId()}
          onClose={() => setChatOpen(false)}
        />
      </Show>

      {/* Floating chat toggle button */}
      <Show when={!chatOpen()}>
        <button
          class="fixed bottom-20 right-6 z-50 rounded-full p-3 shadow-lg transition-transform hover:scale-105"
          style={{ background: "#0E7490" }}
          onClick={() => setChatOpen(true)}
          title="Open project chat"
        >
          <svg
            class="h-5 w-5"
            style={{ color: "#FFFFFF" }}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z" />
          </svg>
        </button>
      </Show>
    </div>
  );
};

export default ProjectDetail;
