/**
 * DirectRuns.tsx — the monitor for the NEW execution model (ADR-2026-06-04-1740).
 *
 * The unit of work is now a direct run: task → one edit → evidence command →
 * commit. This view shows what the agents actually DID and whether it was
 * VERIFIED (evidence passed, commit landed) — replacing the retired liveness
 * signals (personas online / swarms "active" / commitments) that reported
 * activity without output. Polls GET /api/direct/runs.
 */
import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";

interface DirectRun {
  id: number;
  agent: string;
  started_at: string;
  instruction: string;
  file: string;
  model: string;
  ok: boolean;
  attempts: number;
  evidence_passed: boolean;
  committed: string | null;
  duration_ms: number;
  error: string | null;
}
interface RunsResponse {
  summary: { total: number; passed: number; failed: number; committed: number; pass_rate: number };
  runs: DirectRun[];
}

const Stat: Component<{ label: string; value: any; tone?: string }> = (p) => (
  <div class="px-4 py-3 rounded bg-gray-900/50 border border-gray-800 min-w-[96px]">
    <div
      class="text-2xl font-semibold"
      classList={{
        "text-green-400": p.tone === "green",
        "text-red-400": p.tone === "red",
        "text-cyan-400": p.tone === "cyan",
      }}
    >
      {p.value}
    </div>
    <div class="text-xs text-gray-500 mt-1">{p.label}</div>
  </div>
);

const DirectRuns: Component = () => {
  const [data, setData] = createSignal<RunsResponse | null>(null);
  const [err, setErr] = createSignal<string | null>(null);
  let timer: any;

  const load = async () => {
    try {
      const r = await restClient.get<RunsResponse>("/api/direct/runs");
      setData(r);
      setErr(null);
    } catch (e: any) {
      setErr(String(e?.message ?? e));
    }
  };

  onMount(() => {
    load();
    timer = setInterval(load, 4000);
  });
  onCleanup(() => clearInterval(timer));

  return (
    <div class="p-6 text-gray-100">
      <h1 class="text-xl font-semibold mb-1">Agent Runs</h1>
      <p class="text-sm text-gray-400 mb-4">
        What hex agents actually did — direct-executor, adr-steward, … — with the commit they produced. Verified output, not liveness.
      </p>
      <Show when={err()}>
        <div class="text-red-400 text-sm mb-3">{err()}</div>
      </Show>
      <Show when={data()} fallback={<div class="text-gray-500 text-sm">Loading…</div>}>
        {(d) => (
          <>
            <div class="flex gap-3 mb-5 flex-wrap">
              <Stat label="Runs" value={d().summary.total} />
              <Stat label="Passed" value={d().summary.passed} tone="green" />
              <Stat label="Failed" value={d().summary.failed} tone={d().summary.failed > 0 ? "red" : "gray"} />
              <Stat label="Committed" value={d().summary.committed} tone="cyan" />
              <Stat label="Pass rate" value={`${Math.round(d().summary.pass_rate * 100)}%`} tone="green" />
            </div>
            <div class="overflow-auto rounded border border-gray-800">
              <table class="w-full text-sm">
                <thead class="bg-gray-900/60 text-gray-400 text-xs uppercase">
                  <tr>
                    <th class="text-left px-3 py-2">#</th>
                    <th class="text-left px-3 py-2">Agent</th>
                    <th class="text-left px-3 py-2">Task</th>
                    <th class="text-left px-3 py-2">File</th>
                    <th class="text-left px-3 py-2">Evidence</th>
                    <th class="text-left px-3 py-2">Commit</th>
                    <th class="text-left px-3 py-2">Try</th>
                    <th class="text-left px-3 py-2">Dur</th>
                    <th class="text-left px-3 py-2">Model</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={d().runs}>
                    {(r) => (
                      <tr class="border-t border-gray-800/60 hover:bg-gray-900/40">
                        <td class="px-3 py-2 text-gray-500">{r.id}</td>
                        <td class="px-3 py-2"><span class="text-cyan-300 text-xs font-medium">{r.agent || "direct-executor"}</span></td>
                        <td class="px-3 py-2 max-w-md truncate" title={r.instruction}>{r.instruction}</td>
                        <td class="px-3 py-2 text-gray-400 font-mono text-xs">{r.file.split("/").pop()}</td>
                        <td class="px-3 py-2">
                          <span
                            classList={{
                              "text-green-400": r.evidence_passed,
                              "text-red-400": !r.evidence_passed,
                            }}
                          >
                            {r.evidence_passed ? "✓ pass" : "✗ fail"}
                          </span>
                        </td>
                        <td class="px-3 py-2 font-mono text-xs text-cyan-400">{r.committed ?? "—"}</td>
                        <td class="px-3 py-2 text-gray-400">{r.attempts}</td>
                        <td class="px-3 py-2 text-gray-400">{(r.duration_ms / 1000).toFixed(1)}s</td>
                        <td class="px-3 py-2 text-gray-500 text-xs">{r.model}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
              <Show when={d().runs.length === 0}>
                <div class="px-3 py-6 text-center text-gray-500 text-sm">
                  No direct runs yet. POST /api/direct/execute to run one.
                </div>
              </Show>
            </div>
          </>
        )}
      </Show>
    </div>
  );
};

export default DirectRuns;
