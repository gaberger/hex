/**
 * InferencePanel.tsx — Inference control pane.
 *
 * Provider cards with health status, model lists, RPM/TPM meters.
 * Register new endpoints. Cost tracking per session.
 * Data from SpacetimeDB inference-gateway subscription.
 */
import { Component, For, Show, createSignal, createMemo } from "solid-js";
import { inferenceProviders, inferenceRequests } from "../../stores/connection";

function healthColor(status: string): string {
  if (status === "healthy" || status === "active" || status === "online") return "bg-green-500";
  if (status === "degraded" || status === "stale") return "bg-yellow-500";
  return "bg-red-500";
}

// Rough cost estimates per 1K tokens
const COST_PER_1K: Record<string, number> = {
  "anthropic": 0.015,
  "openai": 0.01,
  "ollama": 0,
  "vllm": 0,
  "llama-cpp": 0,
};

const InferencePanel: Component = () => {
  const [registerOpen, setRegisterOpen] = createSignal(false);
  const [formData, setFormData] = createSignal({ name: "", type: "ollama", url: "", model: "" });

  const totalRequests = createMemo(() => inferenceRequests().length);

  const providersByType = createMemo(() => {
    const groups = new Map<string, any[]>();
    for (const p of inferenceProviders()) {
      const type = p.provider_type ?? p.type ?? "unknown";
      if (!groups.has(type)) groups.set(type, []);
      groups.get(type)!.push(p);
    }
    return groups;
  });

  async function registerProvider(e: Event) {
    e.preventDefault();
    const d = formData();
    if (!d.url) return;
    try {
      await fetch("/api/inference/register", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: d.name || d.type,
          provider_type: d.type,
          base_url: d.url,
          models: d.model ? [d.model] : [],
        }),
      });
      setFormData({ name: "", type: "ollama", url: "", model: "" });
      setRegisterOpen(false);
    } catch { /* will appear via SpacetimeDB */ }
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      {/* Header */}
      <div class="mb-4 flex items-center justify-between">
        <div>
          <h3 class="text-sm font-semibold text-gray-100">Inference</h3>
          <p class="text-xs text-gray-300">
            {inferenceProviders().length} providers — {totalRequests()} requests
          </p>
        </div>
        <button
          class="rounded bg-cyan-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 transition-colors"
          onClick={() => setRegisterOpen(!registerOpen())}
        >
          {registerOpen() ? "Cancel" : "Add Provider"}
        </button>
      </div>

      {/* Register form */}
      <Show when={registerOpen()}>
        <form class="mb-4 grid grid-cols-2 gap-2" onSubmit={registerProvider}>
          <input
            type="text"
            placeholder="Provider name"
            value={formData().name}
            onInput={(e) => setFormData({ ...formData(), name: e.currentTarget.value })}
            class="rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 focus:border-cyan-600 focus:outline-none"
          />
          <select
            value={formData().type}
            onChange={(e) => setFormData({ ...formData(), type: e.currentTarget.value })}
            class="rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 focus:border-cyan-600 focus:outline-none"
          >
            <option value="ollama">Ollama</option>
            <option value="vllm">vLLM</option>
            <option value="openai">OpenAI-compatible</option>
            <option value="anthropic">Anthropic</option>
            <option value="llama-cpp">llama.cpp</option>
          </select>
          <input
            type="text"
            placeholder="Base URL (e.g. http://localhost:11434)"
            value={formData().url}
            onInput={(e) => setFormData({ ...formData(), url: e.currentTarget.value })}
            class="col-span-2 rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 focus:border-cyan-600 focus:outline-none"
          />
          <input
            type="text"
            placeholder="Model (optional)"
            value={formData().model}
            onInput={(e) => setFormData({ ...formData(), model: e.currentTarget.value })}
            class="rounded border border-gray-700 bg-gray-800 px-3 py-2 text-sm text-gray-300 focus:border-cyan-600 focus:outline-none"
          />
          <button
            type="submit"
            class="rounded bg-cyan-600 px-3 py-2 text-sm font-medium text-white hover:bg-cyan-500"
          >
            Register
          </button>
        </form>
      </Show>

      {/* Provider cards */}
      <Show
        when={inferenceProviders().length > 0}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-300">No inference providers registered</p>
          </div>
        }
      >
        <div class="grid gap-3 grid-cols-[repeat(auto-fill,minmax(280px,1fr))]">
          <For each={inferenceProviders()}>
            {(provider) => <ProviderCard provider={provider} />}
          </For>
        </div>

        {/* Cost summary */}
        <div class="mt-4 rounded-lg border border-gray-800 bg-gray-900/50 p-4">
          <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300">
            Cost Estimate
          </h4>
          <CostTracker />
        </div>
      </Show>
    </div>
  );
};

const ProviderCard: Component<{ provider: any }> = (props) => {
  const status = () => props.provider.status ?? props.provider.health ?? "unknown";
  const models = () => {
    const m = props.provider.models ?? props.provider.model ?? props.provider.models_json;
    if (Array.isArray(m)) return m;
    if (typeof m === "string" && m) {
      try { return JSON.parse(m); } catch { return [m]; }
    }
    return [];
  };

  return (
    <div class="rounded-lg border border-gray-800 bg-gray-900/60 p-4 transition-all hover:bg-gray-900">
      <div class="flex items-center gap-2 mb-3">
        <span class={`h-2.5 w-2.5 rounded-full ${healthColor(status())}`} />
        <span class="text-sm font-semibold text-gray-100 truncate">
          {props.provider.name ?? props.provider.provider_name ?? "unnamed"}
        </span>
        <span class="ml-auto rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300 uppercase">
          {props.provider.provider_type ?? props.provider.type ?? ""}
        </span>
      </div>

      <Show when={props.provider.base_url}>
        <p class="mb-2 truncate font-mono text-[10px] text-gray-300">
          {props.provider.base_url}
        </p>
      </Show>

      <Show when={models().length > 0}>
        <div class="flex flex-wrap gap-1">
          <For each={models().slice(0, 5)}>
            {(model: string) => (
              <span class="rounded bg-cyan-900/30 px-1.5 py-0.5 text-[10px] text-cyan-300">
                {typeof model === "string" ? model : (model as any)?.id ?? "?"}
              </span>
            )}
          </For>
          <Show when={models().length > 5}>
            <span class="text-[10px] text-gray-300">+{models().length - 5} more</span>
          </Show>
        </div>
      </Show>
    </div>
  );
};

const CostTracker: Component = () => {
  const requests = () => inferenceRequests();

  const stats = createMemo(() => {
    let totalIn = 0;
    let totalOut = 0;
    for (const r of requests()) {
      totalIn += r.input_tokens ?? r.prompt_tokens ?? 0;
      totalOut += r.output_tokens ?? r.completion_tokens ?? 0;
    }
    // Rough cost: average across provider types
    const costPer1k = 0.005; // blended estimate
    const cost = ((totalIn + totalOut) / 1000) * costPer1k;
    return { totalIn, totalOut, cost };
  });

  function formatTokens(n: number): string {
    if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
    if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
    return `${n}`;
  }

  return (
    <div class="flex gap-6">
      <div>
        <p class="text-[10px] text-gray-300">Input</p>
        <p class="text-sm font-bold text-gray-100">{formatTokens(stats().totalIn)}</p>
      </div>
      <div>
        <p class="text-[10px] text-gray-300">Output</p>
        <p class="text-sm font-bold text-gray-100">{formatTokens(stats().totalOut)}</p>
      </div>
      <div>
        <p class="text-[10px] text-gray-300">Est. Cost</p>
        <p class="text-sm font-bold text-gray-100">${stats().cost.toFixed(2)}</p>
      </div>
      <div>
        <p class="text-[10px] text-gray-300">Requests</p>
        <p class="text-sm font-bold text-gray-100">{requests().length}</p>
      </div>
    </div>
  );
};

export default InferencePanel;
