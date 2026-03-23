/**
 * SecretsPane.tsx — Secret management status display.
 *
 * Shows secrets health, grants table, and inference endpoints.
 */
import { Component, For, Show, createResource } from "solid-js";
import { restClient } from "../../services/rest-client";

interface SecretsHealth {
  status: string;
  provider: string;
  message?: string;
}

interface Grant {
  agent_id: string;
  secret_key: string;
  purpose: string;
  status: string;
}

interface InferenceEndpoint {
  id: string;
  url: string;
  model: string;
  status: string;
}

function statusBadgeClass(status: string): string {
  const s = status.toLowerCase();
  if (s === "active" || s === "healthy" || s === "ok") return "bg-green-900/40 text-green-400";
  if (s === "degraded" || s === "warning" || s === "pending") return "bg-yellow-900/40 text-yellow-400";
  if (s === "error" || s === "revoked" || s === "offline") return "bg-red-900/40 text-red-400";
  return "bg-gray-800 text-gray-400";
}

function healthBannerClass(status: string): string {
  const s = status.toLowerCase();
  if (s === "healthy" || s === "ok") return "border-green-800/40 bg-green-950/30 text-green-400";
  if (s === "degraded" || s === "warning") return "border-yellow-800/40 bg-yellow-950/30 text-yellow-400";
  return "border-red-800/40 bg-red-950/30 text-red-400";
}

const SecretsPane: Component = () => {
  const [health, { refetch: refetchHealth }] = createResource(async () => {
    return restClient.get<SecretsHealth>("/api/secrets/health");
  });

  const [grants, { refetch: refetchGrants }] = createResource(async () => {
    return restClient.get<Grant[]>("/secrets/grants");
  });

  const [endpoints, { refetch: refetchEndpoints }] = createResource(async () => {
    return restClient.get<InferenceEndpoint[]>("/api/inference/endpoints");
  });

  function refetchAll() {
    refetchHealth();
    refetchGrants();
    refetchEndpoints();
  }

  const isLoading = () => health.loading && grants.loading && endpoints.loading;

  return (
    <div class="flex flex-col gap-4 p-4">
      {/* Header */}
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-gray-200">Secrets</h2>
        <button
          class="rounded border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors disabled:opacity-50"
          onClick={() => refetchAll()}
          disabled={isLoading()}
        >
          <Show when={isLoading()} fallback="Refresh">
            <span class="animate-pulse">Loading...</span>
          </Show>
        </button>
      </div>

      {/* Health Banner */}
      <Show when={health()}>
        {(h) => (
          <div class={`rounded-lg border px-3 py-2 ${healthBannerClass(h().status)}`}>
            <div class="flex items-center gap-2">
              <span class="text-xs font-semibold uppercase">{h().status}</span>
              <span class="text-[10px] text-gray-500">provider: {h().provider}</span>
            </div>
            <Show when={h().message}>
              <p class="mt-1 text-xs opacity-80">{h().message}</p>
            </Show>
          </div>
        )}
      </Show>

      {/* Grants Section */}
      <div class="flex flex-col gap-2">
        <h3 class="text-xs font-medium text-gray-400 uppercase tracking-wide">Grants</h3>

        <Show when={grants.loading && !grants()}>
          <div class="flex flex-col items-center justify-center py-8 text-gray-500">
            <svg class="h-8 w-8 animate-spin text-cyan-400" viewBox="0 0 24 24" fill="none">
              <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" stroke-dasharray="31.4 31.4" stroke-linecap="round" />
            </svg>
            <span class="mt-3 text-xs">Loading grants...</span>
          </div>
        </Show>

        <Show when={!grants.loading && grants() && grants()!.length === 0}>
          <div class="flex flex-col items-center justify-center py-8 text-gray-500">
            <svg class="h-10 w-10 text-gray-700" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z" />
            </svg>
            <p class="mt-3 text-xs">No grants configured</p>
          </div>
        </Show>

        <Show when={grants() && grants()!.length > 0}>
          <div class="overflow-x-auto rounded-lg border border-gray-800">
            <table class="w-full text-xs">
              <thead>
                <tr class="border-b border-gray-800 bg-gray-950">
                  <th class="px-3 py-2 text-left font-medium text-gray-400">Agent</th>
                  <th class="px-3 py-2 text-left font-medium text-gray-400">Secret Key</th>
                  <th class="px-3 py-2 text-left font-medium text-gray-400">Purpose</th>
                  <th class="px-3 py-2 text-left font-medium text-gray-400">Status</th>
                </tr>
              </thead>
              <tbody>
                <For each={grants()}>
                  {(grant) => (
                    <tr class="border-b border-gray-800/50 hover:bg-gray-900/50">
                      <td class="px-3 py-2 font-mono text-gray-300">{grant.agent_id}</td>
                      <td class="px-3 py-2 font-mono text-gray-300">{grant.secret_key}</td>
                      <td class="px-3 py-2 text-gray-400">{grant.purpose}</td>
                      <td class="px-3 py-2">
                        <span class={`rounded px-1.5 py-0.5 text-[10px] font-medium ${statusBadgeClass(grant.status)}`}>
                          {grant.status}
                        </span>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </div>

      {/* Inference Endpoints Section */}
      <div class="flex flex-col gap-2">
        <h3 class="text-xs font-medium text-gray-400 uppercase tracking-wide">Inference Endpoints</h3>

        <Show when={endpoints.loading && !endpoints()}>
          <div class="flex flex-col items-center justify-center py-8 text-gray-500">
            <svg class="h-8 w-8 animate-spin text-cyan-400" viewBox="0 0 24 24" fill="none">
              <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-width="3" stroke-dasharray="31.4 31.4" stroke-linecap="round" />
            </svg>
            <span class="mt-3 text-xs">Loading endpoints...</span>
          </div>
        </Show>

        <Show when={!endpoints.loading && endpoints() && endpoints()!.length === 0}>
          <div class="flex flex-col items-center justify-center py-8 text-gray-500">
            <svg class="h-10 w-10 text-gray-700" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M8.288 15.038a5.25 5.25 0 017.424 0M5.106 11.856c3.807-3.808 9.98-3.808 13.788 0M1.924 8.674c5.565-5.565 14.587-5.565 20.152 0M12.53 18.22l-.53.53-.53-.53a.75.75 0 011.06 0z" />
            </svg>
            <p class="mt-3 text-xs">No inference endpoints configured</p>
          </div>
        </Show>

        <Show when={endpoints() && endpoints()!.length > 0}>
          <div class="flex flex-col gap-2">
            <For each={endpoints()}>
              {(ep) => (
                <div class="rounded-lg border border-gray-800 bg-gray-950 px-3 py-2">
                  <div class="flex items-center gap-2">
                    <span class={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${statusBadgeClass(ep.status)}`}>
                      {ep.status}
                    </span>
                    <span class="truncate text-xs font-medium text-gray-300">{ep.model}</span>
                    <span class="ml-auto shrink-0 font-mono text-[10px] text-gray-500">{ep.id}</span>
                  </div>
                  <p class="mt-1 truncate font-mono text-[10px] text-gray-500">{ep.url}</p>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default SecretsPane;
