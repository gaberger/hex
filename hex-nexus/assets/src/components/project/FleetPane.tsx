/**
 * FleetPane.tsx — Compute fleet node management.
 *
 * Displays registered compute nodes with status, capacity,
 * and per-node health check actions.
 */
import { Component, For, Show, createSignal, createResource } from "solid-js";
import { restClient } from "../../services/rest-client";

interface FleetNode {
  id: string;
  hostname: string;
  status: string;
  slots_used: number;
  slots_total: number;
  models: string[];
}

function statusBadgeClass(status: string): string {
  const s = status.toLowerCase();
  if (s === "online" || s === "healthy" || s === "active") return "bg-green-900/40 text-green-400";
  if (s === "degraded" || s === "draining" || s === "warning") return "bg-yellow-900/40 text-yellow-400";
  if (s === "offline" || s === "error" || s === "dead") return "bg-red-900/40 text-red-400";
  return "bg-gray-800 text-gray-400";
}

function slotBarClass(used: number, total: number): string {
  if (total === 0) return "bg-gray-700";
  const ratio = used / total;
  if (ratio >= 0.9) return "bg-red-500";
  if (ratio >= 0.7) return "bg-yellow-500";
  return "bg-green-500";
}

const FleetPane: Component = () => {
  const [checkingId, setCheckingId] = createSignal<string | null>(null);

  const [nodes, { refetch }] = createResource(async () => {
    return restClient.get<FleetNode[]>("/api/fleet");
  });

  async function handleHealthCheck(nodeId: string) {
    setCheckingId(nodeId);
    try {
      await restClient.post("/api/fleet/health", { node_id: nodeId });
      refetch();
    } catch (err) {
      console.error("Health check failed for node:", nodeId, err);
    } finally {
      setCheckingId(null);
    }
  }

  return (
    <div class="flex flex-col gap-4 p-4">
      {/* Header */}
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-gray-200">Fleet</h2>
        <button
          class="rounded border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors disabled:opacity-50"
          onClick={() => refetch()}
          disabled={nodes.loading}
        >
          <Show when={nodes.loading} fallback="Refresh">
            <span class="animate-pulse">Loading...</span>
          </Show>
        </button>
      </div>

      {/* Loading state */}
      <Show when={nodes.loading && !nodes()}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg class="h-8 w-8 animate-spin text-cyan-400" viewBox="0 0 24 24" fill="none">
            <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" stroke-dasharray="31.4 31.4" stroke-linecap="round" />
          </svg>
          <span class="mt-3 text-xs">Loading fleet nodes...</span>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!nodes.loading && nodes() && nodes()!.length === 0}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg class="h-10 w-10 text-gray-700" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z" />
          </svg>
          <p class="mt-3 text-xs">No compute nodes registered</p>
        </div>
      </Show>

      {/* Node table */}
      <Show when={nodes() && nodes()!.length > 0}>
        <div class="overflow-x-auto rounded-lg border border-gray-800">
          <table class="w-full text-xs">
            <thead>
              <tr class="border-b border-gray-800 bg-gray-950">
                <th class="px-3 py-2 text-left font-medium text-gray-400">ID</th>
                <th class="px-3 py-2 text-left font-medium text-gray-400">Hostname</th>
                <th class="px-3 py-2 text-left font-medium text-gray-400">Status</th>
                <th class="px-3 py-2 text-left font-medium text-gray-400">Slots</th>
                <th class="px-3 py-2 text-left font-medium text-gray-400">Models</th>
                <th class="px-3 py-2 text-right font-medium text-gray-400">Health</th>
              </tr>
            </thead>
            <tbody>
              <For each={nodes()}>
                {(node) => (
                  <tr class="border-b border-gray-800/50 hover:bg-gray-900/50">
                    <td class="px-3 py-2 font-mono text-gray-300">{node.id}</td>
                    <td class="px-3 py-2 text-gray-300">{node.hostname}</td>
                    <td class="px-3 py-2">
                      <span class={`rounded px-1.5 py-0.5 text-[10px] font-medium ${statusBadgeClass(node.status)}`}>
                        {node.status}
                      </span>
                    </td>
                    <td class="px-3 py-2">
                      <div class="flex items-center gap-2">
                        <div class="h-1.5 w-16 overflow-hidden rounded-full bg-gray-800">
                          <div
                            class={`h-full rounded-full ${slotBarClass(node.slots_used, node.slots_total)}`}
                            style={{ width: node.slots_total > 0 ? `${(node.slots_used / node.slots_total) * 100}%` : "0%" }}
                          />
                        </div>
                        <span class="text-[10px] text-gray-400">
                          {node.slots_used}/{node.slots_total}
                        </span>
                      </div>
                    </td>
                    <td class="px-3 py-2">
                      <div class="flex flex-wrap gap-1">
                        <For each={node.models}>
                          {(model) => (
                            <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-400">
                              {model}
                            </span>
                          )}
                        </For>
                        <Show when={node.models.length === 0}>
                          <span class="text-[10px] text-gray-600">none</span>
                        </Show>
                      </div>
                    </td>
                    <td class="px-3 py-2 text-right">
                      <button
                        class="rounded bg-gray-800 px-2.5 py-1 text-[10px] font-medium text-gray-300 hover:bg-gray-700 hover:text-white transition-colors disabled:opacity-50"
                        onClick={() => handleHealthCheck(node.id)}
                        disabled={checkingId() === node.id}
                      >
                        {checkingId() === node.id ? "Checking..." : "Check"}
                      </button>
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

export default FleetPane;
