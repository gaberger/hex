/**
 * SwarmInitDialog.tsx — Modal dialog for initializing a new swarm.
 *
 * Calls POST /api/swarms with name and topology.
 * Swarms appear in the sidebar via polling/subscription.
 */
import { Component, Show, For, createSignal } from "solid-js";
import { addToast } from "../../stores/toast";
import { getHexfloConn, hexfloConnected } from "../../stores/connection";

const TOPOLOGIES = [
  { value: "hierarchical", label: "Hierarchical", desc: "Leader delegates to workers" },
  { value: "mesh", label: "Mesh", desc: "Peer-to-peer coordination" },
  { value: "adaptive", label: "Adaptive", desc: "Self-organizing topology" },
] as const;

export interface SwarmInitDialogProps {
  open: boolean;
  onClose: () => void;
}

const SwarmInitDialog: Component<SwarmInitDialogProps> = (props) => {
  const [name, setName] = createSignal("");
  const [topology, setTopology] = createSignal("hierarchical");
  const [submitting, setSubmitting] = createSignal(false);
  const [error, setError] = createSignal("");
  const [success, setSuccess] = createSignal("");

  async function handleSubmit(e: Event) {
    e.preventDefault();
    const trimmed = name().trim();
    if (!trimmed) {
      setError("Swarm name is required");
      return;
    }

    setSubmitting(true);
    setError("");
    setSuccess("");

    try {
      const conn = getHexfloConn();
      if (!conn) {
        // Fallback to REST if SpacetimeDB not connected
        const res = await fetch("/api/swarms", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name: trimmed, topology: topology() }),
        });
        const data = await res.json();
        if (!res.ok) {
          const msg = data.error ?? `Failed (${res.status})`;
          setError(msg);
          addToast("error", `Failed to create swarm: ${msg}`);
          return;
        }
      } else {
        // Use SpacetimeDB reducer directly (WebSocket)
        const id = crypto.randomUUID();
        const timestamp = new Date().toISOString();
        conn.reducers.swarmInit(id, trimmed, topology(), "", timestamp);
      }

      setSuccess(`Swarm "${trimmed}" initialized`);
      addToast("success", `Swarm "${trimmed}" initialized via ${conn ? 'SpacetimeDB' : 'REST'}`);
      setTimeout(() => {
        props.onClose();
        setSuccess("");
        setName("");
        setTopology("hierarchical");
      }, 1000);
    } catch (err: any) {
      const msg = err.message ?? "Network error";
      setError(msg);
      addToast("error", `Failed to create swarm: ${msg}`);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Show when={props.open}>
      {/* Backdrop */}
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
        onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}
      >
        {/* Dialog */}
        <div class="w-full max-w-md rounded-xl border border-gray-700 bg-gray-900 shadow-2xl">
          {/* Header */}
          <div class="flex items-center justify-between border-b border-gray-800 px-5 py-4">
            <h2 class="text-sm font-semibold text-gray-100">Initialize Swarm</h2>
            <button
              class="rounded p-1 text-gray-300 hover:bg-gray-800 hover:text-gray-300 transition-colors"
              onClick={props.onClose}
            >
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>

          {/* Form */}
          <form class="space-y-4 px-5 py-4" onSubmit={handleSubmit}>
            {/* Swarm name */}
            <div>
              <label class="mb-1.5 block text-[11px] font-medium uppercase tracking-wider text-gray-300">
                Swarm Name
              </label>
              <input
                type="text"
                placeholder="e.g. feature-auth"
                value={name()}
                onInput={(e) => setName(e.currentTarget.value)}
                class="w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-300 focus:border-cyan-600 focus:outline-none"
                autofocus
              />
            </div>

            {/* Topology */}
            <div>
              <label class="mb-1.5 block text-[11px] font-medium uppercase tracking-wider text-gray-300">
                Topology
              </label>
              <div class="grid grid-cols-3 gap-2">
                <For each={TOPOLOGIES}>
                  {(t) => (
                    <button
                      type="button"
                      class="rounded border px-3 py-2 text-left text-xs transition-colors"
                      classList={{
                        "border-cyan-500 bg-cyan-900/30 text-cyan-200": topology() === t.value,
                        "border-gray-700 bg-gray-800 text-gray-300 hover:border-gray-600": topology() !== t.value,
                      }}
                      onClick={() => setTopology(t.value)}
                    >
                      <div class="font-medium">{t.label}</div>
                      <div class="mt-0.5 text-[10px] text-gray-300">{t.desc}</div>
                    </button>
                  )}
                </For>
              </div>
            </div>

            {/* Error / Success */}
            <Show when={error()}>
              <p class="rounded bg-red-900/30 px-3 py-2 text-xs text-red-300">{error()}</p>
            </Show>
            <Show when={success()}>
              <p class="rounded bg-green-900/30 px-3 py-2 text-xs text-green-300">{success()}</p>
            </Show>

            {/* Submit */}
            <button
              type="submit"
              disabled={submitting()}
              class="w-full rounded bg-cyan-600 py-2.5 text-sm font-medium text-white transition-colors hover:bg-cyan-500 disabled:opacity-50"
            >
              {submitting() ? "Initializing..." : "Initialize Swarm"}
            </button>
          </form>
        </div>
      </div>
    </Show>
  );
};

export default SwarmInitDialog;
