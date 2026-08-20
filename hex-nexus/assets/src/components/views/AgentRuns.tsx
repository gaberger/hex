import { Component, For, Show } from 'solid-js';

interface AgentRun {
  run_id: string;
  intent: string;
  started_at: string;
  iterations: number;
  steps: number;
  stop_reason: string;
  elapsed_ms: number;
}

interface AgentRunsProps {
  runs: AgentRun[];
}

const formatDuration = (ms: number): string => {
  if (ms < 1000) return `${ms}ms`;
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${seconds % 60}s`;
};

const getElapsedAge = (startedAt: string): string => {
  if (!startedAt) return '—';
  const t = Date.parse(startedAt);
  if (isNaN(t)) return '—';
  const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
};

const statusPill = (stopReason: string): string => {
  if (stopReason === 'finish') return 'bg-green-700 text-green-100';
  if (stopReason === 'max_iterations') return 'bg-amber-700 text-amber-100';
  return 'bg-red-800 text-red-100';
};

const truncate = (s: string, n: number): string =>
  s.length > n ? s.slice(0, n) + '…' : s;

const AgentRuns: Component<AgentRunsProps> = (props) => {
  return (
    <div class="space-y-3">
      <Show
        when={props.runs.length > 0}
        fallback={
          <div class="border border-gray-700 rounded bg-gray-900 p-6 text-center">
            <p class="text-gray-500 text-sm font-mono">
              No agent runs yet — try <span class="text-gray-300">hex agent run "..."</span>
            </p>
          </div>
        }
      >
        <For each={props.runs}>
          {(run) => (
            <div class="border border-gray-700 rounded bg-gray-900 p-3">
              <div class="flex items-center gap-2 text-xs">
                <span class="font-mono text-gray-400 bg-gray-800 px-2 py-0.5 rounded">
                  {run.run_id.slice(0, 8)}
                </span>
                <span class={`px-2 py-0.5 rounded ${statusPill(run.stop_reason)}`}>
                  {run.stop_reason}
                </span>
                <span class="text-gray-500 ml-auto">{getElapsedAge(run.started_at)}</span>
              </div>
              <p class="text-sm text-gray-100 mt-1">{truncate(run.intent, 100)}</p>
              <div class="flex items-center gap-4 text-xs text-gray-400 mt-1">
                <span>{run.iterations} iter</span>
                <span>{run.steps} steps</span>
                <span>{formatDuration(run.elapsed_ms)}</span>
              </div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default AgentRuns;
