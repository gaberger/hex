/**
 * OpsSla.tsx — Operator-Acceptance SLA tile.
 *
 * Single-pane view of whether the AIOS is serving the operator:
 *   asks_total / replied / silent / silent-rate / stub-rate / per-persona breakdown.
 *
 * Reads /api/ops-sla which classifies every ceo→persona ask against the
 * matching persona→ceo replies and reports aggregate + per-role stats.
 * Refreshes every 8 seconds.
 *
 * Closes docs/specs/operator-acceptance-sla.md instrumentation gap.
 */

import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";

interface PersonaSla {
  persona: string;
  asks: number;
  replied: number;
  silent: number;
}
interface SilentAsk {
  id: number;
  to: string;
  content: string;
}
interface SlaResponse {
  window_hours: number;
  asks_total: number;
  replied: number;
  silent: number;
  silent_rate: number;   // 0..1
  stub_rate: number;     // 0..1
  by_persona: PersonaSla[];
  latest_silent: SilentAsk[];
}

const REFRESH_MS = 8000;

const pct = (n: number): string => `${Math.round(n * 100)}%`;

const OpsSla: Component = () => {
  const [data, setData] = createSignal<SlaResponse | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  const fetchSla = async () => {
    try {
      const resp: any = await restClient.get("/api/ops-sla?window_hours=24&limit=400");
      setData(resp);
      setError(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    fetchSla();
    timer = setInterval(fetchSla, REFRESH_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  // Color the silent-rate against the SLA targets in the spec:
  //   < 5%  green   (target)
  //   < 15% amber   (watch)
  //   else  red     (breach)
  const silentClass = () => {
    const r = data()?.silent_rate ?? 0;
    if (r < 0.05) return "text-emerald-400";
    if (r < 0.15) return "text-amber-400";
    return "text-red-400";
  };

  return (
    <div class="h-screen flex flex-col bg-gray-950 text-gray-100 p-6">
      <div class="mb-4">
        <h1 class="text-2xl font-bold text-white">Operator-Acceptance SLA</h1>
        <p class="text-sm text-gray-400 mt-1">
          docs/specs/operator-acceptance-sla.md · target silent-rate &lt; 5% · refresh 8s
        </p>
      </div>

      <Show when={error()}>
        <div class="bg-red-950 border border-red-700 rounded-lg p-4 text-red-200 mb-4">
          {error()}
        </div>
      </Show>

      <Show when={loading() && !data()}>
        <div class="text-gray-400">Loading…</div>
      </Show>

      <Show when={data()}>
        {(d) => (
          <div class="space-y-4">
            {/* Top metrics row */}
            <div class="grid grid-cols-4 gap-3">
              <Stat label="Asks (last 24h)" value={d().asks_total.toString()} />
              <Stat label="Replied" value={d().replied.toString()} valueClass="text-emerald-300" />
              <Stat label="Silent" value={d().silent.toString()} valueClass="text-red-300" />
              <Stat label="Silent rate" value={pct(d().silent_rate)} valueClass={silentClass()} />
            </div>

            {/* Secondary row */}
            <div class="grid grid-cols-4 gap-3">
              <Stat
                label="Stub / escalated"
                value={pct(d().stub_rate)}
                valueClass={d().stub_rate > 0.2 ? "text-amber-400" : "text-gray-200"}
                hint="of replies that were stub or escalation"
              />
              <Stat label="Personas active" value={d().by_persona.length.toString()} />
              <Stat label="Window" value={`${d().window_hours}h`} />
              <Stat
                label="SLA"
                value={d().silent_rate < 0.05 ? "✓ green" : d().silent_rate < 0.15 ? "watch" : "breach"}
                valueClass={silentClass()}
              />
            </div>

            {/* Per-persona table */}
            <div class="bg-gray-900 border border-gray-700 rounded-lg overflow-hidden">
              <div class="px-4 py-2 border-b border-gray-700 uppercase tracking-wider text-gray-300 text-xs font-semibold">
                Per persona
              </div>
              <Show when={d().by_persona.length > 0} fallback={
                <div class="p-4 text-sm text-gray-400 italic">No persona traffic in window.</div>
              }>
                <table class="w-full text-sm">
                  <thead class="bg-gray-950 text-xs uppercase tracking-wide text-gray-400">
                    <tr>
                      <th class="px-3 py-2 text-left">Persona</th>
                      <th class="px-3 py-2 text-right">Asks</th>
                      <th class="px-3 py-2 text-right">Replied</th>
                      <th class="px-3 py-2 text-right">Silent</th>
                      <th class="px-3 py-2 text-right">Silent %</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-gray-800">
                    <For each={d().by_persona}>
                      {(p) => {
                        const rate = p.asks === 0 ? 0 : p.silent / p.asks;
                        return (
                          <tr class="hover:bg-gray-800">
                            <td class="px-3 py-1.5 text-gray-100">{p.persona}</td>
                            <td class="px-3 py-1.5 text-right font-mono text-gray-300">{p.asks}</td>
                            <td class="px-3 py-1.5 text-right font-mono text-emerald-300">{p.replied}</td>
                            <td class="px-3 py-1.5 text-right font-mono text-red-300">{p.silent}</td>
                            <td class={`px-3 py-1.5 text-right font-mono ${
                              rate < 0.05 ? "text-emerald-400" : rate < 0.15 ? "text-amber-400" : "text-red-400"
                            }`}>{pct(rate)}</td>
                          </tr>
                        );
                      }}
                    </For>
                  </tbody>
                </table>
              </Show>
            </div>

            {/* Latest silent asks (for triage) */}
            <Show when={d().latest_silent.length > 0}>
              <div class="bg-gray-900 border border-gray-700 rounded-lg overflow-hidden">
                <div class="px-4 py-2 border-b border-gray-700 uppercase tracking-wider text-gray-300 text-xs font-semibold">
                  Latest silent asks ({d().latest_silent.length})
                </div>
                <div class="divide-y divide-gray-800">
                  <For each={d().latest_silent}>
                    {(s) => (
                      <div class="px-3 py-2 text-sm">
                        <div class="flex items-baseline gap-3">
                          <span class="text-xs font-mono text-gray-400">#{s.id}</span>
                          <span class="text-cyan-300">@{s.to}</span>
                        </div>
                        <div class="text-gray-200 mt-0.5">{s.content}</div>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
};

const Stat: Component<{ label: string; value: string; valueClass?: string; hint?: string }> = (p) => (
  <div class="bg-gray-900 border border-gray-700 rounded-lg p-3">
    <div class="text-xs uppercase tracking-wider text-gray-400">{p.label}</div>
    <div class={`text-2xl font-bold mt-1 ${p.valueClass ?? "text-gray-100"}`}>{p.value}</div>
    <Show when={p.hint}>
      <div class="text-[11px] text-gray-500 mt-0.5">{p.hint}</div>
    </Show>
  </div>
);

export default OpsSla;
