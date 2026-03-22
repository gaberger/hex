/**
 * WorkPlanDetail.tsx — Detail view for a workplan.
 *
 * Shows workplan metadata, phases with tier/gate info, action buttons,
 * and linked swarm. Uses REST for workplan data (filesystem op) and
 * SpacetimeDB for swarm linkage via hexflo memory.
 */
import {
  Component,
  For,
  Show,
  createMemo,
  createResource,
  createSignal,
} from "solid-js";
import { swarms, hexfloMemory } from "../../stores/connection";
import { navigate, route } from "../../stores/router";
import { restClient } from "../../services/rest-client";

function statusBadgeClass(status: string): string {
  switch (status) {
    case "running":
    case "in_progress":
    case "active":
      return "bg-cyan-900/40 text-cyan-400";
    case "completed":
    case "done":
      return "bg-green-900/40 text-green-400";
    case "failed":
    case "error":
      return "bg-red-900/40 text-red-400";
    case "paused":
      return "bg-yellow-900/40 text-yellow-400";
    default:
      return "bg-gray-800 text-gray-400";
  }
}

function layerBadgeClass(layer: string): string {
  const l = (layer ?? "").toLowerCase();
  if (l.includes("domain")) return "bg-purple-900/40 text-purple-400";
  if (l.includes("port")) return "bg-blue-900/40 text-blue-400";
  if (l.includes("usecase") || l.includes("use_case") || l.includes("use-case"))
    return "bg-teal-900/40 text-teal-400";
  if (l.includes("adapter") || l.includes("primary") || l.includes("secondary"))
    return "bg-amber-900/40 text-amber-400";
  if (l.includes("integration") || l.includes("test"))
    return "bg-pink-900/40 text-pink-400";
  return "bg-gray-800 text-gray-400";
}

const WorkPlanDetail: Component = () => {
  const projectId = () => (route() as any).projectId ?? "";
  const workplanId = () => (route() as any).workplanId ?? "";

  // Fetch workplan data via REST (filesystem op)
  const [workplan, { refetch }] = createResource(
    () => workplanId(),
    async (wpId) => {
      if (!wpId) return null;
      try {
        return await restClient.get<any>(
          `/api/workplan/${encodeURIComponent(wpId)}`,
        );
      } catch {
        return null;
      }
    },
  );

  // Check hexflo memory for a linked swarm
  const linkedSwarmId = createMemo(() => {
    const key = `workplan:${workplanId()}:swarm`;
    const mem = hexfloMemory().find(
      (m: any) => (m.key ?? "") === key,
    );
    return mem?.value ?? "";
  });

  const linkedSwarm = createMemo(() => {
    const sid = linkedSwarmId();
    if (!sid) return null;
    return swarms().find(
      (s: any) => (s.id ?? s.swarm_id ?? "") === sid,
    );
  });

  const [executing, setExecuting] = createSignal(false);

  function handleBack() {
    navigate({ page: "project-workplans", projectId: projectId() });
  }

  async function handleExecute() {
    setExecuting(true);
    try {
      await restClient.post(`/api/workplan/execute`, {
        workplanId: workplanId(),
        projectId: projectId(),
      });
      refetch();
    } finally {
      setExecuting(false);
    }
  }

  async function handlePause() {
    try {
      await restClient.post(`/api/workplan/${encodeURIComponent(workplanId())}/pause`);
      refetch();
    } catch {
      // ignore
    }
  }

  async function handleResume() {
    try {
      await restClient.post(`/api/workplan/${encodeURIComponent(workplanId())}/resume`);
      refetch();
    } catch {
      // ignore
    }
  }

  function handleSwarmClick(swarmId: string) {
    navigate({
      page: "project-swarm-detail",
      projectId: projectId(),
      swarmId,
    });
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      {/* Back button */}
      <button
        class="mb-4 flex items-center gap-1 text-xs text-gray-400 transition-colors hover:text-gray-200"
        onClick={handleBack}
      >
        <span>&larr;</span>
        <span>Back to WorkPlans</span>
      </button>

      <Show when={workplan.loading}>
        <p class="text-xs text-gray-500">Loading workplan...</p>
      </Show>

      <Show
        when={workplan() && !workplan.loading}
        fallback={
          <Show when={!workplan.loading}>
            <div class="flex flex-1 items-center justify-center">
              <p class="text-sm text-gray-400">
                WorkPlan not found: {workplanId()}
              </p>
            </div>
          </Show>
        }
      >
        {(wp) => {
          const name = () =>
            wp().name ?? wp().title ?? wp().path ?? workplanId();
          const wpStatus = () => wp().status ?? wp().state ?? "pending";
          const currentPhase = () =>
            wp().current_phase ?? wp().currentPhase ?? "";
          const phases = () => wp().phases ?? wp().steps ?? [];

          return (
            <>
              {/* Header */}
              <div class="mb-4">
                <div class="flex items-center gap-2">
                  <h2 class="text-lg font-semibold text-gray-100">
                    {name()}
                  </h2>
                  <span
                    class={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${statusBadgeClass(wpStatus())}`}
                  >
                    {wpStatus()}
                  </span>
                </div>
                <Show when={currentPhase()}>
                  <p class="mt-1 text-[10px] text-gray-500">
                    Current phase: {currentPhase()}
                  </p>
                </Show>
              </div>

              {/* Action Buttons */}
              <div class="mb-6 flex items-center gap-2">
                <Show
                  when={
                    wpStatus() === "pending" || wpStatus() === "ready"
                  }
                >
                  <button
                    class="rounded border border-cyan-700 bg-cyan-900/30 px-3 py-1.5 text-xs text-cyan-300 transition-colors hover:bg-cyan-900/60 disabled:opacity-50"
                    onClick={handleExecute}
                    disabled={executing()}
                  >
                    {executing() ? "Starting..." : "Execute"}
                  </button>
                </Show>
                <Show
                  when={
                    wpStatus() === "running" ||
                    wpStatus() === "in_progress"
                  }
                >
                  <button
                    class="rounded border border-yellow-700 bg-yellow-900/30 px-3 py-1.5 text-xs text-yellow-300 transition-colors hover:bg-yellow-900/60"
                    onClick={handlePause}
                  >
                    Pause
                  </button>
                </Show>
                <Show when={wpStatus() === "paused"}>
                  <button
                    class="rounded border border-green-700 bg-green-900/30 px-3 py-1.5 text-xs text-green-300 transition-colors hover:bg-green-900/60"
                    onClick={handleResume}
                  >
                    Resume
                  </button>
                </Show>
              </div>

              {/* Phases */}
              <SectionHeader
                title="Phases"
                count={phases().length}
              />
              <Show
                when={phases().length > 0}
                fallback={
                  <p class="mb-6 text-xs text-gray-500">
                    No phases defined
                  </p>
                }
              >
                <div class="mb-6 space-y-3">
                  <For each={phases()}>
                    {(phase: any, idx) => {
                      const phaseName =
                        phase.name ?? phase.title ?? `Phase ${idx() + 1}`;
                      const phaseDesc = phase.description ?? "";
                      const tier = phase.tier ?? phase.tier_number ?? "";
                      const gateCmd =
                        phase.gate_command ?? phase.gate ?? "";
                      const gateBlocking =
                        phase.blocking ?? phase.gate_blocking ?? false;
                      const phaseTasks =
                        phase.tasks ?? phase.steps ?? [];
                      const phaseStatus =
                        phase.status ?? phase.state ?? "pending";

                      return (
                        <div class="rounded-lg border border-gray-800 bg-gray-900/50 p-3">
                          {/* Phase header */}
                          <div class="mb-2 flex items-center gap-2">
                            <Show when={tier !== ""}>
                              <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] font-mono text-gray-400">
                                T{tier}
                              </span>
                            </Show>
                            <span class="text-sm font-medium text-gray-100">
                              {phaseName}
                            </span>
                            <span
                              class={`ml-auto rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${statusBadgeClass(phaseStatus)}`}
                            >
                              {phaseStatus}
                            </span>
                          </div>

                          <Show when={phaseDesc}>
                            <p class="mb-2 text-[11px] text-gray-400">
                              {phaseDesc}
                            </p>
                          </Show>

                          {/* Gate */}
                          <Show when={gateCmd}>
                            <div class="mb-2 flex items-center gap-2 rounded bg-gray-800/60 px-2 py-1">
                              <span class="text-[10px] text-gray-500">
                                Gate:
                              </span>
                              <span class="font-mono text-[10px] text-gray-300">
                                {gateCmd}
                              </span>
                              <Show when={gateBlocking}>
                                <span class="rounded-full bg-red-900/40 px-1.5 py-0.5 text-[10px] font-semibold uppercase text-red-400">
                                  blocking
                                </span>
                              </Show>
                            </div>
                          </Show>

                          {/* Phase tasks */}
                          <Show when={phaseTasks.length > 0}>
                            <div class="space-y-1">
                              <For each={phaseTasks}>
                                {(task: any) => {
                                  const tName =
                                    task.name ??
                                    task.title ??
                                    "Untitled";
                                  const tLayer =
                                    task.layer ?? task.boundary ?? "";
                                  const tStatus =
                                    task.status ?? "pending";
                                  const tAgent =
                                    task.assigned_to ??
                                    task.agent ??
                                    "";

                                  return (
                                    <div class="flex items-center gap-2 rounded border border-gray-700/50 bg-gray-900/40 px-2 py-1.5 text-xs">
                                      <span class="flex-1 truncate text-gray-300">
                                        {tName}
                                      </span>
                                      <Show when={tLayer}>
                                        <span
                                          class={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${layerBadgeClass(tLayer)}`}
                                        >
                                          {tLayer}
                                        </span>
                                      </Show>
                                      <span
                                        class={`shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${statusBadgeClass(tStatus)}`}
                                      >
                                        {tStatus}
                                      </span>
                                      <Show when={tAgent}>
                                        <span class="shrink-0 text-[10px] text-gray-500">
                                          {tAgent}
                                        </span>
                                      </Show>
                                    </div>
                                  );
                                }}
                              </For>
                            </div>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </Show>

              {/* Linked Swarm */}
              <SectionHeader
                title="Linked Swarm"
                count={linkedSwarm() ? 1 : 0}
              />
              <Show
                when={linkedSwarm()}
                fallback={
                  <p class="text-xs text-gray-500">
                    No linked swarm found
                  </p>
                }
              >
                {(ls) => (
                  <button
                    class="flex w-full items-center gap-2 rounded-lg border border-gray-800 bg-gray-900/50 px-3 py-2 text-left text-xs transition-colors hover:border-gray-600"
                    onClick={() =>
                      handleSwarmClick(
                        ls().id ?? ls().swarm_id ?? "",
                      )
                    }
                  >
                    <span class="text-gray-100">
                      {ls().name ?? "unnamed"}
                    </span>
                    <span class="rounded-full bg-gray-800 px-2 py-0.5 text-[10px] font-semibold uppercase text-gray-300">
                      {ls().topology ?? "unknown"}
                    </span>
                    <span class="ml-auto text-[10px] text-gray-500">
                      {ls().status ?? ""}
                    </span>
                  </button>
                )}
              </Show>
            </>
          );
        }}
      </Show>
    </div>
  );
};

const SectionHeader: Component<{ title: string; count: number }> = (
  props,
) => (
  <div class="mb-2 flex items-center gap-2">
    <h4 class="text-[11px] font-semibold uppercase tracking-wider text-gray-400">
      {props.title}
    </h4>
    <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-400">
      {props.count}
    </span>
  </div>
);

export default WorkPlanDetail;
