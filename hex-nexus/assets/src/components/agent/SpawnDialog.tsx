/**
 * SpawnDialog.tsx — Modal dialog for GUI-driven agent spawning.
 *
 * Calls POST /api/agents/spawn with project dir, model, and agent type.
 * New agents appear in sidebar automatically via SpacetimeDB subscription.
 */
import { Component, Show, createSignal, createResource, For } from "solid-js";
import { inferenceProviders } from "../../stores/connection";

const AGENT_TYPES = [
  { value: "hex-coder", label: "Coder", desc: "Write code with TDD" },
  { value: "planner", label: "Planner", desc: "Decompose requirements" },
  { value: "tester", label: "Tester", desc: "Run tests + validation" },
  { value: "integrator", label: "Integrator", desc: "Merge + integration tests" },
  { value: "reviewer", label: "Reviewer", desc: "Code review + quality" },
] as const;

async function fetchProjects(): Promise<{ id: string; name: string; path: string }[]> {
  try {
    const res = await fetch("/api/projects");
    if (!res.ok) return [];
    const data = await res.json();
    return (data.projects ?? data ?? []).map((p: any) => ({
      id: p.id ?? p.name,
      name: p.name ?? "unnamed",
      path: p.path ?? "",
    }));
  } catch {
    return [];
  }
}

export interface SpawnDialogProps {
  open: boolean;
  onClose: () => void;
}

const SpawnDialog: Component<SpawnDialogProps> = (props) => {
  const [projects] = createResource(fetchProjects);
  const [projectDir, setProjectDir] = createSignal("");
  const [agentType, setAgentType] = createSignal("hex-coder");
  const [model, setModel] = createSignal("");
  const [spawning, setSpawning] = createSignal(false);
  const [error, setError] = createSignal("");
  const [success, setSuccess] = createSignal("");

  const models = () => {
    const providers = inferenceProviders();
    const modelSet = new Set<string>();
    for (const p of providers) {
      const m = p.model ?? p.models ?? "";
      if (typeof m === "string" && m) modelSet.add(m);
    }
    return Array.from(modelSet);
  };

  async function handleSpawn(e: Event) {
    e.preventDefault();
    const dir = projectDir().trim();
    if (!dir) {
      setError("Project directory is required");
      return;
    }

    setSpawning(true);
    setError("");
    setSuccess("");

    try {
      const res = await fetch("/api/agents/spawn", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          projectDir: dir,
          agentName: agentType(),
          model: model() || undefined,
        }),
      });

      const data = await res.json();
      if (!res.ok) {
        setError(data.error ?? `Spawn failed (${res.status})`);
        return;
      }

      setSuccess(`Agent spawned: ${data.agent?.id ?? "ok"}`);
      setTimeout(() => {
        props.onClose();
        setSuccess("");
        setProjectDir("");
      }, 1000);
    } catch (err: any) {
      setError(err.message ?? "Network error");
    } finally {
      setSpawning(false);
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
            <h2 class="text-sm font-semibold text-gray-100">Spawn Agent</h2>
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
          <form class="space-y-4 px-5 py-4" onSubmit={handleSpawn}>
            {/* Project directory */}
            <div>
              <label class="mb-1.5 block text-[11px] font-medium uppercase tracking-wider text-gray-300">
                Project Directory
              </label>
              <Show
                when={(projects()?.length ?? 0) > 0}
                fallback={
                  <input
                    type="text"
                    placeholder="e.g. /Users/gary/projects/my-app"
                    value={projectDir()}
                    onInput={(e) => setProjectDir(e.currentTarget.value)}
                    class="w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-300 focus:border-cyan-600 focus:outline-none"
                    autofocus
                  />
                }
              >
                <select
                  value={projectDir()}
                  onChange={(e) => setProjectDir(e.currentTarget.value)}
                  class="w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 focus:border-cyan-600 focus:outline-none"
                >
                  <option value="">Select project...</option>
                  <For each={projects()}>
                    {(p) => {
                      const val = p.path || p.id || p.name;
                      return <option value={val}>{p.name}{p.path ? ` — ${p.path}` : ''}</option>;
                    }}
                  </For>
                </select>
              </Show>
              <input
                type="text"
                placeholder="Or type a path manually..."
                value={projectDir()}
                onInput={(e) => setProjectDir(e.currentTarget.value)}
                class="mt-2 w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-100 placeholder-gray-300 focus:border-cyan-600 focus:outline-none"
              />
            </div>

            {/* Agent type */}
            <div>
              <label class="mb-1.5 block text-[11px] font-medium uppercase tracking-wider text-gray-300">
                Agent Type
              </label>
              <div class="grid grid-cols-2 gap-2">
                <For each={AGENT_TYPES}>
                  {(t) => (
                    <button
                      type="button"
                      class="rounded border px-3 py-2 text-left text-xs transition-colors"
                      classList={{
                        "border-cyan-500 bg-cyan-900/30 text-cyan-200": agentType() === t.value,
                        "border-gray-700 bg-gray-800 text-gray-300 hover:border-gray-600": agentType() !== t.value,
                      }}
                      onClick={() => setAgentType(t.value)}
                    >
                      <div class="font-medium">{t.label}</div>
                      <div class="mt-0.5 text-[10px] text-gray-300">{t.desc}</div>
                    </button>
                  )}
                </For>
              </div>
            </div>

            {/* Model override */}
            <div>
              <label class="mb-1.5 block text-[11px] font-medium uppercase tracking-wider text-gray-300">
                Model <span class="font-normal text-gray-300">(optional)</span>
              </label>
              <Show
                when={models().length > 0}
                fallback={
                  <input
                    type="text"
                    placeholder="auto (use default)"
                    value={model()}
                    onInput={(e) => setModel(e.currentTarget.value)}
                    class="w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 placeholder-gray-600 focus:border-cyan-600 focus:outline-none"
                  />
                }
              >
                <select
                  value={model()}
                  onChange={(e) => setModel(e.currentTarget.value)}
                  class="w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 focus:border-cyan-600 focus:outline-none"
                >
                  <option value="">Auto (default)</option>
                  <For each={models()}>
                    {(m) => <option value={m}>{m}</option>}
                  </For>
                </select>
              </Show>
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
              disabled={spawning()}
              class="w-full rounded bg-cyan-600 py-2.5 text-sm font-medium text-white transition-colors hover:bg-cyan-500 disabled:opacity-50"
            >
              {spawning() ? "Spawning..." : "Spawn Agent"}
            </button>
          </form>
        </div>
      </div>
    </Show>
  );
};

export default SpawnDialog;
