import { type Component, createMemo, createEffect, onMount, onCleanup } from "solid-js";
import ProjectHierarchy from "./ProjectHierarchy";
import { route } from "../../stores/router";
import { projects } from "../../stores/projects";
import { registryAgents } from "../../stores/connection";
import { healthData, fetchHealth } from "../../stores/health";
import {
  gitWorktrees,
  gitLog,
  fetchAllGitData,
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

const ProjectDetail: Component = () => {
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
    <div class="flex-1 overflow-auto" style={{ padding: "24px 32px" }}>
      {/* Header — project name + path + grade badge */}
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
        <span
          class="rounded-md px-3.5 py-1.5 text-[11px] font-semibold"
          style={{ color: grade().color, background: grade().bg }}
        >
          {grade().letter}
        </span>
      </div>

      {/* Section label */}
      <h2
        class="mb-4 text-[10px] font-semibold uppercase"
        style={{ color: "#6B7280", "letter-spacing": "1.2px" }}
      >
        Agents &middot; Worktrees &middot; Commits
      </h2>

      {/* Agent → Worktree → Commit hierarchy (sole content) */}
      <ProjectHierarchy
        projectId={projectId()}
        agents={projectAgents()}
        worktrees={worktrees()}
        commits={recentCommits()}
      />
    </div>
  );
};

export default ProjectDetail;
