/**
 * ResearchLab.tsx — Research Lab dashboard view with 4 panels:
 *   1. Frontier Leaderboard
 *   2. Experiment Timeline
 *   3. Config Explorer
 *   4. Strategy Weights
 */
import { Component, For, Show, onMount, onCleanup, createMemo } from "solid-js";
import {
  configs,
  experiments,
  frontier,
  strategies,
  loading,
  error,
  startNeuralLabPoll,
  stopNeuralLabPoll,
} from "../../stores/neural-lab";

// ── Status badge colors ──────────────────────────────────────────────────────

function statusColor(status: string): string {
  switch (status) {
    case "kept":
      return "bg-green-500/20 text-green-400 border-green-500/30";
    case "discarded":
      return "bg-red-500/20 text-red-400 border-red-500/30";
    case "failed":
      return "bg-gray-500/20 text-gray-400 border-gray-500/30";
    case "queued":
      return "bg-yellow-500/20 text-yellow-400 border-yellow-500/30";
    case "training":
      return "bg-blue-500/20 text-blue-400 border-blue-500/30";
    default:
      return "bg-gray-500/20 text-gray-400 border-gray-500/30";
  }
}

function configStatusColor(status: string): string {
  switch (status) {
    case "active":
    case "frontier":
      return "bg-green-500/20 text-green-400";
    case "retired":
    case "discarded":
      return "bg-red-500/20 text-red-400";
    case "training":
      return "bg-blue-500/20 text-blue-400";
    default:
      return "bg-gray-500/20 text-gray-400";
  }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function formatBpb(val: number | null): string {
  if (val === null) return "--";
  return val.toFixed(4);
}

function formatTime(secs: number | null): string {
  if (secs === null) return "--";
  if (secs < 60) return `${secs.toFixed(0)}s`;
  if (secs < 3600) return `${(secs / 60).toFixed(1)}m`;
  return `${(secs / 3600).toFixed(1)}h`;
}

function formatImprovement(val: number | null): string {
  if (val === null) return "--";
  const sign = val > 0 ? "+" : "";
  return `${sign}${(val * 100).toFixed(2)}%`;
}

// ── Panel: Frontier Leaderboard ──────────────────────────────────────────────

const FrontierLeaderboard: Component = () => {
  return (
    <div class="rounded-lg border border-gray-700 bg-gray-900/50 p-4">
      <div class="flex items-center gap-2 mb-3">
        <svg class="h-4 w-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M12 2l3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01L12 2z" />
        </svg>
        <h3 class="text-sm font-semibold text-gray-100">Frontier Leaderboard</h3>
      </div>

      <Show when={frontier().length > 0} fallback={
        <p class="text-xs text-gray-500 py-4 text-center">No lineages found</p>
      }>
        <div class="overflow-x-auto">
          <table class="w-full text-xs">
            <thead>
              <tr class="text-gray-400 border-b border-gray-700/50">
                <th class="text-left py-2 px-2 font-medium">Lineage</th>
                <th class="text-right py-2 px-2 font-medium">Best BPB</th>
                <th class="text-right py-2 px-2 font-medium">Experiments</th>
                <th class="text-right py-2 px-2 font-medium">Kept</th>
                <th class="text-right py-2 px-2 font-medium">Discarded</th>
                <th class="text-right py-2 px-2 font-medium">Keep Rate</th>
              </tr>
            </thead>
            <tbody>
              <For each={frontier()}>
                {(entry) => {
                  const keepRate = () =>
                    entry.total_experiments > 0
                      ? ((entry.kept_count / entry.total_experiments) * 100).toFixed(0)
                      : "0";
                  return (
                    <tr class="border-b border-gray-800/50 hover:bg-gray-800/30 transition-colors">
                      <td class="py-2 px-2">
                        <span class="font-medium text-cyan-300">{entry.lineage}</span>
                      </td>
                      <td class="py-2 px-2 text-right font-mono text-green-400">
                        {formatBpb(entry.best_val_bpb)}
                      </td>
                      <td class="py-2 px-2 text-right text-gray-300">{entry.total_experiments}</td>
                      <td class="py-2 px-2 text-right text-green-400">{entry.kept_count}</td>
                      <td class="py-2 px-2 text-right text-red-400">{entry.discarded_count}</td>
                      <td class="py-2 px-2 text-right">
                        <span class="inline-block rounded-full px-2 py-0.5 text-[10px] font-medium bg-gray-800 text-gray-300">
                          {keepRate()}%
                        </span>
                      </td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
};

// ── Panel: Experiment Timeline ───────────────────────────────────────────────

const ExperimentTimeline: Component = () => {
  const sorted = createMemo(() =>
    [...experiments()].sort(
      (a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
    ),
  );

  return (
    <div class="rounded-lg border border-gray-700 bg-gray-900/50 p-4">
      <div class="flex items-center gap-2 mb-3">
        <svg class="h-4 w-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" />
        </svg>
        <h3 class="text-sm font-semibold text-gray-100">Experiment Timeline</h3>
        <span class="ml-auto text-[10px] text-gray-500">{experiments().length} total</span>
      </div>

      <Show when={sorted().length > 0} fallback={
        <p class="text-xs text-gray-500 py-4 text-center">No experiments yet</p>
      }>
        <div class="space-y-1.5 max-h-[360px] overflow-y-auto pr-1">
          <For each={sorted()}>
            {(exp) => (
              <div class="flex items-start gap-3 rounded-md border border-gray-800/50 bg-gray-800/20 px-3 py-2 hover:bg-gray-800/40 transition-colors">
                <span
                  class={`mt-0.5 inline-block shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-medium ${statusColor(exp.status)}`}
                >
                  {exp.status}
                </span>
                <div class="min-w-0 flex-1">
                  <p class="text-xs text-gray-200 truncate">{exp.hypothesis || "No hypothesis"}</p>
                  <div class="mt-1 flex items-center gap-3 text-[10px] text-gray-500">
                    <span>
                      BPB: <span class="text-gray-300 font-mono">{formatBpb(exp.val_bpb)}</span>
                    </span>
                    <span>
                      Imp: <span class={exp.improvement !== null && exp.improvement > 0 ? "text-green-400" : "text-gray-300"}>
                        {formatImprovement(exp.improvement)}
                      </span>
                    </span>
                    <span>
                      Time: <span class="text-gray-300">{formatTime(exp.wall_time_secs)}</span>
                    </span>
                  </div>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

// ── Panel: Config Explorer ───────────────────────────────────────────────────

const ConfigExplorer: Component = () => {
  return (
    <div class="rounded-lg border border-gray-700 bg-gray-900/50 p-4">
      <div class="flex items-center gap-2 mb-3">
        <svg class="h-4 w-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
        <h3 class="text-sm font-semibold text-gray-100">Config Explorer</h3>
        <span class="ml-auto text-[10px] text-gray-500">{configs().length} configs</span>
      </div>

      <Show when={configs().length > 0} fallback={
        <p class="text-xs text-gray-500 py-4 text-center">No configs found</p>
      }>
        <div class="overflow-x-auto">
          <table class="w-full text-xs">
            <thead>
              <tr class="text-gray-400 border-b border-gray-700/50">
                <th class="text-left py-2 px-2 font-medium">Name</th>
                <th class="text-right py-2 px-2 font-medium">Layers</th>
                <th class="text-right py-2 px-2 font-medium">Heads</th>
                <th class="text-right py-2 px-2 font-medium">Embed</th>
                <th class="text-left py-2 px-2 font-medium">Lineage</th>
                <th class="text-left py-2 px-2 font-medium">Status</th>
              </tr>
            </thead>
            <tbody>
              <For each={configs()}>
                {(cfg) => (
                  <tr class="border-b border-gray-800/50 hover:bg-gray-800/30 transition-colors">
                    <td class="py-2 px-2">
                      <span class="font-medium text-gray-200">{cfg.name}</span>
                      <Show when={cfg.parent_id}>
                        <span class="ml-1 text-[10px] text-gray-600" title={`Parent: ${cfg.parent_id}`}>
                          (child)
                        </span>
                      </Show>
                    </td>
                    <td class="py-2 px-2 text-right font-mono text-gray-300">{cfg.n_layer}</td>
                    <td class="py-2 px-2 text-right font-mono text-gray-300">{cfg.n_head}</td>
                    <td class="py-2 px-2 text-right font-mono text-gray-300">{cfg.n_embd}</td>
                    <td class="py-2 px-2 text-cyan-300">{cfg.lineage}</td>
                    <td class="py-2 px-2">
                      <span class={`inline-block rounded-full px-2 py-0.5 text-[10px] font-medium ${configStatusColor(cfg.status)}`}>
                        {cfg.status}
                      </span>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
};

// ── Panel: Strategy Weights ──────────────────────────────────────────────────

const StrategyWeights: Component = () => {
  const maxWeight = createMemo(() =>
    Math.max(...strategies().map((s) => s.selection_weight), 1),
  );

  return (
    <div class="rounded-lg border border-gray-700 bg-gray-900/50 p-4">
      <div class="flex items-center gap-2 mb-3">
        <svg class="h-4 w-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="18" y1="20" x2="18" y2="10" /><line x1="12" y1="20" x2="12" y2="4" />
          <line x1="6" y1="20" x2="6" y2="14" />
        </svg>
        <h3 class="text-sm font-semibold text-gray-100">Strategy Weights</h3>
      </div>

      <Show when={strategies().length > 0} fallback={
        <p class="text-xs text-gray-500 py-4 text-center">No strategies found</p>
      }>
        <div class="space-y-3">
          <For each={strategies()}>
            {(strat) => {
              const pct = () => ((strat.selection_weight / maxWeight()) * 100).toFixed(0);
              return (
                <div>
                  <div class="flex items-center justify-between mb-1">
                    <span class="text-xs font-medium text-gray-200">{strat.name}</span>
                    <div class="flex items-center gap-3 text-[10px] text-gray-500">
                      <span>
                        Success: <span class="text-green-400">{(strat.success_rate * 100).toFixed(0)}%</span>
                      </span>
                      <span>
                        Tried: <span class="text-gray-300">{strat.total_tried}</span>
                      </span>
                      <span class="font-mono text-gray-300">{strat.selection_weight.toFixed(2)}</span>
                    </div>
                  </div>
                  <div class="h-2 w-full rounded-full bg-gray-800">
                    <div
                      class="h-2 rounded-full bg-gradient-to-r from-cyan-500 to-blue-500 transition-all duration-300"
                      style={{ width: `${pct()}%` }}
                    />
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

// ── Main View ────────────────────────────────────────────────────────────────

const ResearchLab: Component = () => {
  onMount(() => {
    startNeuralLabPoll(10000);
  });

  onCleanup(() => {
    stopNeuralLabPoll();
  });

  return (
    <div class="flex-1 overflow-y-auto p-4 md:p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div class="flex items-center gap-3">
          <svg class="h-6 w-6 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M9.75 17L9 20l-1 1h8l-1-1-.75-3M3 13h18M5 17h14a2 2 0 002-2V5a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
          </svg>
          <div>
            <h1 class="text-lg font-bold text-gray-100">Research Lab</h1>
            <p class="text-xs text-gray-500">Neural architecture search experiments</p>
          </div>
        </div>
        <Show when={loading()}>
          <span class="flex items-center gap-2 text-xs text-gray-500">
            <span class="h-2 w-2 rounded-full bg-cyan-400 animate-pulse" />
            Refreshing...
          </span>
        </Show>
      </div>

      {/* Error banner */}
      <Show when={error()}>
        <div class="mb-4 rounded-lg border border-red-800/50 bg-red-950/30 px-4 py-2.5 text-xs text-red-400">
          {error()}
        </div>
      </Show>

      {/* 4-panel grid */}
      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <FrontierLeaderboard />
        <ExperimentTimeline />
        <ConfigExplorer />
        <StrategyWeights />
      </div>
    </div>
  );
};

export default ResearchLab;
