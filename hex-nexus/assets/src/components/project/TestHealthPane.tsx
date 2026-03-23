/**
 * TestHealthPane.tsx — Test session history, trends, and flaky test display.
 *
 * Fetches test session data, per-category pass/fail trends, and flaky tests
 * from the hex-nexus REST API.
 */
import { Component, For, Show, createSignal, createResource } from "solid-js";
import { restClient } from "../../services/rest-client";

interface TestSession {
  id: string;
  commit: string;
  branch: string;
  passed: number;
  failed: number;
  skipped: number;
  status: string;
  duration_ms: number;
  timestamp: string;
}

interface CategoryTrend {
  category: string;
  results: Array<{ status: string }>;
}

interface FlakyTest {
  name: string;
  category: string;
  flake_rate: number;
}

function statusBadgeClass(status: string): string {
  switch (status.toLowerCase()) {
    case "passed":
    case "pass":
      return "bg-green-900/40 text-green-400";
    case "failed":
    case "fail":
      return "bg-red-900/40 text-red-400";
    default:
      return "bg-gray-800 text-gray-400";
  }
}

function shortHash(commit: string): string {
  return commit.substring(0, 7);
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1000)}s`;
}

function relativeTime(timestamp: string): string {
  const diff = Date.now() - new Date(timestamp).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

const TestHealthPane: Component = () => {
  const [sessionsOpen, setSessionsOpen] = createSignal(true);
  const [trendsOpen, setTrendsOpen] = createSignal(true);
  const [flakyOpen, setFlakyOpen] = createSignal(true);

  const [sessions, { refetch: refetchSessions }] = createResource(async () => {
    return restClient.get<TestSession[]>("/api/test-sessions?limit=10");
  });

  const [trends] = createResource(async () => {
    return restClient.get<CategoryTrend[]>("/api/test-sessions/trends");
  });

  const [flaky] = createResource(async () => {
    return restClient.get<FlakyTest[]>("/api/test-sessions/flaky");
  });

  const isLoading = () => sessions.loading && trends.loading && flaky.loading;
  const hasData = () => sessions() || trends() || flaky();

  return (
    <div class="flex flex-col gap-4 p-4">
      {/* Header */}
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-gray-200">Test Health</h2>
        <button
          class="rounded border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors disabled:opacity-50"
          onClick={() => refetchSessions()}
          disabled={sessions.loading}
        >
          <Show when={sessions.loading} fallback="Refresh">
            <span class="animate-pulse">Loading...</span>
          </Show>
        </button>
      </div>

      {/* Loading state */}
      <Show when={isLoading() && !hasData()}>
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
          <span class="mt-3 text-xs">Loading test data...</span>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!isLoading() && !hasData()}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg
            class="h-10 w-10 text-gray-700"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <path d="M9.75 3.104v5.714a2.25 2.25 0 01-.659 1.591L5 14.5M9.75 3.104c-.251.023-.501.05-.75.082m.75-.082a24.301 24.301 0 014.5 0m0 0v5.714c0 .597.237 1.17.659 1.591L19 14.5M14.25 3.104c.251.023.501.05.75.082M19 14.5l-2.47 2.47a3.375 3.375 0 01-2.386.988H9.856a3.375 3.375 0 01-2.386-.988L5 14.5m14 0V17a3 3 0 01-3 3H8a3 3 0 01-3-3v-2.5" />
          </svg>
          <p class="mt-3 text-xs">No test sessions recorded</p>
        </div>
      </Show>

      {/* Sessions table */}
      <Show when={sessions() && sessions()!.length > 0}>
        <div class="rounded-lg border border-gray-800 bg-gray-950">
          <button
            class="flex w-full items-center justify-between px-3 py-2 text-xs font-semibold text-gray-300 hover:bg-gray-900 transition-colors"
            onClick={() => setSessionsOpen(!sessionsOpen())}
          >
            <span>Sessions ({sessions()!.length})</span>
            <svg
              class="h-3 w-3 transition-transform"
              classList={{ "rotate-180": sessionsOpen() }}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
            >
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </button>
          <Show when={sessionsOpen()}>
            <div class="border-t border-gray-800">
              <table class="w-full text-xs">
                <thead>
                  <tr class="text-left text-[10px] uppercase tracking-wider text-gray-500">
                    <th class="px-3 py-1.5">Commit</th>
                    <th class="px-3 py-1.5">Branch</th>
                    <th class="px-3 py-1.5 text-right">Pass</th>
                    <th class="px-3 py-1.5 text-right">Fail</th>
                    <th class="px-3 py-1.5 text-right">Skip</th>
                    <th class="px-3 py-1.5 text-center">Status</th>
                    <th class="px-3 py-1.5 text-right">Duration</th>
                    <th class="px-3 py-1.5 text-right">When</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-gray-800/50">
                  <For each={sessions()}>
                    {(session) => (
                      <tr class="hover:bg-gray-900/50 transition-colors">
                        <td class="px-3 py-1.5 font-mono text-gray-300">
                          {shortHash(session.commit)}
                        </td>
                        <td class="px-3 py-1.5 truncate max-w-[120px] text-gray-400">
                          {session.branch}
                        </td>
                        <td class="px-3 py-1.5 text-right text-green-400">
                          {session.passed}
                        </td>
                        <td class="px-3 py-1.5 text-right"
                          classList={{
                            "text-red-400": session.failed > 0,
                            "text-gray-500": session.failed === 0,
                          }}
                        >
                          {session.failed}
                        </td>
                        <td class="px-3 py-1.5 text-right text-gray-500">
                          {session.skipped}
                        </td>
                        <td class="px-3 py-1.5 text-center">
                          <span
                            class={`rounded px-1.5 py-0.5 text-[10px] font-medium ${statusBadgeClass(session.status)}`}
                          >
                            {session.status}
                          </span>
                        </td>
                        <td class="px-3 py-1.5 text-right font-mono text-gray-400">
                          {formatDuration(session.duration_ms)}
                        </td>
                        <td class="px-3 py-1.5 text-right text-gray-500">
                          {relativeTime(session.timestamp)}
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </div>
      </Show>

      {/* Trends — per-category pass rate dots */}
      <Show when={trends() && trends()!.length > 0}>
        <div class="rounded-lg border border-gray-800 bg-gray-950">
          <button
            class="flex w-full items-center justify-between px-3 py-2 text-xs font-semibold text-gray-300 hover:bg-gray-900 transition-colors"
            onClick={() => setTrendsOpen(!trendsOpen())}
          >
            <span>Trends ({trends()!.length} categories)</span>
            <svg
              class="h-3 w-3 transition-transform"
              classList={{ "rotate-180": trendsOpen() }}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
            >
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </button>
          <Show when={trendsOpen()}>
            <div class="border-t border-gray-800 divide-y divide-gray-800/50">
              <For each={trends()}>
                {(trend) => (
                  <div class="flex items-center gap-3 px-3 py-2">
                    <span class="w-24 shrink-0 truncate text-xs text-gray-300">
                      {trend.category}
                    </span>
                    <div class="flex items-center gap-1">
                      <For each={trend.results}>
                        {(result) => (
                          <span
                            class="h-2.5 w-2.5 rounded-full"
                            classList={{
                              "bg-green-400": result.status.toLowerCase() === "pass" || result.status.toLowerCase() === "passed",
                              "bg-red-400": result.status.toLowerCase() === "fail" || result.status.toLowerCase() === "failed",
                              "bg-gray-600": result.status.toLowerCase() !== "pass" && result.status.toLowerCase() !== "passed" && result.status.toLowerCase() !== "fail" && result.status.toLowerCase() !== "failed",
                            }}
                            title={result.status}
                          />
                        )}
                      </For>
                    </div>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>

      {/* Flaky tests */}
      <Show when={flaky() && flaky()!.length > 0}>
        <div class="rounded-lg border border-gray-800 bg-gray-950">
          <button
            class="flex w-full items-center justify-between px-3 py-2 text-xs font-semibold text-gray-300 hover:bg-gray-900 transition-colors"
            onClick={() => setFlakyOpen(!flakyOpen())}
          >
            <span>
              Flaky Tests ({flaky()!.length})
            </span>
            <svg
              class="h-3 w-3 transition-transform"
              classList={{ "rotate-180": flakyOpen() }}
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
            >
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </button>
          <Show when={flakyOpen()}>
            <div class="border-t border-gray-800 divide-y divide-gray-800/50">
              <For each={flaky()}>
                {(test) => (
                  <div class="flex items-center gap-3 px-3 py-2 text-xs">
                    <span class="flex-1 truncate font-mono text-gray-300">
                      {test.name}
                    </span>
                    <span class="shrink-0 rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-400">
                      {test.category}
                    </span>
                    <span
                      class="shrink-0 font-mono"
                      classList={{
                        "text-red-400": test.flake_rate >= 50,
                        "text-yellow-400": test.flake_rate >= 20 && test.flake_rate < 50,
                        "text-gray-400": test.flake_rate < 20,
                      }}
                    >
                      {test.flake_rate.toFixed(0)}%
                    </span>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default TestHealthPane;
