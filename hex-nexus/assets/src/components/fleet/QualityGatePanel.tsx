/**
 * QualityGatePanel.tsx — Quality gate status for active swarms.
 *
 * Shows per-tier gate results (pass/fail/running/pending) and fix history.
 * Data fetched from hex-nexus REST endpoints with 5s polling.
 */
import { Component, For, Show, createSignal, createMemo, onMount, onCleanup } from "solid-js";
import { swarms } from "../../stores/connection";
import { restClient } from "../../services/rest-client";

// ── Types ────────────────────────────────────────────────────────────────

interface QualityGate {
  id: string;
  swarm_id: string;
  tier: number;
  status: "pass" | "fail" | "running" | "pending";
  score: number | null;
  grade: string | null;
  iterations: number;
}

interface GateFix {
  id: string;
  gate_id: string;
  file: string;
  issue: string;
  model: string;
  cost_usd: number;
}

// ── Helpers ──────────────────────────────────────────────────────────────

function statusIcon(status: string): string {
  switch (status) {
    case "pass": return "\u2713";
    case "fail": return "\u2717";
    case "running": return "\u25B6";
    default: return "\u25CB";
  }
}

function statusColorClass(status: string): string {
  switch (status) {
    case "pass": return "text-green-400";
    case "fail": return "text-red-400";
    case "running": return "text-yellow-400";
    default: return "text-gray-500";
  }
}

function statusBgClass(status: string): string {
  switch (status) {
    case "pass": return "bg-green-900/30 border-green-800";
    case "fail": return "bg-red-900/30 border-red-800";
    case "running": return "bg-yellow-900/30 border-yellow-800";
    default: return "bg-gray-900/30 border-gray-800";
  }
}

// ── Component ────────────────────────────────────────────────────────────

const QualityGatePanel: Component = () => {
  const [gates, setGates] = createSignal<QualityGate[]>([]);
  const [fixes, setFixes] = createSignal<Record<string, GateFix[]>>({});
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [expandedGate, setExpandedGate] = createSignal<string | null>(null);

  // Pick the first active swarm (or use all)
  const activeSwarms = createMemo(() =>
    swarms().filter((s: any) => (s.status ?? s.state ?? "") === "active")
  );

  async function fetchGates() {
    try {
      const activeList = activeSwarms();
      if (activeList.length === 0) {
        setGates([]);
        setLoading(false);
        return;
      }

      let allGates: QualityGate[] = [];
      for (const s of activeList) {
        const sid = s.id ?? s.swarm_id ?? "";
        if (!sid) continue;
        try {
          const data = await restClient.get<QualityGate[]>(
            `/api/hexflo/quality-gate?swarm_id=${encodeURIComponent(sid)}`
          );
          if (Array.isArray(data)) {
            allGates = allGates.concat(data);
          }
        } catch {
          // Swarm may not have gates yet — skip silently
        }
      }
      setGates(allGates);
      setError(null);
    } catch (err: any) {
      setError(err.message ?? "Failed to fetch quality gates");
    } finally {
      setLoading(false);
    }
  }

  async function fetchFixes(gateId: string) {
    if (fixes()[gateId]) return; // already loaded
    try {
      const data = await restClient.get<GateFix[]>(
        `/api/hexflo/quality-gate/${encodeURIComponent(gateId)}/fixes`
      );
      setFixes((prev) => ({ ...prev, [gateId]: Array.isArray(data) ? data : [] }));
    } catch {
      setFixes((prev) => ({ ...prev, [gateId]: [] }));
    }
  }

  function toggleGate(gateId: string) {
    if (expandedGate() === gateId) {
      setExpandedGate(null);
    } else {
      setExpandedGate(gateId);
      fetchFixes(gateId);
    }
  }

  // Poll every 5 seconds
  let pollTimer: ReturnType<typeof setInterval>;
  onMount(() => {
    fetchGates();
    pollTimer = setInterval(fetchGates, 5000);
  });
  onCleanup(() => clearInterval(pollTimer));

  // Group gates by swarm
  const gatesBySwarm = createMemo(() => {
    const map = new Map<string, { swarmName: string; gates: QualityGate[] }>();
    for (const gate of gates()) {
      const sid = gate.swarm_id;
      if (!map.has(sid)) {
        const swarm = swarms().find((s: any) => (s.id ?? s.swarm_id ?? "") === sid);
        const name = swarm?.name ?? sid.slice(0, 8);
        const topology = swarm?.topology ?? swarm?.swarm_topology ?? "";
        map.set(sid, {
          swarmName: `${name}${topology ? ` (${topology})` : ""}`,
          gates: [],
        });
      }
      map.get(sid)!.gates.push(gate);
    }
    // Sort gates within each swarm by tier
    for (const entry of map.values()) {
      entry.gates.sort((a, b) => a.tier - b.tier);
    }
    return map;
  });

  return (
    <div class="rounded-lg border border-gray-800 bg-gray-900/50 p-4">
      {/* Header */}
      <div class="mb-3 flex items-center justify-between">
        <h3 class="text-xs font-semibold uppercase tracking-wider text-gray-400">
          Quality Gates
        </h3>
        <Show when={gates().length > 0}>
          <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-400">
            {gates().filter((g) => g.status === "pass").length}/{gates().length} passing
          </span>
        </Show>
      </div>

      {/* Loading state */}
      <Show when={loading()}>
        <p class="text-xs text-gray-500">Loading quality gates...</p>
      </Show>

      {/* Error state */}
      <Show when={error()}>
        <p class="text-xs text-red-400">{error()}</p>
      </Show>

      {/* Empty state */}
      <Show when={!loading() && !error() && gates().length === 0}>
        <p class="text-xs text-gray-500">
          No quality gates found. Gates are created during validation phases.
        </p>
      </Show>

      {/* Gate list grouped by swarm */}
      <Show when={gates().length > 0}>
        <div class="space-y-4">
          <For each={Array.from(gatesBySwarm().entries())}>
            {([swarmId, { swarmName, gates: swarmGates }]) => (
              <div>
                {/* Swarm header */}
                <p class="mb-1.5 text-[11px] font-medium text-gray-300">
                  {swarmName}
                </p>

                {/* Tier rows */}
                <div class="space-y-1">
                  <For each={swarmGates}>
                    {(gate) => {
                      const isExpanded = () => expandedGate() === gate.id;
                      const gateFixes = () => fixes()[gate.id] ?? [];

                      return (
                        <div>
                          <button
                            class={`flex w-full items-center gap-2 rounded-lg border px-3 py-1.5 text-left text-xs transition-colors hover:brightness-110 ${statusBgClass(gate.status)}`}
                            onClick={() => toggleGate(gate.id)}
                          >
                            <span class={`font-mono font-bold ${statusColorClass(gate.status)}`}>
                              {statusIcon(gate.status)}
                            </span>
                            <span class="text-gray-300">
                              Tier {gate.tier}:
                            </span>
                            <span class={`font-semibold uppercase ${statusColorClass(gate.status)}`}>
                              {gate.status}
                            </span>
                            <Show when={gate.score != null}>
                              <span class="text-gray-500">
                                (Score: {gate.score}, Grade {gate.grade ?? "?"})
                              </span>
                            </Show>
                            <Show when={gate.iterations > 1}>
                              <span class="ml-auto text-[10px] text-gray-500">
                                {gate.iterations} iterations
                              </span>
                            </Show>
                          </button>

                          {/* Fix history (expanded) */}
                          <Show when={isExpanded() && gateFixes().length > 0}>
                            <div class="ml-6 mt-1 space-y-0.5">
                              <For each={gateFixes()}>
                                {(fix) => (
                                  <div class="flex items-center gap-2 text-[10px]">
                                    <span class="text-yellow-500">[fix]</span>
                                    <span class="truncate font-mono text-gray-300" title={fix.file}>
                                      {fix.file}
                                    </span>
                                    <span class="text-gray-500">—</span>
                                    <span class="text-gray-400">{fix.issue}</span>
                                    <Show when={fix.model || fix.cost_usd > 0}>
                                      <span class="ml-auto shrink-0 text-gray-600">
                                        ({fix.model}{fix.cost_usd > 0 ? `, $${fix.cost_usd.toFixed(3)}` : ""})
                                      </span>
                                    </Show>
                                  </div>
                                )}
                              </For>
                            </div>
                          </Show>

                          {/* Expanded but no fixes */}
                          <Show when={isExpanded() && gateFixes().length === 0}>
                            <p class="ml-6 mt-1 text-[10px] text-gray-600">
                              No fixes for this gate
                            </p>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default QualityGatePanel;
