/**
 * RightPanel.tsx — Right sidebar: nexus status, SpacetimeDB connections,
 * inference providers, fleet nodes, token usage.
 *
 * All data is live from SpacetimeDB subscriptions + nexus health polling.
 */
import { Component, For, Show, createMemo } from 'solid-js';
import {
  inferenceProviders,
  inferenceRequests,
  fleetNodes,
  registryAgents,
  swarms,
  hexfloConnected,
  agentRegistryConnected,
  inferenceConnected,
  fleetConnected,
} from '../../stores/connection';
import { openPane } from '../../stores/panes';
import { nexusStatus } from '../../stores/nexus-health';

// ── Helpers ──

function healthColor(status: string): string {
  if (status === 'healthy' || status === 'active' || status === 'online') return 'bg-green-500';
  if (status === 'degraded' || status === 'stale') return 'bg-yellow-500';
  if (status === 'error' || status === 'dead' || status === 'offline') return 'bg-red-500';
  return 'bg-gray-300';
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

const RightPanel: Component = () => {
  // Token stats from inference requests
  const tokenStats = createMemo(() => {
    let totalIn = 0;
    let totalOut = 0;
    for (const r of inferenceRequests()) {
      totalIn += r.input_tokens ?? r.prompt_tokens ?? 0;
      totalOut += r.output_tokens ?? r.completion_tokens ?? 0;
    }
    const cost = ((totalIn + totalOut) / 1000) * 0.005; // blended estimate
    return { totalIn, totalOut, cost, requests: inferenceRequests().length };
  });

  const status = nexusStatus;

  return (
    <aside class="flex h-full w-70 flex-col border-l border-gray-800 bg-gray-900 overflow-y-auto">
      {/* NEXUS STATUS */}
      <div class="border-b border-gray-800 px-3 py-3">
        <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300">
          Nexus
        </h3>
        <div class="space-y-1.5 text-xs">
          <div class="flex items-center gap-2">
            <span class={`h-2 w-2 rounded-full ${status().online ? 'bg-green-500' : 'bg-red-500'}`} />
            <span class="text-gray-100">
              {status().online ? 'Online' : 'Offline'}
            </span>
            <Show when={status().online}>
              <span class="ml-auto font-mono text-gray-300">v{status().version}</span>
            </Show>
          </div>
          <Show when={status().online}>
            <div class="flex items-center justify-between text-gray-300">
              <span>Agents</span>
              <span class="font-mono">{registryAgents().length}</span>
            </div>
            <div class="flex items-center justify-between text-gray-300">
              <span>Swarms</span>
              <span class="font-mono">{swarms().length}</span>
            </div>
          </Show>
        </div>
      </div>

      {/* SPACETIMEDB CONNECTIONS */}
      <div class="border-b border-gray-800 px-3 py-3">
        <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300">
          SpacetimeDB <span class="font-normal text-gray-300">ws://localhost:3000</span>
        </h3>
        <div class="space-y-1 text-xs">
          <ConnStatus label="hexflo" connected={hexfloConnected()} module="hexflo-coordination" />
          <ConnStatus label="agents" connected={agentRegistryConnected()} module="agent-registry" />
          <ConnStatus label="inference" connected={inferenceConnected()} module="inference-gateway" />
          <ConnStatus label="fleet" connected={fleetConnected()} module="fleet-state" />
        </div>
        <Show when={!hexfloConnected() && !agentRegistryConnected()}>
          <p class="mt-2 text-[10px] text-gray-300 leading-relaxed">
            Modules not deployed. Publish with:
            <code class="block mt-1 rounded bg-gray-800 px-2 py-1 font-mono text-cyan-300">
              spacetime publish hexflo-coordination
            </code>
          </p>
        </Show>
      </div>

      {/* INFERENCE */}
      <div class="border-b border-gray-800 px-3 py-3">
        <h3
          class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300 cursor-pointer hover:text-cyan-300 transition-colors"
          onClick={() => openPane('inference', 'Inference')}
        >Inference</h3>
        <Show when={inferenceProviders().length === 0}>
          <p class="text-xs text-gray-300">No providers</p>
        </Show>
        <div class="space-y-1">
          <For each={inferenceProviders()}>
            {(provider) => (
              <div class="flex items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-gray-800 transition-colors">
                <span class={`h-2 w-2 shrink-0 rounded-full ${healthColor(provider?.status ?? provider?.health ?? '')}`} />
                <div class="flex flex-col min-w-0">
                  <span class="truncate font-mono text-gray-100">
                    {provider.name ?? provider.provider_name ?? 'unnamed'}
                  </span>
                  <span class="truncate text-[10px] text-gray-300">
                    {provider.provider_type ?? provider.type ?? 'unknown'}
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
                <span class={`h-2 w-2 shrink-0 rounded-full ${healthColor(node?.status ?? node?.state ?? '')}`} />
                <span class="truncate font-mono text-gray-100">
                  {node.hostname ?? node.name ?? 'unknown'}
                </span>
                <span class="ml-auto text-[10px] text-gray-300">
                  {node.agent_count ?? node.agents ?? 0}
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
            <span class="font-mono text-gray-100">{formatTokens(tokenStats().totalIn)}</span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-gray-300">Out:</span>
            <span class="font-mono text-gray-100">{formatTokens(tokenStats().totalOut)}</span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-gray-300">Cost:</span>
            <span class="font-mono text-gray-100">${tokenStats().cost.toFixed(2)}</span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-gray-300">Requests:</span>
            <span class="font-mono text-gray-100">{tokenStats().requests}</span>
          </div>
        </div>
      </div>
    </aside>
  );
};

/** Per-module SpacetimeDB connection indicator. */
const ConnStatus: Component<{ label: string; connected: boolean; module?: string }> = (props) => (
  <div class="flex items-center gap-2" title={props.module ? `Module: ${props.module}` : undefined}>
    <span
      class="h-1.5 w-1.5 rounded-full"
      classList={{
        "bg-green-500": props.connected,
        "bg-yellow-500 animate-pulse": !props.connected,
      }}
    />
    <span class="font-mono text-gray-100">{props.label}</span>
    <span class="ml-auto text-[10px]"
      classList={{
        "text-green-400": props.connected,
        "text-yellow-400": !props.connected,
      }}
    >
      {props.connected ? 'connected' : 'not deployed'}
    </span>
  </div>
);

export default RightPanel;
