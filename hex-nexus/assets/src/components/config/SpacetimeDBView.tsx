import { Component, For } from 'solid-js';
import { addToast } from '../../stores/toast';
import { hexfloConnected, agentRegistryConnected, inferenceConnected, fleetConnected } from '../../stores/connection';

interface ModuleDef {
  name: string;
  connected: () => boolean;
  tables: string[];
}

const MODULES: ModuleDef[] = [
  {
    name: "hexflo-coordination",
    connected: hexfloConnected,
    tables: ["swarm", "swarm_task", "swarm_agent", "hexflo_memory", "project"],
  },
  {
    name: "agent-registry",
    connected: agentRegistryConnected,
    tables: ["agent", "agent_heartbeat"],
  },
  {
    name: "inference-gateway",
    connected: inferenceConnected,
    tables: ["inference_provider", "inference_request", "inference_response"],
  },
  {
    name: "fleet-state",
    connected: fleetConnected,
    tables: ["compute_node"],
  },
];

const SpacetimeDBView: Component = () => {
  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">SpacetimeDB Modules</h2>
          <p class="mt-1 text-sm text-gray-400">
            Module connections and table subscriptions.
          </p>
        </div>
        <div class="flex items-center gap-3">
          <button class="rounded-lg bg-cyan-900/40 px-4 py-2 text-sm font-medium text-cyan-300 hover:bg-cyan-900/60 transition-colors border border-cyan-700/40"
            onClick={() => addToast("info", "Run: spacetime publish <module-name> from the module directory")}>
            Publish Module
          </button>
          <button class="rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-gray-100 transition-colors border border-gray-700"
            onClick={() => addToast("info", "Run: spacetime generate --lang typescript --out-dir src/spacetimedb/<module> --module-path ../../spacetime-modules/<module>")}>
            Generate Bindings
          </button>
        </div>
      </div>

      {/* URI info */}
      <div class="mb-6 rounded-lg bg-gray-800/50 border border-gray-700 px-4 py-3 flex items-center gap-3">
        <span class="text-xs font-medium uppercase tracking-wider text-gray-500">URI</span>
        <code class="font-mono text-sm text-gray-300">ws://localhost:3000</code>
      </div>

      {/* Module cards */}
      <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
        <For each={MODULES}>
          {(mod) => {
            const isConnected = () => mod.connected();
            return (
              <div
                class="rounded-xl bg-[var(--bg-surface)] p-4 border"
                classList={{
                  "border-[rgba(34,211,238,0.25)]": isConnected(),
                  "border-[rgba(107,114,128,0.3)]": !isConnected(),
                }}
              >
                {/* Status dot + module name */}
                <div class="flex items-center gap-2 mb-3">
                  <span
                    class="h-2.5 w-2.5 rounded-full shrink-0"
                    classList={{
                      "bg-emerald-400": isConnected(),
                      "bg-gray-600": !isConnected(),
                    }}
                  />
                  <span class="font-bold font-mono text-sm text-gray-100">{mod.name}</span>
                </div>

                {/* Connection status */}
                <div class="mb-3">
                  <span
                    class="inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium"
                    classList={{
                      "bg-emerald-900/40 text-emerald-300 border-emerald-700/40": isConnected(),
                      "bg-gray-800 text-gray-500 border-gray-700": !isConnected(),
                    }}
                  >
                    {isConnected() ? "Connected" : "Disconnected"}
                  </span>
                </div>

                {/* Table chips */}
                <div class="flex flex-wrap gap-1.5 mb-4">
                  <For each={mod.tables}>
                    {(table) => (
                      <span class="rounded-full bg-gray-800 border border-gray-700 px-2.5 py-0.5 text-xs font-mono text-gray-400">
                        {table}
                      </span>
                    )}
                  </For>
                </div>

                {/* Reconnect button (always shown, but styled differently) */}
                {!isConnected() && (
                  <button class="rounded-lg bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors border border-gray-700"
                    onClick={() => { Object.keys(localStorage).filter(k => k.startsWith('stdb_token_')).forEach(k => localStorage.removeItem(k)); location.reload(); }}>
                    Reconnect
                  </button>
                )}
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
};

export default SpacetimeDBView;
