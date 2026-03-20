import { Component, For, Show } from 'solid-js';
import { inferenceProviders, fleetNodes } from '../../stores/connection';
import { openPane } from '../../stores/panes';

function healthColor(provider: any): string {
  const status = provider?.status ?? provider?.health ?? '';
  if (status === 'healthy' || status === 'active' || status === 'online') return 'bg-green-500';
  if (status === 'degraded' || status === 'stale') return 'bg-yellow-500';
  if (status === 'error' || status === 'dead' || status === 'offline') return 'bg-red-500';
  return 'bg-gray-500';
}

function nodeStatusColor(node: any): string {
  const status = node?.status ?? node?.state ?? '';
  if (status === 'active' || status === 'online' || status === 'healthy') return 'bg-green-500';
  if (status === 'stale' || status === 'degraded') return 'bg-yellow-500';
  if (status === 'dead' || status === 'offline') return 'bg-red-500';
  return 'bg-gray-500';
}

const RightPanel: Component = () => {
  return (
    <aside class="flex h-full w-70 flex-col border-l border-gray-800 bg-gray-900 overflow-y-auto">
      {/* INFERENCE */}
      <div class="border-b border-gray-800 px-3 py-3">
        <h3
          class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300 cursor-pointer hover:text-cyan-300 transition-colors"
          onClick={() => openPane('inference', 'Inference')}
        >Inference</h3>
        <Show when={inferenceProviders().length === 0}>
          <p class="text-xs text-gray-300">No providers connected</p>
        </Show>
        <div class="space-y-1">
          <For each={inferenceProviders()}>
            {(provider) => (
              <div class="flex items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-gray-800 transition-colors">
                <span class={`h-2 w-2 shrink-0 rounded-full ${healthColor(provider)}`} />
                <div class="flex flex-col min-w-0">
                  <span class="truncate font-mono text-gray-300">
                    {provider.name ?? provider.provider_name ?? 'unnamed'}
                  </span>
                  <span class="truncate text-[10px] text-gray-300">
                    {provider.provider_type ?? provider.type ?? 'unknown'}
                    {provider.model ? ` / ${provider.model}` : ''}
                  </span>
                </div>
              </div>
            )}
          </For>
        </div>
      </div>

      {/* FLEET */}
      <div class="border-b border-gray-800 px-3 py-3">
        <h3
          class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300 cursor-pointer hover:text-cyan-300 transition-colors"
          onClick={() => openPane('fleet-view', 'Fleet')}
        >Fleet</h3>
        <Show when={fleetNodes().length === 0}>
          <p class="text-xs text-gray-300">No fleet nodes</p>
        </Show>
        <div class="space-y-1">
          <For each={fleetNodes()}>
            {(node) => (
              <div class="flex items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-gray-800 transition-colors">
                <span class={`h-2 w-2 shrink-0 rounded-full ${nodeStatusColor(node)}`} />
                <span class="truncate font-mono text-gray-300">
                  {node.hostname ?? node.name ?? 'unknown'}
                </span>
                <span class="ml-auto text-[10px] text-gray-300">
                  {node.agent_count ?? node.agents ?? 0} agents
                </span>
              </div>
            )}
          </For>
        </div>
      </div>

      {/* TOKENS */}
      <div class="px-3 py-3">
        <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300">Tokens</h3>
        <div class="space-y-1.5 text-xs">
          <div class="flex items-center justify-between">
            <span class="text-gray-300">In:</span>
            <span class="font-mono text-gray-300">--</span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-gray-300">Out:</span>
            <span class="font-mono text-gray-300">--</span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-gray-300">Cost:</span>
            <span class="font-mono text-gray-300">--</span>
          </div>
        </div>
      </div>
    </aside>
  );
};

export default RightPanel;
