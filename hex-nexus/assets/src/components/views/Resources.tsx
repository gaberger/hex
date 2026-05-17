/**
 * Resources.tsx — system resource utilisation (ADR-2026-05-08-2200).
 *
 * Reads /api/resources (process_observation) and /api/resources/anomalies
 * (resource_anomaly). Mirrors `top` for the hex fleet plus an anomaly
 * inbox the operator can ack from the dashboard.
 */

import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";

interface ProcessRow {
  pid: number;
  host: string;
  argv_sha: string;
  argv_first: string;
  state: string;
  ppid: number;
  started_micros: number;
  rss_kb: number;
  cpu_pct: number;
  observed_at: string;
}

interface AnomalyRow {
  id: number;
  detected_at: string;
  kind: string;
  severity: string;
  pids: string;
  note: string;
  handled: boolean;
  handled_at: string;
  handled_by: string;
}

const REFRESH_MS = 5000;

const fmtRss = (kb: number): string => {
  if (kb >= 1024 * 1024) return `${(kb / 1024 / 1024).toFixed(1)} GiB`;
  if (kb >= 1024) return `${(kb / 1024).toFixed(0)} MiB`;
  return `${kb} KiB`;
};

const sevColor = (s: string) => {
  switch (s) {
    case "critical":
      return "bg-red-900 text-red-300 border-red-700";
    case "warn":
      return "bg-yellow-900 text-yellow-300 border-yellow-700";
    case "info":
      return "bg-blue-900 text-blue-300 border-blue-700";
    default:
      return "bg-gray-800 text-gray-300 border-gray-700";
  }
};

const stateColor = (s: string): string => {
  switch (s) {
    case "Z":
      return "text-red-400";
    case "D":
      return "text-orange-400";
    case "R":
      return "text-green-400";
    case "S":
      return "text-gray-400";
    default:
      return "text-gray-500";
  }
};

const stateLabel = (s: string): string => {
  switch (s) {
    case "R": return "running";
    case "S": return "sleep";
    case "D": return "uninterruptible";
    case "Z": return "zombie";
    case "T": return "stopped";
    default:  return s || "?";
  }
};

const Resources: Component = () => {
  const [processes, setProcesses] = createSignal<ProcessRow[]>([]);
  const [anomalies, setAnomalies] = createSignal<AnomalyRow[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [busyId, setBusyId] = createSignal<number | null>(null);

  let timer: ReturnType<typeof setInterval> | null = null;

  const refresh = async () => {
    try {
      const [procResp, anomResp] = await Promise.all([
        restClient.get("/api/resources"),
        restClient.get("/api/resources/anomalies?status=open&limit=100"),
      ]);
      setProcesses(procResp.processes || []);
      setAnomalies(anomResp.anomalies || []);
      setError(null);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    refresh();
    timer = setInterval(refresh, REFRESH_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  const ack = async (id: number) => {
    setBusyId(id);
    try {
      await restClient.post("/api/resources/anomalies/ack", { id, handled_by: "dashboard" });
      await refresh();
    } catch (e: any) {
      setError(`ack failed: ${e?.message || String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const totalRssGib = () =>
    processes().reduce((acc, p) => acc + p.rss_kb / 1024 / 1024, 0);

  return (
    <div class="flex flex-col bg-gray-950 min-h-screen text-gray-100">
      <div class="p-6 border-b border-gray-800">
        <div class="flex items-baseline justify-between">
          <div>
            <h1 class="text-2xl font-bold mb-1">Resources</h1>
            <p class="text-gray-400 text-sm">
              ADR-2026-05-08-2200 · /proc walker every 15s · supervisor tick every 60s
            </p>
          </div>
          <div class="text-xs text-gray-500">
            {processes().length} processes · {totalRssGib().toFixed(1)} GiB RSS · refresh {REFRESH_MS / 1000}s
          </div>
        </div>
      </div>

      <Show when={error()}>
        <div class="p-4 bg-red-950/40 border-b border-red-900 text-red-300 text-sm">
          {error()}
        </div>
      </Show>

      {/* Anomalies */}
      <div class="px-6 pt-4">
        <h2 class="text-sm uppercase text-gray-500 tracking-wide mb-2">
          Anomalies <span class="text-gray-600">({anomalies().length} open)</span>
        </h2>
        <Show
          when={anomalies().length > 0}
          fallback={
            <div class="text-gray-500 text-sm p-3 border border-gray-900 rounded bg-gray-900/30">
              No open anomalies. Supervisor is satisfied.
            </div>
          }
        >
          <div class="space-y-2">
            <For each={anomalies()}>
              {(a) => (
                <div class="flex items-start gap-3 p-3 border border-gray-800 rounded bg-gray-900/40">
                  <span
                    class={`shrink-0 px-2 py-0.5 rounded text-xs border ${sevColor(a.severity)}`}
                  >
                    {a.severity}
                  </span>
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2 text-xs text-gray-400">
                      <span class="text-cyan-400 font-mono">{a.kind}</span>
                      <span class="text-gray-500">pids {a.pids}</span>
                      <span class="text-gray-600 ml-auto">#{a.id}</span>
                    </div>
                    <div class="text-sm text-gray-200 mt-1">{a.note}</div>
                  </div>
                  <button
                    class="shrink-0 px-3 py-1 rounded bg-gray-800 hover:bg-gray-700 text-gray-200 text-xs border border-gray-700 disabled:opacity-50"
                    disabled={busyId() === a.id}
                    onClick={() => ack(a.id)}
                  >
                    Ack
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>

      {/* Process table */}
      <div class="px-6 pt-6 pb-4 flex-1 overflow-y-auto">
        <h2 class="text-sm uppercase text-gray-500 tracking-wide mb-2">Processes</h2>
        <Show when={loading() && processes().length === 0}>
          <div class="text-gray-500 text-sm">Waiting for /proc walker…</div>
        </Show>
        <Show when={!loading() && processes().length === 0}>
          <div class="text-gray-500 text-sm">
            No observations. Is the observer enabled? (HEX_DISABLE_RESOURCE_OBSERVER unset)
          </div>
        </Show>
        <Show when={processes().length > 0}>
          <table class="w-full text-sm">
            <thead>
              <tr class="text-left text-gray-500 uppercase text-xs border-b border-gray-800">
                <th class="py-2 pr-4">pid</th>
                <th class="py-2 pr-4">state</th>
                <th class="py-2 pr-4 text-right">cpu %</th>
                <th class="py-2 pr-4 text-right">rss</th>
                <th class="py-2 pr-4">argv</th>
                <th class="py-2 pr-4">ppid</th>
              </tr>
            </thead>
            <tbody>
              <For each={processes()}>
                {(p) => {
                  const rssGib = p.rss_kb / 1024 / 1024;
                  const rssCls =
                    rssGib > 30
                      ? "text-red-400 font-semibold"
                      : rssGib > 20
                      ? "text-yellow-400"
                      : "text-gray-300";
                  const cpuCls =
                    p.cpu_pct > 800
                      ? "text-red-400 font-semibold"
                      : p.cpu_pct > 200
                      ? "text-yellow-400"
                      : "text-gray-300";
                  return (
                    <tr class="border-b border-gray-900/50 hover:bg-gray-900/30">
                      <td class="py-2 pr-4 font-mono text-cyan-400">{p.pid}</td>
                      <td class="py-2 pr-4">
                        <span class={stateColor(p.state)}>
                          {p.state}
                        </span>
                        <span class="text-xs text-gray-600 ml-1">{stateLabel(p.state)}</span>
                      </td>
                      <td class={`py-2 pr-4 text-right tabular-nums ${cpuCls}`}>
                        {p.cpu_pct.toFixed(0)}
                      </td>
                      <td class={`py-2 pr-4 text-right tabular-nums ${rssCls}`}>
                        {fmtRss(p.rss_kb)}
                      </td>
                      <td class="py-2 pr-4 text-gray-200 truncate max-w-2xl font-mono text-xs">
                        {p.argv_first}
                      </td>
                      <td class="py-2 pr-4 text-gray-500 font-mono">{p.ppid}</td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </Show>
      </div>
    </div>
  );
};

export default Resources;
