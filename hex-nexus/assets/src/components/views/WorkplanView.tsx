/**
 * WorkplanView.tsx — Main workplan dashboard view showing active execution
 * banner, execution history table, and expandable detail panels.
 *
 * Data source: hex-nexus REST API via workplan store.
 */
import { Component, For, Show, createSignal, createMemo, createResource, onMount, onCleanup } from "solid-js";
import { navigate, route } from "../../stores/router";
import {
  workplans,
  activeWorkplan,
  workplanLoading,
  workplanError,
  fetchWorkplans,
  fetchReport,
  executeWorkplan,
  pauseWorkplan,
  resumeWorkplan,
  startWorkplanPoll,
  stopWorkplanPoll,
  type WorkplanExecution,
  type WorkplanPhase,
  type WorkplanReport,
} from "../../stores/workplan";
import { restClient } from "../../services/rest-client";

// ── Workplan file types ──────────────────────────────────────────────────────

interface WorkplanFile {
  file: string;
  id: string;
  title: string;
  priority: string;
  created_at: string;
  phases: number;
  tasks: number;
  related_adrs: string[];
}

interface WorkplanFilesResponse {
  ok: boolean;
  count: number;
  workplans: WorkplanFile[];
}

async function fetchWorkplanFiles(): Promise<WorkplanFilesResponse> {
  try {
    return await restClient.get<WorkplanFilesResponse>("/api/workplans");
  } catch {
    // Fallback: direct fetch in case restClient has issues
    const res = await fetch("/api/workplans");
    if (!res.ok) return { ok: false as any, count: 0, workplans: [] };
    return res.json();
  }
}

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
    case "paused":
      return "bg-blue-900/30 text-blue-400";
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
    case "paused":
      return "bg-blue-400";
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
  const [showExecuteModal, setShowExecuteModal] = createSignal(false);
  const [executePath, setExecutePath] = createSignal("");
  const [actionLoading, setActionLoading] = createSignal(false);
  const [toast, setToast] = createSignal<{ message: string; type: "success" | "error" } | null>(null);

  const [workplanFiles, { refetch: refetchFiles }] = createResource(fetchWorkplanFiles);

  let toastTimer: ReturnType<typeof setTimeout> | undefined;

  function showToast(message: string, type: "success" | "error") {
    if (toastTimer) clearTimeout(toastTimer);
    setToast({ message, type });
    toastTimer = setTimeout(() => setToast(null), 4000);
  }

  async function handleExecute() {
    const path = executePath().trim();
    if (!path) return;
    setActionLoading(true);
    const result = await executeWorkplan(path);
    setActionLoading(false);
    setShowExecuteModal(false);
    setExecutePath("");
    if (result.ok) {
      showToast("Workplan execution started", "success");
    } else {
      showToast(`Execute failed: ${result.error}`, "error");
    }
  }

  async function handleExecuteFile(file: string) {
    setActionLoading(true);
    const result = await executeWorkplan(`docs/workplans/${file}`);
    setActionLoading(false);
    if (result.ok) {
      showToast(`Workplan "${file}" execution started`, "success");
    } else {
      showToast(`Execute failed: ${result.error}`, "error");
    }
  }

  async function handlePause() {
    setActionLoading(true);
    const result = await pauseWorkplan();
    setActionLoading(false);
    if (result.ok) {
      showToast("Workplan paused", "success");
    } else {
      showToast(`Pause failed: ${result.error}`, "error");
    }
  }

  async function handleResume() {
    setActionLoading(true);
    const result = await resumeWorkplan();
    setActionLoading(false);
    if (result.ok) {
      showToast("Workplan resumed", "success");
    } else {
      showToast(`Resume failed: ${result.error}`, "error");
    }
  }

  onMount(() => {
    startWorkplanPoll();
  });

  onCleanup(() => {
    stopWorkplanPoll();
    if (toastTimer) clearTimeout(toastTimer);
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
      {/* Toast notification */}
      <Show when={toast()}>
        {(t) => (
          <div
            class="fixed right-6 top-6 z-50 rounded-lg border px-4 py-3 text-sm shadow-lg transition-all"
            classList={{
              "border-green-800/50 bg-green-950/90 text-green-300": t().type === "success",
              "border-red-800/50 bg-red-950/90 text-red-300": t().type === "error",
            }}
          >
            {t().message}
          </div>
        )}
      </Show>

      {/* Execute modal */}
      <Show when={showExecuteModal()}>
        <div class="fixed inset-0 z-40 flex items-center justify-center bg-black/60">
          <div class="w-full max-w-md rounded-xl border border-gray-700 bg-gray-900 p-6 shadow-2xl">
            <h3 class="mb-4 text-base font-semibold text-gray-100">Execute Workplan</h3>
            <label class="mb-1.5 block text-xs font-medium text-gray-400">
              Workplan JSON path
            </label>
            <input
              type="text"
              class="mb-4 w-full rounded-lg border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-200 placeholder-gray-500 focus:border-cyan-600 focus:outline-none focus:ring-1 focus:ring-cyan-600"
              placeholder="docs/workplans/feat-my-feature.json"
              value={executePath()}
              onInput={(e) => setExecutePath(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter") handleExecute(); }}
            />
            <div class="flex justify-end gap-2">
              <button
                class="rounded-lg border border-gray-700 bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:border-gray-600 hover:text-gray-100"
                onClick={() => { setShowExecuteModal(false); setExecutePath(""); }}
              >
                Cancel
              </button>
              <button
                class="rounded-lg border border-green-700 bg-green-900/60 px-3 py-1.5 text-xs font-medium text-green-300 transition-colors hover:border-green-600 hover:bg-green-900/80 disabled:opacity-50"
                disabled={actionLoading() || !executePath().trim()}
                onClick={handleExecute}
              >
                {actionLoading() ? "Starting..." : "Execute"}
              </button>
            </div>
          </div>
        </div>
      </Show>

      {/* Header */}
      <div class="mb-6 flex items-center justify-between">
        <div>
          <h2 class="text-[22px] font-bold text-gray-100">Workplans</h2>
          <p class="mt-0.5 text-xs text-gray-400">
            {workplans().length} execution{workplans().length !== 1 ? "s" : ""}
          </p>
        </div>
        <div class="flex items-center gap-2">
          {/* Execute — show when no active/running workplan */}
          <Show when={!activeWorkplan() || activeWorkplan()?.status === "pending"}>
            <button
              class="rounded-lg border border-green-700 bg-green-900/40 px-3 py-1.5 text-xs font-medium text-green-300 transition-colors hover:border-green-600 hover:bg-green-900/60"
              onClick={() => setShowExecuteModal(true)}
            >
              Execute
            </button>
          </Show>

          {/* Pause — show when workplan is active/running */}
          <Show when={activeWorkplan()?.status === "active"}>
            <button
              class="rounded-lg border border-yellow-700 bg-yellow-900/40 px-3 py-1.5 text-xs font-medium text-yellow-300 transition-colors hover:border-yellow-600 hover:bg-yellow-900/60 disabled:opacity-50"
              disabled={actionLoading()}
              onClick={handlePause}
            >
              {actionLoading() ? "Pausing..." : "Pause"}
            </button>
          </Show>

          {/* Resume — show when workplan is paused */}
          <Show when={activeWorkplan()?.status === "paused"}>
            <button
              class="rounded-lg border border-blue-700 bg-blue-900/40 px-3 py-1.5 text-xs font-medium text-blue-300 transition-colors hover:border-blue-600 hover:bg-blue-900/60 disabled:opacity-50"
              disabled={actionLoading()}
              onClick={handleResume}
            >
              {actionLoading() ? "Resuming..." : "Resume"}
            </button>
          </Show>

          <button
            class="rounded-lg border border-gray-700 bg-gray-900 px-3 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:border-gray-600 hover:text-gray-100"
            onClick={() => fetchWorkplans()}
          >
            Refresh
          </button>
        </div>
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

      {/* Execution history table (shown above file list when there are executions) */}
      <Show when={workplans().length > 0}>
        <section class="mb-6">
          <h3 class="mb-3 text-[12px] font-semibold uppercase tracking-wider text-gray-500">
            Execution History
          </h3>

          <div class="overflow-hidden rounded-xl border border-gray-800">
            {/* Table header */}
            <div class="grid grid-cols-[1fr_100px] md:grid-cols-[1fr_140px_100px_120px_80px] gap-2 border-b border-gray-800 bg-gray-900/60 px-4 py-2.5 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
              <span>Feature</span>
              <span>Status</span>
              <span class="hidden md:inline">Phases</span>
              <span class="hidden md:inline">Duration</span>
              <span class="hidden md:inline">Agents</span>
            </div>

            {/* Table rows */}
            <For each={sortedWorkplans()}>
              {(execution) => (
                <>
                  <button
                    class="grid w-full grid-cols-[1fr_100px] md:grid-cols-[1fr_140px_100px_120px_80px] gap-2 border-b border-gray-800/50 px-4 py-3 text-left text-sm transition-colors hover:bg-gray-900/50 focus:outline-none focus:ring-1 focus:ring-inset focus:ring-cyan-500/30"
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
                    <div class="hidden md:flex items-center gap-2">
                      <span class="text-xs text-gray-300">
                        {completedPhases(execution.phases)}/{execution.phases.length}
                      </span>
                      <PhaseProgressMini phases={execution.phases} />
                    </div>

                    {/* Duration */}
                    <span class="hidden md:inline text-xs text-gray-400">
                      {formatDuration(execution.startedAt, execution.completedAt)}
                    </span>

                    {/* Agent count */}
                    <span class="hidden md:inline text-xs text-gray-400">
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

      {/* Workplan Files section */}
      <section>
        <h3 class="mb-3 text-[12px] font-semibold uppercase tracking-wider text-gray-500">
          Workplan Files{" "}
          <Show when={workplanFiles()?.count}>
            ({workplanFiles()!.count})
          </Show>
        </h3>

        <Show when={workplanFiles.loading}>
          <div class="flex items-center justify-center py-8">
            <p class="text-sm text-gray-500">Loading workplan files...</p>
          </div>
        </Show>

        <Show when={workplanFiles.error}>
          <div class="mb-4 rounded-lg border border-red-900/50 bg-red-950/30 px-4 py-3 text-sm text-red-400">
            Failed to load workplan files
          </div>
        </Show>

        <Show when={!workplanFiles.loading && workplanFiles()?.workplans?.length === 0}>
          <div class="rounded-xl border border-dashed border-gray-800 bg-gray-900/30 px-6 py-12 text-center">
            <p class="text-sm text-gray-400">No workplan files found</p>
            <p class="mt-1 text-[11px] text-gray-500">
              Create workplans in{" "}
              <code class="rounded bg-gray-800 px-1 py-0.5 font-mono text-[10px] text-cyan-300">
                docs/workplans/
              </code>{" "}
              or use{" "}
              <code class="rounded bg-gray-800 px-1 py-0.5 font-mono text-[10px] text-cyan-300">
                /hex-feature-dev
              </code>{" "}
              to generate one.
            </p>
          </div>
        </Show>

        <Show when={(workplanFiles()?.workplans?.length ?? 0) > 0}>
          <div class="overflow-hidden rounded-xl border border-gray-800">
            {/* Table header */}
            <div class="grid grid-cols-[1fr_80px] md:grid-cols-[1fr_90px_70px_70px_150px_120px_80px] gap-2 border-b border-gray-800 bg-gray-900/60 px-4 py-2.5 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
              <span>File</span>
              <span>Priority</span>
              <span class="hidden md:inline">Phases</span>
              <span class="hidden md:inline">Tasks</span>
              <span class="hidden md:inline">Related ADRs</span>
              <span class="hidden md:inline">Created</span>
              <span />
            </div>

            <For each={workplanFiles()!.workplans}>
              {(wp) => (
                <div
                  class="grid grid-cols-[1fr_80px] md:grid-cols-[1fr_90px_70px_70px_150px_120px_80px] gap-2 border-b border-gray-800/50 px-4 py-3 text-sm transition-colors hover:bg-gray-900/30 cursor-pointer"
                  onClick={() => {
                    const pid = (route() as any).projectId ?? "";
                    const wpId = wp.id || wp.file.replace(/\.json$/, "");
                    if (pid) {
                      navigate({ page: "project-workplan-detail", projectId: pid, workplanId: wpId });
                    }
                  }}
                >
                  {/* File name */}
                  <div class="flex items-center gap-2">
                    <span class="h-2 w-2 shrink-0 rounded-full bg-gray-600" />
                    <span class="truncate font-medium text-gray-200" title={wp.file}>
                      {wp.title || wp.file.replace(/\.json$/, "")}
                    </span>
                  </div>

                  {/* Priority badge */}
                  <div>
                    <Show when={wp.priority} fallback={<span class="text-xs text-gray-600">--</span>}>
                      <span
                        class="inline-block rounded-full px-2 py-0.5 text-[10px] font-medium"
                        classList={{
                          "bg-red-900/30 text-red-400": wp.priority === "critical",
                          "bg-orange-900/30 text-orange-400": wp.priority === "high",
                          "bg-yellow-900/30 text-yellow-400": wp.priority === "medium",
                          "bg-gray-800 text-gray-400": wp.priority === "low" || (wp.priority !== "critical" && wp.priority !== "high" && wp.priority !== "medium"),
                        }}
                      >
                        {wp.priority}
                      </span>
                    </Show>
                  </div>

                  {/* Phase count */}
                  <span class="hidden md:inline text-xs text-gray-400">
                    {wp.phases}
                  </span>

                  {/* Task count */}
                  <span class="hidden md:inline text-xs text-gray-400">
                    {wp.tasks}
                  </span>

                  {/* Related ADRs */}
                  <div class="hidden md:flex items-center gap-1 overflow-hidden">
                    <For each={wp.related_adrs?.slice(0, 3) ?? []}>
                      {(adr) => (
                        <span class="shrink-0 rounded bg-gray-800 px-1.5 py-0.5 text-[10px] font-mono text-cyan-300">
                          {adr}
                        </span>
                      )}
                    </For>
                    <Show when={(wp.related_adrs?.length ?? 0) > 3}>
                      <span class="text-[10px] text-gray-500">
                        +{wp.related_adrs!.length - 3}
                      </span>
                    </Show>
                    <Show when={!wp.related_adrs?.length}>
                      <span class="text-[10px] text-gray-600">--</span>
                    </Show>
                  </div>

                  {/* Created date */}
                  <span class="hidden md:inline text-xs text-gray-500">
                    {wp.created_at
                      ? new Date(wp.created_at).toLocaleDateString(undefined, {
                          month: "short",
                          day: "numeric",
                        })
                      : "--"}
                  </span>

                  {/* Execute button */}
                  <div>
                    <button
                      class="rounded-lg border border-green-800/50 bg-green-900/20 px-2.5 py-1 text-[10px] font-medium text-green-400 transition-colors hover:border-green-700 hover:bg-green-900/40 disabled:opacity-50"
                      disabled={actionLoading()}
                      onClick={() => handleExecuteFile(wp.file)}
                    >
                      Execute
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </section>
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
      <div class="mt-4 flex flex-wrap gap-x-6 gap-y-1 border-t border-gray-800/50 pt-3 text-[11px] text-gray-500">
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
