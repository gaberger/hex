/**
 * SwapsView — substrate swap-ticket dashboard
 * (ADR-2604261500 P5, wp-substrate-shadow-promotion P5.1).
 *
 * Lists in-flight swap_ticket rows with shadow-sample summaries. Polls
 * /api/swaps every 5s. The proper STDB-reactive subscription path
 * requires republishing the hexflo-coordination WASM module + regenerating
 * SDK bindings; that deploy step is queued as a follow-up.
 *
 * Click a ticket to load its samples on demand.
 */
import { createSignal, createEffect, For, Show, onCleanup } from "solid-js";

interface SwapTicket {
  id: string;
  project_id: string;
  port_id: string;
  incumbent_adapter_id: string;
  candidate_adapter_id: string;
  state: string;
  shadow_traffic_fraction: number;
  shadow_window_seconds: number;
  shadow_started_at: string;
  success_criteria_json: string;
  created_at: string;
  updated_at: string;
}

interface ShadowSample {
  id: number;
  ticket_id: string;
  call_seq: number;
  incumbent_adapter_id: string;
  candidate_adapter_id: string;
  incumbent_metrics_json: string;
  candidate_metrics_json: string;
  agreed: boolean;
  reason: string;
  recorded_at: string;
}

function timeInShadow(started: string): string {
  if (!started) return "—";
  const start = new Date(started).getTime();
  if (Number.isNaN(start)) return "—";
  const elapsed = Math.max(0, Math.floor((Date.now() - start) / 1000));
  if (elapsed < 60) return `${elapsed}s`;
  if (elapsed < 3600) return `${Math.floor(elapsed / 60)}m`;
  return `${Math.floor(elapsed / 3600)}h${Math.floor((elapsed % 3600) / 60)}m`;
}

function p99Latency(samples: ShadowSample[]): string {
  if (samples.length === 0) return "—";
  const latencies: number[] = [];
  for (const s of samples) {
    try {
      const m = JSON.parse(s.candidate_metrics_json);
      if (typeof m.latency_ms === "number") latencies.push(m.latency_ms);
    } catch {
      /* ignore parse errors */
    }
  }
  if (latencies.length === 0) return "—";
  latencies.sort((a, b) => a - b);
  const idx = Math.min(latencies.length - 1, Math.ceil(latencies.length * 0.99) - 1);
  return `${latencies[Math.max(0, idx)]}ms`;
}

export default function SwapsView() {
  const [tickets, setTickets] = createSignal<SwapTicket[]>([]);
  const [samplesByTicket, setSamplesByTicket] = createSignal<Record<string, ShadowSample[]>>({});
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());
  const [warning, setWarning] = createSignal<string>("");
  const [error, setError] = createSignal<string>("");

  async function fetchTickets() {
    try {
      const resp = await fetch("/api/swaps");
      const body = await resp.json();
      setTickets(body.tickets ?? []);
      setWarning(body.warning ?? "");
      setError(body.error ?? "");
      // Refresh samples for any expanded ticket.
      for (const id of expanded()) {
        await fetchSamples(id);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function fetchSamples(ticketId: string) {
    try {
      const resp = await fetch(`/api/swaps/${encodeURIComponent(ticketId)}/samples`);
      const body = await resp.json();
      setSamplesByTicket((prev) => ({ ...prev, [ticketId]: body.samples ?? [] }));
    } catch {
      /* ignore — leave stale */
    }
  }

  function toggle(ticketId: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(ticketId)) {
        next.delete(ticketId);
      } else {
        next.add(ticketId);
        fetchSamples(ticketId);
      }
      return next;
    });
  }

  createEffect(() => {
    fetchTickets();
    const interval = setInterval(fetchTickets, 5000);
    onCleanup(() => clearInterval(interval));
  });

  return (
    <div class="flex flex-1 flex-col overflow-y-auto p-6">
      <div class="mb-4">
        <h1 class="text-xl font-semibold text-gray-100">Substrate swaps</h1>
        <p class="mt-1 text-sm text-gray-400">
          In-flight swap tickets per ADR-2604261500. Polls every 5s.
        </p>
      </div>

      <Show when={warning()}>
        <div class="mb-3 rounded-md border border-amber-700/50 bg-amber-950/30 px-3 py-2 text-sm text-amber-200">
          {warning()}
        </div>
      </Show>
      <Show when={error()}>
        <div class="mb-3 rounded-md border border-red-700/50 bg-red-950/30 px-3 py-2 text-sm text-red-200">
          {error()}
        </div>
      </Show>

      <Show
        when={tickets().length > 0}
        fallback={
          <div class="rounded-md border border-gray-800 bg-gray-900/40 px-4 py-8 text-center text-sm text-gray-500">
            No active swap tickets.
          </div>
        }
      >
        <div class="overflow-x-auto rounded-md border border-gray-800">
          <table class="w-full text-left text-[13px]">
            <thead class="bg-gray-900/60 text-xs uppercase tracking-wide text-gray-400">
              <tr>
                <th class="px-3 py-2 font-medium">Port</th>
                <th class="px-3 py-2 font-medium">Incumbent → Candidate</th>
                <th class="px-3 py-2 font-medium">State</th>
                <th class="px-3 py-2 font-medium">Fraction</th>
                <th class="px-3 py-2 font-medium">In shadow</th>
                <th class="px-3 py-2 font-medium">Samples</th>
                <th class="px-3 py-2 font-medium">Cand. p99</th>
              </tr>
            </thead>
            <tbody>
              <For each={tickets()}>
                {(t) => {
                  const samples = () => samplesByTicket()[t.id] ?? [];
                  const agreed = () => samples().filter((s) => s.agreed).length;
                  const total = () => samples().length;
                  return (
                    <>
                      <tr
                        class="border-t border-gray-800 hover:bg-gray-900/40 cursor-pointer"
                        onClick={() => toggle(t.id)}
                      >
                        <td class="px-3 py-2 font-mono text-gray-100">{t.port_id}</td>
                        <td class="px-3 py-2 text-gray-300">
                          <span class="font-mono">{t.incumbent_adapter_id || "(none)"}</span>
                          <span class="px-2 text-gray-600">→</span>
                          <span class="font-mono text-cyan-300">{t.candidate_adapter_id}</span>
                        </td>
                        <td class="px-3 py-2">
                          <span
                            classList={{
                              "rounded px-1.5 py-0.5 text-xs font-medium": true,
                              "bg-blue-950/40 text-blue-300": t.state === "shadow",
                              "bg-emerald-950/40 text-emerald-300": t.state === "shadow_green",
                              "bg-red-950/40 text-red-300": t.state === "shadow_red",
                              "bg-amber-950/40 text-amber-300": t.state === "candidate",
                              "bg-gray-800 text-gray-300": ["promoted", "rolled_back"].includes(t.state),
                            }}
                          >
                            {t.state}
                          </span>
                        </td>
                        <td class="px-3 py-2 text-gray-400">{(t.shadow_traffic_fraction * 100).toFixed(0)}%</td>
                        <td class="px-3 py-2 text-gray-400">{timeInShadow(t.shadow_started_at)}</td>
                        <td class="px-3 py-2 text-gray-300">
                          <Show when={total() > 0} fallback={<span class="text-gray-600">click to load</span>}>
                            {agreed()} / {total()} agreed
                          </Show>
                        </td>
                        <td class="px-3 py-2 text-gray-300">{p99Latency(samples())}</td>
                      </tr>
                      <Show when={expanded().has(t.id)}>
                        <tr class="border-t border-gray-800 bg-gray-950/40">
                          <td colspan={7} class="px-6 py-3">
                            <div class="text-xs text-gray-500 mb-2">
                              Recent samples ({samples().length}):
                            </div>
                            <div class="grid grid-cols-1 gap-1 text-xs font-mono">
                              <For each={samples().slice(-10).reverse()}>
                                {(s) => (
                                  <div class="flex gap-3">
                                    <span class="text-gray-600">#{s.call_seq}</span>
                                    <span classList={{ "text-emerald-400": s.agreed, "text-red-400": !s.agreed }}>
                                      {s.agreed ? "✓ agreed" : "✗ disagreed"}
                                    </span>
                                    <Show when={s.reason}>
                                      <span class="text-gray-400">— {s.reason}</span>
                                    </Show>
                                  </div>
                                )}
                              </For>
                            </div>
                          </td>
                        </tr>
                      </Show>
                    </>
                  );
                }}
              </For>
            </tbody>
          </table>
        </div>
      </Show>
    </div>
  );
}
