/**
 * InferencePanel.tsx — Inference provider management.
 *
 * Provider cards with health status, model lists, RPM/TPM meters.
 * Register, test, select, and remove endpoints. Cost tracking.
 * Data from SpacetimeDB inference-gateway subscription.
 */
import { Component, For, Show, createSignal, createMemo } from "solid-js";
import { inferenceProviders, inferenceRequests, getInferenceConn, inferenceConnected } from "../../stores/connection";
import { addToast } from "../../stores/toast";

// Cost estimates per 1K tokens by provider type
const COST_PER_1K: Record<string, number> = {
  anthropic: 0.015,
  openai: 0.01,
  openai_compat: 0.005,
  ollama: 0,
  vllm: 0,
  "llama-cpp": 0,
};

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

const InferencePanel: Component = () => {
  const [registerOpen, setRegisterOpen] = createSignal(false);
  const [formData, setFormData] = createSignal({ name: "", type: "ollama", url: "", model: "" });
  const [selectedId, setSelectedId] = createSignal<string | null>(null);

  const totalRequests = createMemo(() => inferenceRequests().length);

  const tokenStats = createMemo(() => {
    let totalIn = 0, totalOut = 0;
    for (const r of inferenceRequests()) {
      totalIn += r.inputTokens ?? r.input_tokens ?? r.prompt_tokens ?? 0;
      totalOut += r.outputTokens ?? r.output_tokens ?? r.completion_tokens ?? 0;
    }
    // Estimate cost based on provider mix
    const avgCost = 0.005;
    const cost = ((totalIn + totalOut) / 1000) * avgCost;
    return { totalIn, totalOut, cost, count: inferenceRequests().length };
  });

  function registerProvider(e: Event) {
    e.preventDefault();
    const d = formData();
    if (!d.url) { addToast("error", "URL is required"); return; }

    const conn = getInferenceConn();
    if (!conn) {
      addToast("error", "Not connected to SpacetimeDB inference-gateway");
      return;
    }

    const id = d.name || `${d.type}-${Date.now()}`;
    // Map UI provider types to SpacetimeDB provider_type values
    const providerType = d.type === "openai-compatible" ? "openai_compat"
      : d.type === "llama-cpp" ? "openai_compat" : d.type;
    const modelsJson = d.model ? JSON.stringify([d.model]) : "[]";

    try {
      conn.reducers.registerProvider(id, providerType, d.url, "", modelsJson, 60, BigInt(100000));
      addToast("success", `Provider "${id}" registered via SpacetimeDB`);
      setFormData({ name: "", type: "ollama", url: "", model: "" });
      setRegisterOpen(false);
    } catch (err: any) {
      addToast("error", `Registration failed: ${err.message || "reducer error"}`);
    }
  }

  function removeProvider(id: string) {
    const conn = getInferenceConn();
    if (!conn) {
      addToast("error", "Not connected to SpacetimeDB inference-gateway");
      return;
    }

    try {
      conn.reducers.removeProvider(id);
      addToast("success", `Provider "${id}" removed via SpacetimeDB`);
      if (selectedId() === id) setSelectedId(null);
    } catch (err: any) {
      addToast("error", `Remove failed: ${err.message || "reducer error"}`);
    }
  }

  async function testProvider(id: string, _url: string) {
    addToast("info", `Testing all providers...`);
    try {
      const res = await fetch("/api/inference/health", { method: "POST" });
      if (res.ok) {
        const data = await res.json();
        const results = data.results ?? [];
        const result = results.find((r: any) => r.id === id);
        if (result) {
          if (result.status === "healthy") {
            addToast("success", `${id}: healthy (${result.latency_ms}ms)`);
          } else {
            addToast("error", `${id}: ${result.status}`);
          }
        } else {
          addToast("error", `${id}: not found in health check results`);
        }
        // Also show summary for other providers
        const healthy = results.filter((r: any) => r.status === "healthy").length;
        if (results.length > 1) {
          addToast("info", `${healthy}/${results.length} providers healthy`);
        }
      } else {
        addToast("error", `Health check failed: ${res.statusText}`);
      }
    } catch (err: any) {
      addToast("error", `Health check error: ${err.message}`);
    }
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-6">
      {/* Header */}
      <div class="mb-6 flex items-center justify-between">
        <div>
          <h2 class="text-[22px] font-bold text-gray-100">Inference</h2>
          <p class="mt-0.5 text-xs text-gray-400">
            {inferenceProviders().length} provider{inferenceProviders().length !== 1 ? 's' : ''} — {totalRequests()} requests
          </p>
        </div>
        <button
          class="rounded bg-cyan-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 transition-colors"
          onClick={() => setRegisterOpen(!registerOpen())}
        >
          {registerOpen() ? "Cancel" : "+ Add"}
        </button>
      </div>

      {/* Register form */}
      <Show when={registerOpen()}>
        <form class="mb-4 rounded-lg border border-gray-700 bg-gray-900 p-3 space-y-2" onSubmit={registerProvider}>
          <div class="grid grid-cols-2 gap-2">
            <input type="text" placeholder="Name (e.g. my-ollama)"
              value={formData().name} onInput={(e) => setFormData({ ...formData(), name: e.currentTarget.value })}
              class="rounded border border-gray-700 bg-gray-800 px-3 py-2 text-xs text-gray-200 placeholder-gray-600 focus:border-cyan-600 focus:outline-none" />
            <select value={formData().type} onChange={(e) => setFormData({ ...formData(), type: e.currentTarget.value })}
              class="rounded border border-gray-700 bg-gray-800 px-3 py-2 text-xs text-gray-200 focus:border-cyan-600 focus:outline-none">
              <option value="ollama">Ollama</option>
              <option value="vllm">vLLM</option>
              <option value="openai-compatible">OpenAI-compatible</option>
              <option value="llama-cpp">llama.cpp</option>
            </select>
          </div>
          <input type="text" placeholder="Base URL (e.g. http://localhost:11434)"
            value={formData().url} onInput={(e) => setFormData({ ...formData(), url: e.currentTarget.value })}
            class="w-full rounded border border-gray-700 bg-gray-800 px-3 py-2 text-xs text-gray-200 placeholder-gray-600 focus:border-cyan-600 focus:outline-none" />
          <div class="flex gap-2">
            <input type="text" placeholder="Model (optional, e.g. qwen3.5:27b)"
              value={formData().model} onInput={(e) => setFormData({ ...formData(), model: e.currentTarget.value })}
              class="flex-1 rounded border border-gray-700 bg-gray-800 px-3 py-2 text-xs text-gray-200 placeholder-gray-600 focus:border-cyan-600 focus:outline-none" />
            <button type="submit" class="rounded bg-cyan-600 px-4 py-2 text-xs font-medium text-white hover:bg-cyan-500">Register</button>
          </div>
        </form>
      </Show>

      {/* Provider list */}
      <Show when={inferenceProviders().length > 0} fallback={
        <div class="flex flex-1 flex-col items-center justify-center gap-2 text-gray-600">
          <svg class="h-8 w-8" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5" />
          </svg>
          <p class="text-xs">No inference providers</p>
          <p class="text-[10px]">Click "+ Add" to register one</p>
        </div>
      }>
        <div class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
          <For each={inferenceProviders()}>
            {(provider) => {
              const id = () => provider.providerId ?? provider.provider_id ?? provider.name ?? provider.provider_name ?? "unnamed";
              const pType = () => provider.providerType ?? provider.provider_type ?? provider.type ?? "";
              const url = () => provider.baseUrl ?? provider.base_url ?? "";
              const isHealthy = () => {
                if (typeof provider.healthy === "number") return provider.healthy === 1;
                const s = provider.status ?? provider.health ?? "";
                return s === "healthy" || s === "active" || s === "online";
              };
              const lastCheck = () => provider.lastHealthCheck ?? provider.last_health_check ?? "";
              const latency = () => provider.avgLatencyMs ?? provider.avg_latency_ms ?? null;
              const rpm = () => provider.currentRpm ?? provider.current_rpm ?? 0;
              const rpmLimit = () => provider.rateLimitRpm ?? provider.rate_limit_rpm ?? 0;
              const tpm = () => provider.currentTpm ?? provider.current_tpm ?? 0;
              const tpmLimit = () => provider.rateLimitTpm ?? provider.rate_limit_tpm ?? 0;
              const models = () => {
                const m = provider.modelsJson ?? provider.models_json ?? provider.models ?? provider.model;
                if (Array.isArray(m)) return m;
                if (typeof m === "string" && m) { try { return JSON.parse(m); } catch { return m ? [m] : []; } }
                return [];
              };
              const isSelected = () => selectedId() === id();

              return (
                <div
                  class="rounded-lg border p-3 transition-all cursor-pointer"
                  classList={{
                    "border-cyan-600 bg-cyan-900/10": isSelected(),
                    "border-gray-800 bg-gray-900/60 hover:border-gray-700": !isSelected(),
                  }}
                  onClick={() => setSelectedId(isSelected() ? null : id())}
                >
                  {/* Top row: status dot, name, type badge */}
                  <div class="flex items-center gap-2 mb-1">
                    <span class="relative flex h-2.5 w-2.5">
                      <span class={`absolute inline-flex h-full w-full rounded-full ${isHealthy() ? 'bg-green-400' : 'bg-gray-500'}`} classList={{ "animate-ping opacity-75": isHealthy() }} />
                      <span class={`relative inline-flex h-2.5 w-2.5 rounded-full ${isHealthy() ? 'bg-green-500' : 'bg-gray-500'}`} />
                    </span>
                    <span class="text-xs font-semibold text-gray-100 truncate">{id()}</span>
                    <span class="ml-auto rounded bg-gray-800 px-1.5 py-0.5 text-[9px] text-gray-400 uppercase">{pType()}</span>
                  </div>

                  {/* URL */}
                  <Show when={url()}>
                    <p class="mb-1.5 truncate font-mono text-[10px] text-gray-500">{url()}</p>
                  </Show>

                  {/* Models */}
                  <Show when={models().length > 0}>
                    <div class="flex flex-wrap gap-1 mb-2">
                      <For each={models().slice(0, 6)}>
                        {(model: string) => (
                          <span class="rounded bg-cyan-900/30 px-1.5 py-0.5 text-[10px] text-cyan-300">
                            {typeof model === "string" ? model : (model as any)?.id ?? "?"}
                          </span>
                        )}
                      </For>
                      <Show when={models().length > 6}>
                        <span class="text-[10px] text-gray-500">+{models().length - 6}</span>
                      </Show>
                    </div>
                  </Show>

                  {/* Stats row */}
                  <div class="flex flex-wrap gap-x-4 gap-y-0.5 text-[10px] text-gray-500">
                    <Show when={latency() != null}><span>Latency: <span class="text-gray-300">{latency()}ms</span></span></Show>
                    <Show when={rpmLimit() > 0}><span>RPM: <span class="text-gray-300">{rpm()}/{rpmLimit()}</span></span></Show>
                    <Show when={tpmLimit() > 0}><span>TPM: <span class="text-gray-300">{formatTokens(Number(tpm()))}/{formatTokens(Number(tpmLimit()))}</span></span></Show>
                    <span>{isHealthy() ? 'healthy' : 'unknown'}</span>
                    <Show when={lastCheck()}><span>checked: {new Date(lastCheck()).toLocaleTimeString()}</span></Show>
                  </div>

                  {/* Action buttons (shown when selected) */}
                  <Show when={isSelected()}>
                    <div class="mt-3 flex gap-2 border-t border-gray-800 pt-2">
                      <button
                        class="rounded border border-gray-700 px-2.5 py-1 text-[10px] text-gray-300 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
                        onClick={(e) => { e.stopPropagation(); testProvider(id(), url()); }}
                      >
                        Test Health
                      </button>
                      <button
                        class="rounded border border-gray-700 px-2.5 py-1 text-[10px] text-gray-300 hover:border-green-600 hover:text-green-300 transition-colors"
                        onClick={(e) => { e.stopPropagation(); addToast("info", `Set ${id()} as default inference provider (coming soon — currently uses first available)`); }}
                      >
                        Set Active
                      </button>
                      <button
                        class="ml-auto rounded border border-gray-700 px-2.5 py-1 text-[10px] text-gray-300 hover:border-red-600 hover:text-red-400 transition-colors"
                        onClick={(e) => { e.stopPropagation(); removeProvider(id()); }}
                      >
                        Remove
                      </button>
                    </div>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>

        {/* Token budget & cost */}
        <div class="mt-4 rounded-lg border border-gray-800 bg-gray-900/50 p-3">
          <h4 class="mb-2 text-[10px] font-semibold uppercase tracking-wider text-gray-500">Token Budget & Cost</h4>
          <div class="grid grid-cols-4 gap-2 text-center">
            <div>
              <p class="text-sm font-bold font-mono text-gray-100">{formatTokens(tokenStats().totalIn)}</p>
              <p class="text-[9px] text-gray-500 uppercase">Input</p>
            </div>
            <div>
              <p class="text-sm font-bold font-mono text-gray-100">{formatTokens(tokenStats().totalOut)}</p>
              <p class="text-[9px] text-gray-500 uppercase">Output</p>
            </div>
            <div>
              <p class="text-sm font-bold font-mono text-gray-100">${tokenStats().cost.toFixed(2)}</p>
              <p class="text-[9px] text-gray-500 uppercase">Est. Cost</p>
            </div>
            <div>
              <p class="text-sm font-bold font-mono text-gray-100">{tokenStats().count}</p>
              <p class="text-[9px] text-gray-500 uppercase">Requests</p>
            </div>
          </div>
          {/* Budget bar */}
          <Show when={tokenStats().totalIn + tokenStats().totalOut > 0}>
            <div class="mt-2">
              <div class="flex justify-between text-[9px] text-gray-500 mb-0.5">
                <span>Token usage</span>
                <span>{formatTokens(tokenStats().totalIn + tokenStats().totalOut)} total</span>
              </div>
              <div class="h-1.5 rounded bg-gray-800 overflow-hidden">
                <div class="h-full flex">
                  <div class="bg-blue-500 transition-all" style={{ width: `${tokenStats().totalIn / Math.max(tokenStats().totalIn + tokenStats().totalOut, 1) * 100}%` }} />
                  <div class="bg-green-500 transition-all" style={{ width: `${tokenStats().totalOut / Math.max(tokenStats().totalIn + tokenStats().totalOut, 1) * 100}%` }} />
                </div>
              </div>
              <div class="flex gap-3 mt-1 text-[9px]">
                <span class="flex items-center gap-1"><span class="h-1.5 w-1.5 rounded bg-blue-500" /> Input</span>
                <span class="flex items-center gap-1"><span class="h-1.5 w-1.5 rounded bg-green-500" /> Output</span>
              </div>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default InferencePanel;
