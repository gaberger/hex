/**
 * ProjectSidebar.tsx — Left sidebar listing all registered projects + fleet status.
 *
 * Matches the Pencil design: 220px dark sidebar with project list,
 * selection highlight, and fleet status summary.
 */
import { type Component, For, Show, createMemo } from "solid-js";
import { projects } from "../../stores/projects";
import { registryAgents } from "../../stores/connection";
import { route, navigate } from "../../stores/router";

const ProjectSidebar: Component = () => {
  const currentProjectId = createMemo(() => {
    const r = route();
    return (r as any).projectId ?? "";
  });

  const fleetSummary = createMemo(() => {
    const agents = registryAgents();
    const online = agents.filter((a: any) => {
      const s = (a.status ?? "idle").toLowerCase();
      return s === "online" || s === "active" || s === "running" || s === "registered";
    }).length;
    const busy = agents.filter((a: any) => {
      const s = (a.status ?? "idle").toLowerCase();
      return s === "busy" || s === "spawning";
    }).length;
    const idle = agents.filter((a: any) => {
      const s = (a.status ?? "idle").toLowerCase();
      return s === "idle" || s === "completed";
    }).length;
    const offline = agents.length - online - busy - idle;
    return { online, busy, idle, offline, total: agents.length };
  });

  return (
    <div
      class="flex w-[220px] min-w-[220px] flex-col gap-3 overflow-y-auto bg-[var(--bg-base)] px-3 py-4"
    >
      {/* Projects section */}
      <span
        class="text-[10px] font-semibold uppercase tracking-[1.2px] text-[var(--text-faint)]"
      >
        Projects
      </span>

      <div class="flex flex-col gap-1">
        <For each={projects()}>
          {(project) => {
            const isSelected = () => currentProjectId() === project.id;
            return (
              <button
                class="flex w-full items-center gap-2 rounded-md px-3 py-2 text-left transition-colors"
                style={{
                  background: isSelected() ? "var(--bg-elevated)" : "transparent",
                }}
                classList={{
                  "hover:bg-[#1E293B]/50": !isSelected(),
                }}
                onClick={() => navigate({ page: "project", projectId: project.id })}
              >
                <span
                  class="h-1.5 w-1.5 shrink-0 rounded-full"
                  classList={{
                    "bg-status-active": isSelected(),
                    "bg-status-idle": !isSelected(),
                  }}
                />
                <span
                  class="truncate text-[13px]"
                  style={{
                    "font-family": "'JetBrains Mono', monospace",
                    "font-weight": isSelected() ? "600" : "400",
                    color: isSelected() ? "var(--text-body)" : "var(--text-muted)",
                  }}
                >
                  {project.name}
                </span>
              </button>
            );
          }}
        </For>

        {/* Empty state */}
        <Show when={projects().length === 0}>
          <p class="px-3 py-4 text-center text-[11px] text-gray-600">
            No projects registered
          </p>
        </Show>
      </div>

      {/* Divider */}
      <div class="h-px bg-[var(--border-subtle)]" />

      {/* Fleet Status section */}
      <span
        class="text-[10px] font-semibold uppercase tracking-[1.2px] text-[var(--text-faint)]"
      >
        Fleet Status
      </span>

      <div class="flex flex-col gap-2">
        <Show when={fleetSummary().online > 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full bg-status-active" />
            <span class="text-[11px] text-[var(--text-muted)]">
              {fleetSummary().online} agent{fleetSummary().online !== 1 ? "s" : ""} online
            </span>
          </div>
        </Show>
        <Show when={fleetSummary().busy > 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full bg-status-warning" />
            <span class="text-[11px] text-[var(--text-muted)]">
              {fleetSummary().busy} agent{fleetSummary().busy !== 1 ? "s" : ""} busy
            </span>
          </div>
        </Show>
        <Show when={fleetSummary().idle > 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full bg-status-idle" />
            <span class="text-[11px] text-[var(--text-muted)]">
              {fleetSummary().idle} agent{fleetSummary().idle !== 1 ? "s" : ""} idle
            </span>
          </div>
        </Show>
        <Show when={fleetSummary().total === 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full bg-status-error" />
            <span class="text-[11px] text-[var(--text-muted)]">
              No agents connected
            </span>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default ProjectSidebar;
