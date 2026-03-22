import { Component, Show, For, createSignal } from "solid-js";
import { healthData, healthLoading, fetchHealth } from "../../stores/health";

const RING_RADIUS = 52;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS; // ~326.73

function scoreColor(score: number): string {
  if (score >= 80) return "#22c55e"; // green-500
  if (score >= 60) return "#eab308"; // yellow-500
  return "#ef4444"; // red-500
}

function scoreTextColor(score: number): string {
  if (score >= 80) return "text-green-400";
  if (score >= 60) return "text-yellow-400";
  return "text-red-400";
}

function severityBadge(severity: string): string {
  switch (severity.toLowerCase()) {
    case "error":
      return "bg-red-900/40 text-red-400";
    case "warning":
      return "bg-yellow-900/40 text-yellow-400";
    default:
      return "bg-gray-800 text-gray-400";
  }
}

const StatBox: Component<{ label: string; value: number | undefined; warn?: boolean }> = (
  props,
) => (
  <div class="flex flex-col items-center rounded-lg border border-gray-800 bg-gray-900 px-3 py-2">
    <span
      class="text-lg font-bold font-mono"
      classList={{
        "text-gray-200": !props.warn,
        "text-red-400": !!props.warn,
      }}
    >
      {props.value ?? 0}
    </span>
    <span class="text-[10px] uppercase tracking-wider text-gray-500">
      {props.label}
    </span>
  </div>
);

const HealthPane: Component = () => {
  const [violationsOpen, setViolationsOpen] = createSignal(true);
  const [unusedOpen, setUnusedOpen] = createSignal(false);

  const data = healthData;
  const loading = healthLoading;

  return (
    <div class="flex flex-col gap-4 p-4">
      {/* Header */}
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-gray-200">
          Architecture Health
        </h2>
        <button
          class="rounded border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors disabled:opacity-50"
          onClick={() => fetchHealth()}
          disabled={loading()}
        >
          <Show when={loading()} fallback="Re-analyze">
            <span class="animate-pulse">Analyzing...</span>
          </Show>
        </button>
      </div>

      {/* Loading state */}
      <Show when={loading() && !data()}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg
            class="h-8 w-8 animate-spin text-cyan-400"
            viewBox="0 0 24 24"
            fill="none"
          >
            <circle
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              stroke-width="3"
              stroke-dasharray="31.4 31.4"
              stroke-linecap="round"
            />
          </svg>
          <span class="mt-3 text-xs">Running architecture analysis...</span>
        </div>
      </Show>

      {/* No data yet */}
      <Show when={!loading() && !data()}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg
            class="h-10 w-10 text-gray-700"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
          </svg>
          <p class="mt-3 text-xs">No analysis data yet.</p>
          <button
            class="mt-2 rounded bg-cyan-600 px-4 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 transition-colors"
            onClick={() => fetchHealth()}
          >
            Run Analysis
          </button>
        </div>
      </Show>

      {/* Results */}
      <Show when={data()}>
        {(d) => {
          const score = () => d().health_score;
          const dashOffset = () =>
            RING_CIRCUMFERENCE - (RING_CIRCUMFERENCE * score()) / 100;

          return (
            <>
              {/* Ring chart + score */}
              <div class="flex items-center gap-6">
                <div class="relative flex-shrink-0">
                  <svg viewBox="0 0 120 120" width="120" height="120">
                    {/* Background ring */}
                    <circle
                      cx="60"
                      cy="60"
                      r={RING_RADIUS}
                      fill="none"
                      stroke="var(--bg-elevated)"
                      stroke-width="8"
                    />
                    {/* Score ring */}
                    <circle
                      cx="60"
                      cy="60"
                      r={RING_RADIUS}
                      fill="none"
                      stroke={scoreColor(score())}
                      stroke-width="8"
                      stroke-linecap="round"
                      stroke-dasharray={RING_CIRCUMFERENCE}
                      stroke-dashoffset={dashOffset()}
                      transform="rotate(-90 60 60)"
                      class="transition-[stroke-dashoffset] duration-[0.6s] ease-out"
                    />
                  </svg>
                  {/* Center text */}
                  <div class="absolute inset-0 flex flex-col items-center justify-center">
                    <span
                      class={`text-2xl font-bold font-mono ${scoreTextColor(score())}`}
                    >
                      {score()}
                    </span>
                    <span class="text-[10px] text-gray-500">/ 100</span>
                  </div>
                </div>

                {/* Summary */}
                {(() => {
                  const raw = d() as any;
                  const vCount = raw.violation_count ?? (Array.isArray(raw.violations) ? raw.violations.length : 0);
                  const dCount = raw.dead_export_count ?? (Array.isArray(raw.dead_exports) ? raw.dead_exports.length : 0);
                  const cCount = raw.circular_dep_count ?? (Array.isArray(raw.circular_deps) ? raw.circular_deps.length : 0);
                  return (
                    <div class="flex flex-col gap-1 text-xs">
                      <Show when={vCount === 0}>
                        <span class="text-green-400">No violations found</span>
                      </Show>
                      <Show when={vCount > 0}>
                        <span class="text-red-400">{vCount} violation{vCount !== 1 ? "s" : ""} detected</span>
                      </Show>
                      <Show when={dCount > 0}>
                        <span class="text-yellow-400">{dCount} dead export{dCount !== 1 ? "s" : ""}</span>
                      </Show>
                      <Show when={cCount > 0}>
                        <span class="text-yellow-400">{cCount} circular dep{cCount !== 1 ? "s" : ""}</span>
                      </Show>
                      <span class="text-gray-500">{raw.file_count ?? 0} files, {raw.edge_count ?? 0} edges</span>
                    </div>
                  );
                })()}
              </div>

              {/* Stats grid */}
              {(() => {
                const raw = d() as any;
                const fileCount = raw.file_count ?? 0;
                const edgeCount = raw.edge_count ?? 0;
                const violationCount = raw.violation_count ?? (Array.isArray(raw.violations) ? raw.violations.length : 0);
                const unusedPortCount = raw.unused_port_count ?? (Array.isArray(raw.unused_ports) ? raw.unused_ports.length : 0);
                const deadExportCount = raw.dead_export_count ?? (Array.isArray(raw.dead_exports) ? raw.dead_exports.length : 0);
                const circularDepCount = raw.circular_dep_count ?? (Array.isArray(raw.circular_deps) ? raw.circular_deps.length : 0);
                return (
                  <div class="grid grid-cols-3 gap-2">
                    <StatBox label="Files" value={fileCount} />
                    <StatBox label="Edges" value={edgeCount} />
                    <StatBox label="Violations" value={violationCount} warn={violationCount > 0} />
                    <StatBox label="Unused Ports" value={unusedPortCount} warn={unusedPortCount > 0} />
                    <StatBox label="Dead Exports" value={deadExportCount} warn={deadExportCount > 0} />
                    <StatBox label="Circular Deps" value={circularDepCount} warn={circularDepCount > 0} />
                  </div>
                );
              })()}

              {/* Violations list */}
              <Show when={d().violations && d().violations.length > 0}>
                <div class="rounded-lg border border-gray-800 bg-gray-950">
                  <button
                    class="flex w-full items-center justify-between px-3 py-2 text-xs font-semibold text-gray-300 hover:bg-gray-900 transition-colors"
                    onClick={() => setViolationsOpen(!violationsOpen())}
                  >
                    <span>
                      Violations ({d().violations.length})
                    </span>
                    <svg
                      class="h-3 w-3 transition-transform"
                      classList={{ "rotate-180": violationsOpen() }}
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2.5"
                    >
                      <polyline points="6 9 12 15 18 9" />
                    </svg>
                  </button>
                  <Show when={violationsOpen()}>
                    <div class="border-t border-gray-800 divide-y divide-gray-800/50">
                      <For each={d().violations}>
                        {(v) => (
                          <div class="px-3 py-2 text-xs">
                            <div class="flex items-center gap-2">
                              <span class="font-mono text-gray-300 truncate">
                                {v.file}
                              </span>
                              <span
                                class={`ml-auto shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${severityBadge(v.severity)}`}
                              >
                                {v.severity}
                              </span>
                            </div>
                            <p class="mt-1 text-gray-500">{v.message}</p>
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>
              </Show>

              {/* Unused ports list */}
              <Show when={d().unused_ports && d().unused_ports.length > 0}>
                <div class="rounded-lg border border-gray-800 bg-gray-950">
                  <button
                    class="flex w-full items-center justify-between px-3 py-2 text-xs font-semibold text-gray-300 hover:bg-gray-900 transition-colors"
                    onClick={() => setUnusedOpen(!unusedOpen())}
                  >
                    <span>
                      Unused Ports ({d().unused_ports.length})
                    </span>
                    <svg
                      class="h-3 w-3 transition-transform"
                      classList={{ "rotate-180": unusedOpen() }}
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2.5"
                    >
                      <polyline points="6 9 12 15 18 9" />
                    </svg>
                  </button>
                  <Show when={unusedOpen()}>
                    <div class="border-t border-gray-800 divide-y divide-gray-800/50">
                      <For each={d().unused_ports}>
                        {(p) => (
                          <div class="flex items-center gap-2 px-3 py-2 text-xs">
                            <span class="font-mono text-yellow-400">
                              {p.name}
                            </span>
                            <span class="ml-auto text-gray-500 truncate max-w-[180px]">
                              {p.file}
                            </span>
                          </div>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>
              </Show>
            </>
          );
        }}
      </Show>
    </div>
  );
};

export default HealthPane;
