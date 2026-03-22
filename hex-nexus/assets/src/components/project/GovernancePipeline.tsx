/**
 * GovernancePipeline.tsx — Horizontal pipeline banner showing ADR -> WorkPlan -> HexFlo flow.
 *
 * Each node is a clickable pill with count + status, connected by arrows.
 * Data: ADRs from REST, workplans from REST, swarms from SpacetimeDB subscription.
 */
import { Component, createResource, createMemo } from "solid-js";
import { swarms, swarmTasks } from "../../stores/connection";
import { navigate } from "../../stores/router";
import { restClient } from "../../services/rest-client";

interface GovernancePipelineProps {
  projectId: string;
}

const GovernancePipeline: Component<GovernancePipelineProps> = (props) => {
  // ADR count from REST
  const [adrData] = createResource(
    () => props.projectId,
    async (pid) => {
      try {
        const data = await restClient.get<any>(`/api/projects/${pid}/adrs`);
        const adrs = Array.isArray(data) ? data : data?.adrs ?? [];
        const accepted = adrs.filter(
          (a: any) => a.status === "accepted" || a.status === "Accepted",
        );
        return { total: adrs.length, accepted: accepted.length };
      } catch {
        return null;
      }
    },
  );

  // Workplan count from REST
  const [workplanData] = createResource(
    () => props.projectId,
    async (pid) => {
      try {
        const data = await restClient.get<any>(`/api/projects/${pid}/workplans`);
        const plans = Array.isArray(data) ? data : data?.workplans ?? [];
        const active = plans.filter(
          (w: any) => w.status === "active" || w.status === "in_progress",
        );
        return { total: plans.length, active: active.length };
      } catch {
        return null;
      }
    },
  );

  // Swarm data from SpacetimeDB subscription
  const projectSwarms = createMemo(() =>
    swarms().filter(
      (s: any) => (s.project ?? s.project_id ?? "") === props.projectId,
    ),
  );

  const runningSwarms = createMemo(() =>
    projectSwarms().filter(
      (s: any) => s.status === "active" || s.status === "running" || !s.status,
    ),
  );

  const swarmProgress = createMemo(() => {
    const running = runningSwarms();
    if (running.length === 0) return 0;
    const ids = running.map((s: any) => s.id ?? s.swarm_id ?? "");
    const tasks = swarmTasks().filter((t: any) =>
      ids.includes(t.swarmId ?? t.swarm_id ?? ""),
    );
    if (tasks.length === 0) return 0;
    const done = tasks.filter(
      (t: any) => t.status === "completed" || t.status === "done",
    ).length;
    return Math.round((done / tasks.length) * 100);
  });

  return (
    <div class="flex items-center gap-2 rounded-lg border border-gray-800 bg-gray-900 px-4 py-3">
      {/* ADR node */}
      <button
        class="flex items-center gap-2 rounded-full border border-gray-700 bg-gray-950 px-3 py-1.5 text-[13px] font-semibold cursor-pointer transition-colors hover:border-[var(--accent)]"
        onClick={() =>
          navigate({ page: "project-adrs", projectId: props.projectId })
        }
      >
        <span class="text-hex-primary">ADRs:</span>
        <span class="text-[var(--text-body)]">
          {adrData()
            ? `${adrData()!.accepted} accepted`
            : adrData.loading
              ? "..."
              : "No ADRs yet"}
        </span>
      </button>

      {/* Arrow */}
      <span class="text-[var(--text-faint)] text-lg select-none">&rarr;</span>

      {/* WorkPlan node */}
      <button
        class="flex items-center gap-2 rounded-full border border-gray-700 bg-gray-950 px-3 py-1.5 text-[13px] font-semibold cursor-pointer transition-colors hover:border-[var(--accent)]"
        onClick={() =>
          navigate({ page: "project-workplans", projectId: props.projectId })
        }
      >
        <span class="text-hex-ports">WorkPlans:</span>
        <span class="text-[var(--text-body)]">
          {workplanData()
            ? `${workplanData()!.active} active`
            : workplanData.loading
              ? "..."
              : "No workplans"}
        </span>
      </button>

      {/* Arrow */}
      <span class="text-[var(--text-faint)] text-lg select-none">&rarr;</span>

      {/* Swarm node */}
      <button
        class="flex items-center gap-2 rounded-full border border-gray-700 bg-gray-950 px-3 py-1.5 text-[13px] font-semibold cursor-pointer transition-colors hover:border-[var(--accent)]"
        onClick={() =>
          navigate({ page: "project-swarms", projectId: props.projectId })
        }
      >
        <span class="text-hex-usecases">Swarms:</span>
        <span class="text-[var(--text-body)]">
          {runningSwarms().length > 0
            ? `${runningSwarms().length} running (${swarmProgress()}%)`
            : "No swarms"}
        </span>
      </button>
    </div>
  );
};

export default GovernancePipeline;
