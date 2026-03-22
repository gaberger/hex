/**
 * WorkplanView.tsx — Main workplan dashboard view showing active execution
 * banner, execution history table, and expandable detail panels.
 *
 * Data source: hex-nexus REST API via workplan store.
 */
import { Component, For, Show, createSignal, createMemo, onMount, onCleanup } from "solid-js";
import {
  workplans,
  activeWorkplan,
  workplanLoading,
  workplanError,
  fetchWorkplans,
  fetchReport,
  startWorkplanPoll,
  stopWorkplanPoll,
  type WorkplanExecution,
  type WorkplanPhase,
  type WorkplanReport,
} from "../../stores/workplan";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function statusColor(status: string): string {
  switch (status) {
    case "active":
      return "bg-cyan-900/30 text-cyan-400";
    case "completed":
    case "passed":
      return "bg-green-900/30 text-green-400";
    case "failed":
      return "bg-red-900/30 text-red-400";
    case "cancelled":
    case "skipped":
      return "bg-gray-800 text-gray-400";
    case "pending":
    default:
      return "bg-yellow-900/30 text-yellow-400";
  }
}

function statusDotColor(status: string): string {
  switch (status) {
    case "active":
      return "bg-cyan-400";
    case "completed":
    case "passed":
      return "bg-green-500";
    case "failed":
      return "bg-red-500";
    case "cancelled":
    case "skipped":
      return "bg-gray-500";
    case "pending":
    default:
      return "bg-yellow-500";
  }
}

function phaseBgColor(status: string): string {
  switch (status) {
    case "active":
      return "bg-cyan-500";
    case "passed":
    case "completed":
      return "bg-green-500";
    case "failed":
      return "bg-red-500";
    case "skipped":
      return "bg-gray-600";
    case "pending":
    default:
      return "bg-gray-700";
  }
}

function formatDuration(start: string | null, end: string | null): string {
  if (!start) return "--";
  const s = new Date(start).getTime();
  const e = end ? new Date(end).getTime() : Date.now();
  const diff = e - s;
  if (diff < 0) return "--";
  const secs = Math.floor(diff / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ${secs % 60}s`;
  const hrs = Math.floor(mins / 60);
  return `${hrs}h ${mins % 60}m`;
}

function formatTime(ts: string | null): string {
  if (!ts) return "--";
  const d = new Date(ts);
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function completedPhases(phases: WorkplanPhase[]): number {
  return phases.filter(
    (p) => p.status === "passed" || p.status === "completed" || p.status === "skipped"
  ).length;
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

const WorkplanView: Component = () => {
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [report, setReport] = createSignal<WorkplanReport | null>(null);
  const [reportLoading, setReportLoading] = createSignal(false);

  onMount(() => {
    startWorkplanPoll();
  });

  onCleanup(() => {
    stopWorkplanPoll();
  });

  const sortedWorkplans = createMemo(() => {
    return [...workplans()].sort((a, b) => {
      // Active first, then by creation time descending
      if (a.status === "active" && b.status !== "active") return -1;
      if (b.status === "active" && a.status !== "active") return 1;
      return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
    });
  });

  async function handleSelectExecution(id: string) {
    if (selectedId() === id) {
      setSelectedId(null);
      setReport(null);
      return;
    }
    setSelectedId(id);
    setReport(null);
    setReportLoading(true);
    const r = await fetchReport(id);
    setReport(r);
    setReportLoading(false);
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-6">
      {/* Header */}
      <div class="mb-6 flex items-center justify-between">
        <div>
          <h2 class="text-[22px] font-bold text-gray-100">Workplans</h2>
          <p class="mt-0.5 text-xs text-gray-400">
            {workplans().length} execution{workplans().length !== 1 ? "s" : ""}
          </p>
        </div>
        <button
          class="rounded-lg border border-gray-700 bg-gray-900 px-3 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:border-gray-600 hover:text-gray-100"
          onClick={() => fetchWorkplans()}
        >
          Refresh
        </button>
      </div>

      {/* Active execution banner */}
      <Show when={activeWorkplan()}>
        {(active) => (
          <ActiveBanner execution={active()} onSelect={handleSelectExecution} />
        )}
      </Show>

      {/* Error state */}
      <Show when={workplanError()}>
        <div class="mb-4 rounded-lg border border-red-900/50 bg-red-950/30 px-4 py-3 text-sm text-red-400">
          {workplanError()}
        </div>
      </Show>

      {/* Loading state */}
      <Show when={workplanLoading() && workplans().length === 0}>
        <div class="flex flex-1 items-center justify-center">
          <p class="text-sm text-gray-500">Loading workplans...</p>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!workplanLoading() && workplans().length === 0 && !workplanError()}>
        <div class="rounded-xl border border-dashed border-gray-800 bg-gray-900/30 px-6 py-12 text-center">
          <p class="text-sm text-gray-400">No workplan executions found</p>
          <p class="mt-1 text-[11px] text-gray-500">
            Start a feature with{" "}
            <code class="rounded bg-gray-800 px-1 py-0.5 font-mono text-[10px] text-cyan-300">
              /hex-feature-dev
            </code>{" "}
            to create a workplan execution.
          </p>
        </div>
      </Show>

      {/* Execution history table */}
      <Show when={workplans().length > 0}>
        <section>
          <h3 class="mb-3 text-[12px] font-semibold uppercase tracking-wider text-gray-500">
            Execution History
          </h3>

          <div class="overflow-hidden rounded-xl border border-gray-800">
            {/* Table header */}
            <div class="grid grid-cols-[1fr_140px_100px_120px_80px] gap-2 border-b border-gray-800 bg-gray-900/60 px-4 py-2.5 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
              <span>Feature</span>
              <span>Status</span>
              <span>Phases</span>
              <span>Duration</span>
              <span>Agents</span>
            </div>

            {/* Table rows */}
            <For each={sortedWorkplans()}>
              {(execution) => (
                <>
                  <button
                    class="grid w-full grid-cols-[1fr_140px_100px_120px_80px] gap-2 border-b border-gray-800/50 px-4 py-3 text-left text-sm transition-colors hover:bg-gray-900/50 focus:outline-none focus:ring-1 focus:ring-inset focus:ring-cyan-500/30"
                    classList={{
                      "bg-gray-900/40": selectedId() === execution.id,
                    }}
                    onClick={() => handleSelectExecution(execution.id)}
                  >
                    {/* Feature name */}
                    <div class="flex items-center gap-2">
                      <span
                        class={`h-2 w-2 shrink-0 rounded-full ${statusDotColor(execution.status)}`}
                        classList={{
                          "animate-pulse": execution.status === "active",
                        }}
                      />
                      <span class="truncate font-medium text-gray-200">
                        {execution.feature}
                      </span>
                    </div>

                    {/* Status badge */}
                    <div>
                      <span
                        class={`inline-block rounded-full px-2 py-0.5 text-[10px] font-medium ${statusColor(execution.status)}`}
                      >
                        {execution.status}
                      </span>
                    </div>

                    {/* Phases progress */}
                    <div class="flex items-center gap-2">
                      <span class="text-xs text-gray-300">
                        {completedPhases(execution.phases)}/{execution.phases.length}
                      </span>
                      <PhaseProgressMini phases={execution.phases} />
                    </div>

                    {/* Duration */}
                    <span class="text-xs text-gray-400">
                      {formatDuration(execution.startedAt, execution.completedAt)}
                    </span>

                    {/* Agent count */}
                    <span class="text-xs text-gray-400">
                      {execution.agents.length}
                    </span>
                  </button>

                  {/* Expanded detail panel */}
                  <Show when={selectedId() === execution.id}>
                    <DetailPanel
                      execution={execution}
                      report={report()}
                      loading={reportLoading()}
                    />
                  </Show>
                </>
              )}
            </For>
          </div>
        </section>
      </Show>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

/** Active execution banner with phase progress bar */
const ActiveBanner: Component<{
  execution: WorkplanExecution;
  onSelect: (id: string) => void;
}> = (props) => {
  const progress = createMemo(() => {
    const phases = props.execution.phases;
    if (phases.length === 0) return 0;
    return Math.round((completedPhases(phases) / phases.length) * 100);
  });

  return (
    <button
      class="mb-6 flex w-full flex-col gap-3 rounded-xl border border-cyan-800/40 bg-cyan-950/20 p-4 text-left transition-colors hover:border-cyan-700/50"
      onClick={() => props.onSelect(props.execution.id)}
    >
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div class="h-2.5 w-2.5 animate-pulse rounded-full bg-cyan-400" />
          <span class="font-semibold text-gray-100">
            {props.execution.feature}
          </span>
          <span class="rounded-full bg-cyan-900/30 px-2 py-0.5 text-[10px] font-medium text-cyan-400">
            active
          </span>
        </div>
        <span class="text-xs text-gray-400">
          Phase: {props.execution.currentPhase ?? "starting"}
        </span>
      </div>

      {/* Phase progress bar */}
      <div class="flex items-center gap-3">
        <div class="flex flex-1 gap-1">
          <For each={props.execution.phases}>
            {(phase) => (
              <div
                class={`h-2 flex-1 rounded-full ${phaseBgColor(phase.status)}`}
                classList={{
                  "animate-pulse": phase.status === "active",
                }}
                title={`${phase.name}: ${phase.status}`}
              />
            )}
          </For>
        </div>
        <span class="shrink-0 text-xs font-medium text-gray-300">
          {progress()}%
        </span>
      </div>

      {/* Phase labels */}
      <div class="flex gap-1 overflow-x-auto">
        <For each={props.execution.phases}>
          {(phase) => (
            <span
              class="shrink-0 rounded px-1.5 py-0.5 text-[10px]"
              classList={{
                "bg-cyan-900/30 text-cyan-300 font-medium": phase.status === "active",
                "text-green-400": phase.status === "passed" || phase.status === "completed",
                "text-gray-500": phase.status === "pending",
                "text-red-400": phase.status === "failed",
                "text-gray-600": phase.status === "skipped",
              }}
            >
              {phase.name}
            </span>
          )}
        </For>
      </div>
    </button>
  );
};

/** Mini progress bar for the table row */
const PhaseProgressMini: Component<{ phases: WorkplanPhase[] }> = (props) => (
  <div class="flex flex-1 gap-0.5">
    <For each={props.phases}>
      {(phase) => (
        <div
          class={`h-1.5 flex-1 rounded-full ${phaseBgColor(phase.status)}`}
          classList={{ "animate-pulse": phase.status === "active" }}
        />
      )}
    </For>
  </div>
);

/** Expandable detail panel for a selected execution */
const DetailPanel: Component<{
  execution: WorkplanExecution;
  report: WorkplanReport | null;
  loading: boolean;
}> = (props) => {
  return (
    <div class="border-b border-gray-800 bg-gray-900/30 px-6 py-4">
      {/* Phase breakdown */}
      <div class="mb-4">
        <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
          Phase Breakdown
        </h4>
        <div class="grid gap-2">
          <For each={props.execution.phases}>
            {(phase) => (
              <div class="flex items-center gap-3 rounded-lg border border-gray-800/50 bg-gray-900/50 px-3 py-2">
                <span
                  class={`h-2 w-2 shrink-0 rounded-full ${statusDotColor(phase.status)}`}
                  classList={{ "animate-pulse": phase.status === "active" }}
                />
                <span class="min-w-[100px] text-xs font-medium text-gray-200">
                  {phase.name}
                </span>
                <span
                  class={`rounded-full px-2 py-0.5 text-[10px] font-medium ${statusColor(phase.status)}`}
                >
                  {phase.status}
                </span>
                <span class="text-[11px] text-gray-500">
                  {formatDuration(phase.startedAt, phase.completedAt)}
                </span>
                <Show when={phase.agents.length > 0}>
                  <span class="text-[11px] text-gray-500">
                    {phase.agents.length} agent{phase.agents.length !== 1 ? "s" : ""}
                  </span>
                </Show>
                <Show when={phase.gateResult}>
                  <span
                    class="ml-auto rounded px-1.5 py-0.5 text-[10px] font-mono"
                    classList={{
                      "bg-green-900/30 text-green-400": phase.gateResult === "PASS",
                      "bg-red-900/30 text-red-400": phase.gateResult === "FAIL",
                      "bg-yellow-900/30 text-yellow-400": phase.gateResult !== "PASS" && phase.gateResult !== "FAIL",
                    }}
                  >
                    Gate: {phase.gateResult}
                  </span>
                </Show>
              </div>
            )}
          </For>
        </div>
      </div>

      {/* Report details (loaded async) */}
      <Show when={props.loading}>
        <p class="text-xs text-gray-500">Loading report...</p>
      </Show>

      <Show when={props.report}>
        {(rpt) => (
          <>
            {/* Gate results */}
            <Show when={rpt().gateResults.length > 0}>
              <div class="mb-4">
                <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
                  Gate Results
                </h4>
                <div class="grid gap-1.5">
                  <For each={rpt().gateResults}>
                    {(gate) => (
                      <div class="flex items-center gap-3 text-xs">
                        <span
                          class="rounded px-1.5 py-0.5 font-mono text-[10px]"
                          classList={{
                            "bg-green-900/30 text-green-400": gate.result === "PASS",
                            "bg-red-900/30 text-red-400": gate.result === "FAIL",
                          }}
                        >
                          {gate.result}
                        </span>
                        <span class="font-medium text-gray-300">{gate.phase}</span>
                        <span class="text-gray-500">{gate.details}</span>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            </Show>

            {/* Git commits */}
            <Show when={rpt().commits.length > 0}>
              <div>
                <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
                  Commits
                </h4>
                <div class="grid gap-1.5">
                  <For each={rpt().commits}>
                    {(commit) => (
                      <div class="flex items-center gap-3 text-xs">
                        <code class="shrink-0 rounded bg-gray-800 px-1.5 py-0.5 font-mono text-[10px] text-cyan-300">
                          {commit.sha.slice(0, 7)}
                        </code>
                        <span class="truncate text-gray-300">{commit.message}</span>
                        <span class="shrink-0 text-gray-600">{commit.phase}</span>
                        <span class="shrink-0 text-gray-600">{commit.author}</span>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            </Show>
          </>
        )}
      </Show>

      {/* Execution metadata */}
      <div class="mt-4 flex gap-6 border-t border-gray-800/50 pt-3 text-[11px] text-gray-500">
        <span>ID: <code class="font-mono text-gray-400">{props.execution.id}</code></span>
        <span>Topology: {props.execution.topology}</span>
        <span>Created: {formatTime(props.execution.createdAt)}</span>
        <Show when={props.execution.completedAt}>
          <span>Completed: {formatTime(props.execution.completedAt)}</span>
        </Show>
      </div>
    </div>
  );
};

export default WorkplanView;
