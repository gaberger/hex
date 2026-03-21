/**
 * FleetView.tsx — Remote fleet management pane.
 *
 * Shows compute nodes with hostname, agent count, health status.
 * Data from SpacetimeDB fleet-state subscription + REST API.
 */
import { Component, For, Show, createSignal, createMemo } from "solid-js";
import { fleetNodes, fleetConnected, getFleetConn } from "../../stores/connection";
import { addToast } from "../../stores/toast";

function healthColor(status: string): string {
  if (status === "healthy" || status === "active" || status === "online") return "bg-green-500";
  if (status === "degraded" || status === "stale") return "bg-yellow-500";
  if (status === "offline" || status === "dead" || status === "error") return "bg-red-500";
  return "bg-gray-300";
}

function healthBorder(status: string): string {
  if (status === "healthy" || status === "active" || status === "online") return "border-green-800/50";
  if (status === "degraded" || status === "stale") return "border-yellow-800/50";
  if (status === "offline" || status === "dead") return "border-red-800/50";
  return "border-gray-800";
}

const FleetView: Component = () => {
  const [registerOpen, setRegisterOpen] = createSignal(false);
  const [newHost, setNewHost] = createSignal("");
  const [registering, setRegistering] = createSignal(false);

  const totalAgents = createMemo(() =>
    fleetNodes().reduce((sum: number, n: any) => sum + (n.agent_count ?? n.agents ?? 0), 0)
  );

  const onlineCount = createMemo(() =>
    fleetNodes().filter((n: any) => {
      const s = n.status ?? n.state ?? "";
      return s === "healthy" || s === "active" || s === "online";
    }).length
  );

  async function registerNode(e: Event) {
    e.preventDefault();
    const host = newHost().trim();
    if (!host) return;
    setRegistering(true);
    try {
      const conn = getFleetConn();
      if (fleetConnected() && conn?.reducers?.registerNode) {
        // SpacetimeDB reducer: registerNode(id, host, port, username, maxAgents)
        const nodeId = crypto.randomUUID();
        conn.reducers.registerNode(nodeId, host, 22, "", 4);
        addToast("success", `Node registered via SpacetimeDB: ${host}`);
      } else {
        // REST fallback
        const res = await fetch("/api/fleet/register", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ hostname: host }),
        });
        if (res.ok) {
          addToast("success", `Node registered: ${host}`);
        } else {
          const err = await res.json().catch(() => ({}));
          addToast("error", `Register failed: ${(err as any).error || res.statusText}`);
        }
      }
      setNewHost("");
      setRegisterOpen(false);
    } catch (err: any) {
      addToast("error", `Register error: ${err.message}`);
    } finally {
      setRegistering(false);
    }
  }

  async function removeNode(id: string) {
    try {
      const conn = getFleetConn();
      if (fleetConnected() && conn?.reducers?.removeNode) {
        conn.reducers.removeNode(id);
        addToast("info", "Node removed via SpacetimeDB");
      } else {
        const res = await fetch(`/api/fleet/${encodeURIComponent(id)}`, { method: "DELETE" });
        if (res.ok) {
          addToast("info", "Node removed");
        } else {
          addToast("error", `Remove failed: ${res.statusText}`);
        }
      }
    } catch (err: any) {
      addToast("error", `Remove error: ${err.message}`);
    }
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      {/* Header */}
      <div class="mb-4 flex items-center justify-between">
        <div>
          <h3 class="text-sm font-semibold text-gray-100">Fleet</h3>
          <p class="text-xs text-gray-300">
            {onlineCount()}/{fleetNodes().length} nodes online — {totalAgents()} agents
          </p>
        </div>
        <button
          class="rounded bg-cyan-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 transition-colors"
          onClick={() => setRegisterOpen(!registerOpen())}
        >
          {registerOpen() ? "Cancel" : "Add Node"}
        </button>
      </div>

      {/* Register form */}
      <Show when={registerOpen()}>
        <form class="mb-4 flex gap-2" onSubmit={registerNode}>
          <input
            type="text"
            placeholder="hostname or IP"
            value={newHost()}
            onInput={(e) => setNewHost(e.currentTarget.value)}
            class="flex-1 rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 placeholder-gray-300 focus:border-cyan-600 focus:outline-none"
            autofocus
          />
          <button
            type="submit"
            disabled={registering()}
            class="rounded bg-cyan-600 px-4 py-2 text-sm text-white hover:bg-cyan-500 disabled:opacity-50"
          >
            Register
          </button>
        </form>
      </Show>

      {/* Node grid */}
      <Show
        when={fleetNodes().length > 0}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-300">No fleet nodes registered</p>
          </div>
        }
      >
        <div class="grid gap-3 grid-cols-[repeat(auto-fill,minmax(240px,1fr))]">
          <For each={fleetNodes()}>
            {(node) => {
              const status = () => node.status ?? node.state ?? "unknown";
              return (
                <div class={`group rounded-lg border ${healthBorder(status())} bg-gray-900/60 p-4 transition-all hover:bg-gray-900`}>
                  <div class="flex items-center justify-between mb-3">
                    <div class="flex items-center gap-2">
                      <span class={`h-2.5 w-2.5 rounded-full ${healthColor(status())}`} />
                      <span class="text-sm font-semibold text-gray-100 truncate">
                        {node.hostname ?? node.name ?? "unknown"}
                      </span>
                    </div>
                    <button
                      class="rounded p-1 text-gray-300 opacity-0 group-hover:opacity-100 hover:bg-red-900/30 hover:text-red-300 transition-all"
                      onClick={() => removeNode(node.id ?? node.node_id ?? "")}
                      title="Remove node"
                    >
                      <svg class="h-3 w-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                        <line x1="18" y1="6" x2="6" y2="18" />
                        <line x1="6" y1="6" x2="18" y2="18" />
                      </svg>
                    </button>
                  </div>

                  <div class="grid grid-cols-2 gap-2 text-xs">
                    <div>
                      <p class="text-gray-300">Agents</p>
                      <p class="text-lg font-bold text-gray-100">{node.agent_count ?? node.agents ?? 0}</p>
                    </div>
                    <div>
                      <p class="text-gray-300">Status</p>
                      <p class="font-medium text-gray-100 capitalize">{status()}</p>
                    </div>
                  </div>

                  <Show when={node.gpu_info ?? node.gpu}>
                    <div class="mt-2 rounded bg-gray-800 px-2 py-1 text-[10px] text-gray-300">
                      GPU: {node.gpu_info ?? node.gpu}
                    </div>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default FleetView;
