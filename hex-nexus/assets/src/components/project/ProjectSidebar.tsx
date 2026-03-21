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
      return s === "online" || s === "active";
    }).length;
    const busy = agents.filter((a: any) => {
      const s = (a.status ?? "idle").toLowerCase();
      return s === "busy";
    }).length;
    const idle = agents.filter((a: any) => {
      const s = (a.status ?? "idle").toLowerCase();
      return s === "idle";
    }).length;
    const offline = agents.length - online - busy - idle;
    return { online, busy, idle, offline, total: agents.length };
  });

  return (
    <div
      class="flex flex-col gap-3 overflow-y-auto"
      style={{
        width: "220px",
        "min-width": "220px",
        background: "#0D1526",
        padding: "16px 12px",
      }}
    >
      {/* Projects section */}
      <span
        class="text-[10px] font-semibold uppercase tracking-wider"
        style={{ color: "#6B7280", "letter-spacing": "1.2px" }}
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
                  background: isSelected() ? "#1E293B" : "transparent",
                }}
                classList={{
                  "hover:bg-[#1E293B]/50": !isSelected(),
                }}
                onClick={() => navigate({ page: "project", projectId: project.id })}
              >
                <span
                  class="h-1.5 w-1.5 shrink-0 rounded-full"
                  style={{
                    background: isSelected() ? "#10B981" : "#6B7280",
                  }}
                />
                <span
                  class="truncate text-[13px]"
                  style={{
                    "font-family": "'JetBrains Mono', monospace",
                    "font-weight": isSelected() ? "600" : "400",
                    color: isSelected() ? "#E5E7EB" : "#9CA3AF",
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
      <div style={{ height: "1px", background: "#1E293B" }} />

      {/* Fleet Status section */}
      <span
        class="text-[10px] font-semibold uppercase tracking-wider"
        style={{ color: "#6B7280", "letter-spacing": "1.2px" }}
      >
        Fleet Status
      </span>

      <div class="flex flex-col gap-2">
        <Show when={fleetSummary().online > 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full" style={{ background: "#10B981" }} />
            <span class="text-[11px]" style={{ color: "#9CA3AF" }}>
              {fleetSummary().online} agent{fleetSummary().online !== 1 ? "s" : ""} online
            </span>
          </div>
        </Show>
        <Show when={fleetSummary().busy > 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full" style={{ background: "#FBBF24" }} />
            <span class="text-[11px]" style={{ color: "#9CA3AF" }}>
              {fleetSummary().busy} agent{fleetSummary().busy !== 1 ? "s" : ""} busy
            </span>
          </div>
        </Show>
        <Show when={fleetSummary().idle > 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full" style={{ background: "#6B7280" }} />
            <span class="text-[11px]" style={{ color: "#9CA3AF" }}>
              {fleetSummary().idle} agent{fleetSummary().idle !== 1 ? "s" : ""} idle
            </span>
          </div>
        </Show>
        <Show when={fleetSummary().total === 0}>
          <div class="flex items-center gap-1.5">
            <span class="h-1.5 w-1.5 rounded-full" style={{ background: "#EF4444" }} />
            <span class="text-[11px]" style={{ color: "#9CA3AF" }}>
              No agents connected
            </span>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default ProjectSidebar;
