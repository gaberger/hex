/**
 * ContextPanel.tsx — Context-sensitive right panel.
 *
 * Shows a compact default view (nexus status + tokens) when nothing is
 * selected, and switches to agent/swarm detail views when the user clicks
 * an item in the sidebar.
 */
import { Component, Show, Switch, Match, For, createMemo } from 'solid-js';
import {
  registryAgents, swarms, swarmTasks, swarmAgents,
  inferenceProviders, inferenceRequests, fleetNodes,
  hexfloConnected, agentRegistryConnected,
  inferenceConnected, fleetConnected,
} from '../../stores/connection';
import { openPane } from '../../stores/panes';
import { nexusStatus } from '../../stores/nexus-health';
import { panelContent, setPanelContent, resetPanel } from '../../stores/context-panel';
import HealthPane from '../health/HealthPane';
import InferencePanel from '../fleet/InferencePanel';

function healthColor(status: string): string {
  if (status === 'healthy' || status === 'active' || status === 'online') return 'bg-green-500';
  if (status === 'degraded' || status === 'stale') return 'bg-yellow-500';
  if (status === 'error' || status === 'dead' || status === 'offline') return 'bg-red-500';
  return 'bg-gray-500';
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

const ContextPanel: Component = () => {
  const content = panelContent;
  const status = nexusStatus;

  const tokenStats = createMemo(() => {
    let totalIn = 0, totalOut = 0;
    for (const r of inferenceRequests()) {
      totalIn += r.input_tokens ?? r.prompt_tokens ?? 0;
      totalOut += r.output_tokens ?? r.completion_tokens ?? 0;
    }
    return { totalIn, totalOut, cost: ((totalIn + totalOut) / 1000) * 0.005, requests: inferenceRequests().length };
  });

  const connCount = () => [hexfloConnected(), agentRegistryConnected(), inferenceConnected(), fleetConnected()].filter(Boolean).length;

  return (
    <aside class="flex h-full w-72 lg:w-80 flex-col border-l border-gray-800 bg-gray-900 overflow-y-auto">
      <Switch>
        {/* ── DEFAULT VIEW ── */}
        <Match when={content().type === "default"}>
          {/* Nexus status */}
          <div class="border-b border-gray-800 px-3 py-3">
            <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500">Nexus</h3>
            <div class="space-y-1.5 text-xs">
              <div class="flex items-center gap-2">
                <span class={`h-2 w-2 rounded-full ${status().online ? 'bg-green-500' : 'bg-red-500'}`} />
                <span class="text-gray-200">{status().online ? 'Online' : 'Offline'}</span>
                <Show when={status().online}>
                  <span class="ml-auto font-mono text-gray-500">v{status().version}</span>
                </Show>
              </div>
              <div class="flex items-center justify-between text-gray-400">
                <span>Agents</span>
                <span class="font-mono text-gray-300">{registryAgents().length}</span>
              </div>
              <div class="flex items-center justify-between text-gray-400">
                <span>Swarms</span>
                <span class="font-mono text-gray-300">{swarms().length}</span>
              </div>
            </div>
          </div>

          {/* SpacetimeDB connections */}
          <div class="border-b border-gray-800 px-3 py-3">
            <div class="mb-2 flex items-center justify-between">
              <h3 class="text-[11px] font-semibold uppercase tracking-wider text-gray-500">SpacetimeDB</h3>
              <button
                class="rounded border border-gray-700 px-2 py-0.5 text-[9px] text-gray-500 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
                onClick={() => {
                  Object.keys(localStorage)
                    .filter(k => k.startsWith('stdb_token_'))
                    .forEach(k => localStorage.removeItem(k));
                  location.reload();
                }}
                title="Clear cached tokens and reconnect"
              >
                Reconnect
              </button>
            </div>
            <div class="space-y-1 text-xs">
              <ConnStatus label="hexflo" connected={hexfloConnected()} />
              <ConnStatus label="agents" connected={agentRegistryConnected()} />
              <ConnStatus label="inference" connected={inferenceConnected()} />
              <ConnStatus label="fleet" connected={fleetConnected()} />
            </div>
          </div>

          {/* Inference providers */}
          <div class="border-b border-gray-800 px-3 py-3">
            <h3
              class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500 cursor-pointer hover:text-cyan-300 transition-colors"
              onClick={() => setPanelContent({ type: "inference" })}
              title="Click for full inference management"
            >Inference →</h3>
            <Show when={inferenceProviders().length === 0}>
              <p class="text-xs text-gray-600">No providers registered</p>
            </Show>
            <div class="space-y-1">
              <For each={inferenceProviders()}>
                {(provider) => {
                  const name = provider.providerId ?? provider.provider_id ?? provider.name ?? provider.provider_name ?? 'unnamed';
                  const pType = provider.providerType ?? provider.provider_type ?? provider.type ?? '';
                  const isHealthy = typeof provider.healthy === 'number' ? provider.healthy === 1 : ['healthy', 'active', 'online'].includes(provider.status ?? provider.health ?? '');
                  return (
                    <div class="flex items-center gap-2 rounded px-2 py-1 text-xs hover:bg-gray-800 transition-colors cursor-pointer"
                      onClick={() => setPanelContent({ type: "inference" })}>
                      <span class={`h-2 w-2 shrink-0 rounded-full ${isHealthy ? 'bg-green-500' : 'bg-gray-500'}`} />
                      <div class="flex flex-col min-w-0">
                        <span class="truncate font-mono text-gray-200">{name}</span>
                        <span class="truncate text-[10px] text-gray-500">{pType}</span>
                      </div>
                    </div>
                  );
                }}
              </For>
            </div>
          </div>

          {/* Fleet nodes */}
          <div class="border-b border-gray-800 px-3 py-3">
            <h3
              class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500 cursor-pointer hover:text-cyan-300 transition-colors"
              onClick={() => setPanelContent({ type: "fleet" })}
              title="Click for fleet management"
            >Fleet →</h3>
            <Show when={fleetNodes().length === 0}>
              <p class="text-xs text-gray-600">No fleet nodes</p>
            </Show>
            <div class="space-y-1">
              <For each={fleetNodes()}>
                {(node) => (
                  <div class="flex items-center gap-2 rounded px-2 py-1 text-xs hover:bg-gray-800 transition-colors">
                    <span class={`h-2 w-2 shrink-0 rounded-full ${healthColor(node?.status ?? node?.state ?? '')}`} />
                    <span class="truncate font-mono text-gray-200">{node.hostname ?? node.name ?? 'unknown'}</span>
                    <span class="ml-auto text-[10px] text-gray-500">{node.agent_count ?? node.agents ?? 0} agents</span>
                  </div>
                )}
              </For>
            </div>
          </div>

          {/* Token stats */}
          <div class="px-3 py-3">
            <h3 class="mb-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500">Tokens</h3>
            <div class="space-y-1.5 text-xs">
              <div class="flex items-center justify-between">
                <span class="text-gray-400">Input</span>
                <span class="font-mono text-gray-300">{formatTokens(tokenStats().totalIn)}</span>
              </div>
              <div class="flex items-center justify-between">
                <span class="text-gray-400">Output</span>
                <span class="font-mono text-gray-300">{formatTokens(tokenStats().totalOut)}</span>
              </div>
              <div class="flex items-center justify-between">
                <span class="text-gray-400">Est. Cost</span>
                <span class="font-mono text-gray-300">${tokenStats().cost.toFixed(2)}</span>
              </div>
              <div class="flex items-center justify-between">
                <span class="text-gray-400">Requests</span>
                <span class="font-mono text-gray-300">{tokenStats().requests}</span>
              </div>
            </div>
          </div>
        </Match>

        {/* ── AGENT DETAIL ── */}
        <Match when={content().type === "agent-detail"}>
          {(() => {
            const c = content() as { type: "agent-detail"; agentId: string; agentName: string };
            const agent = () => registryAgents().find((a: any) => (a.id ?? a.agent_id) === c.agentId);
            return (
              <div class="flex flex-col">
                {/* Header with back button */}
                <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-2">
                  <button class="rounded p-1 text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors" onClick={resetPanel}>
                    <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="15 18 9 12 15 6" /></svg>
                  </button>
                  <span class="text-xs font-semibold text-gray-200">{c.agentName}</span>
                  <span class="ml-auto text-[10px] text-gray-500">Agent</span>
                </div>
                <div class="px-3 py-3 space-y-2 text-xs">
                  <Show when={agent()} fallback={<p class="text-gray-500">Agent not found in registry</p>}>
                    {(a) => (
                      <>
                        <div class="flex justify-between"><span class="text-gray-400">Status</span><span class="font-mono text-gray-200">{a().status ?? a().state ?? 'unknown'}</span></div>
                        <div class="flex justify-between"><span class="text-gray-400">Role</span><span class="font-mono text-gray-200">{a().role ?? a().agent_name ?? '--'}</span></div>
                        <div class="flex justify-between"><span class="text-gray-400">Project</span><span class="font-mono text-gray-200 truncate max-w-[140px]">{a().project ?? '--'}</span></div>
                        <div class="flex justify-between"><span class="text-gray-400">ID</span><span class="font-mono text-gray-500 text-[10px]">{(a().id ?? a().agent_id ?? '').slice(0, 12)}</span></div>
                      </>
                    )}
                  </Show>
                </div>
              </div>
            );
          })()}
        </Match>

        {/* ── SWARM DETAIL ── */}
        <Match when={content().type === "swarm-detail"}>
          {(() => {
            const c = content() as { type: "swarm-detail"; swarmId: string; swarmName: string };
            const swarm = () => swarms().find((s: any) => (s.id ?? s.swarm_id) === c.swarmId);
            const tasks = () => swarmTasks().filter((t: any) => (t.swarmId ?? t.swarm_id) === c.swarmId);
            const agents = () => swarmAgents().filter((a: any) => (a.swarmId ?? a.swarm_id) === c.swarmId);
            const doneCount = () => tasks().filter((t: any) => t.status === 'completed').length;

            return (
              <div class="flex flex-col">
                <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-2">
                  <button class="rounded p-1 text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors" onClick={resetPanel}>
                    <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="15 18 9 12 15 6" /></svg>
                  </button>
                  <span class="text-xs font-semibold text-gray-200">{c.swarmName}</span>
                  <span class="ml-auto rounded bg-cyan-900/40 px-1.5 py-0.5 text-[10px] text-cyan-400">{swarm()?.topology ?? 'mesh'}</span>
                </div>

                <div class="px-3 py-3 space-y-3 text-xs">
                  {/* Progress */}
                  <div>
                    <div class="flex justify-between mb-1 text-gray-400">
                      <span>Progress</span>
                      <span class="font-mono text-gray-300">{doneCount()}/{tasks().length}</span>
                    </div>
                    <div class="h-1.5 rounded bg-gray-800 overflow-hidden">
                      <div class="h-full rounded bg-cyan-500 transition-all" style={{ width: tasks().length > 0 ? `${(doneCount() / tasks().length) * 100}%` : '0%' }} />
                    </div>
                  </div>

                  {/* Agents */}
                  <div>
                    <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-gray-500">Agents ({agents().length})</div>
                    <Show when={agents().length === 0}><p class="text-gray-600">No agents assigned</p></Show>
                    <For each={agents()}>
                      {(a) => (
                        <div class="flex items-center gap-2 py-1">
                          <span class="h-1.5 w-1.5 rounded-full bg-green-500" />
                          <span class="font-mono text-gray-300">{a.name ?? a.agent_name ?? 'agent'}</span>
                        </div>
                      )}
                    </For>
                  </div>

                  {/* Tasks */}
                  <div>
                    <div class="mb-1 text-[10px] font-semibold uppercase tracking-wider text-gray-500">Tasks ({tasks().length})</div>
                    <Show when={tasks().length === 0}><p class="text-gray-600">No tasks</p></Show>
                    <For each={tasks()}>
                      {(t) => (
                        <div class="flex items-center gap-2 py-1 rounded px-1 hover:bg-gray-800/50">
                          <span class="h-1.5 w-1.5 shrink-0 rounded-full"
                            classList={{
                              "bg-green-500": t.status === "completed",
                              "bg-cyan-500 animate-pulse": t.status === "in-progress" || t.status === "in_progress",
                              "bg-gray-600": t.status === "pending",
                              "bg-red-500": t.status === "failed",
                            }}
                          />
                          <span class="truncate text-gray-300">{t.title ?? t.name ?? 'task'}</span>
                          <span class="ml-auto text-[10px] uppercase text-gray-600">{t.status}</span>
                        </div>
                      )}
                    </For>
                  </div>
                </div>
              </div>
            );
          })()}
        </Match>
        {/* ── DEP GRAPH HINT ── */}
        <Match when={content().type === "dep-graph"}>
          <div class="flex flex-col">
            <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-2">
              <button class="rounded p-1 text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors" onClick={resetPanel}>
                <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="15 18 9 12 15 6" /></svg>
              </button>
              <span class="text-xs font-semibold text-gray-200">Dependency Graph</span>
            </div>
            <div class="px-3 py-4 text-xs text-gray-400">
              <p>Open in Panes view for the full interactive graph.</p>
              <button
                class="mt-3 rounded border border-gray-700 px-3 py-1.5 text-[11px] text-gray-300 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
                onClick={() => openPane('dep-graph', 'Dependencies')}
              >
                Open Graph Pane
              </button>
            </div>
          </div>
        </Match>
        {/* ── HEALTH DETAIL ── */}
        <Match when={content().type === "health-detail"}>
          <div class="flex flex-col">
            <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-2">
              <button class="rounded p-1 text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors" onClick={resetPanel}>
                <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="15 18 9 12 15 6" /></svg>
              </button>
              <span class="text-xs font-semibold text-gray-200">Architecture Health</span>
              <span class="ml-auto text-[10px] text-gray-500">Analysis</span>
            </div>
            <HealthPane />
          </div>
        </Match>

        {/* ── INFERENCE ── */}
        <Match when={content().type === "inference"}>
          <div class="flex h-full flex-col">
            <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-2">
              <button class="rounded p-1 text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors" onClick={resetPanel}>
                <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="15 18 9 12 15 6" /></svg>
              </button>
              <span class="text-xs font-semibold text-gray-200">Inference Providers</span>
              <span class="ml-auto text-[10px] text-gray-500">Config</span>
            </div>
            <div class="flex-1 overflow-auto">
              <InferencePanel />
            </div>
          </div>
        </Match>

        {/* ── FLEET ── */}
        <Match when={content().type === "fleet"}>
          <div class="flex flex-col">
            <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-2">
              <button class="rounded p-1 text-gray-500 hover:text-gray-300 hover:bg-gray-800 transition-colors" onClick={resetPanel}>
                <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><polyline points="15 18 9 12 15 6" /></svg>
              </button>
              <span class="text-xs font-semibold text-gray-200">Fleet Nodes</span>
              <span class="ml-auto text-[10px] text-gray-500">Compute</span>
            </div>
            <div class="px-3 py-3 space-y-2 text-xs">
              <Show when={fleetNodes().length === 0}>
                <p class="py-8 text-center text-gray-600">No fleet nodes registered</p>
              </Show>
              <For each={fleetNodes()}>
                {(node) => (
                  <div class="rounded-lg border border-gray-800 bg-gray-900/60 p-3">
                    <div class="flex items-center gap-2 mb-2">
                      <span class={`h-2.5 w-2.5 rounded-full ${healthColor(node?.status ?? node?.state ?? '')}`} />
                      <span class="font-mono font-medium text-gray-200">{node.hostname ?? node.name ?? 'unknown'}</span>
                    </div>
                    <div class="space-y-1 text-gray-400">
                      <div class="flex justify-between"><span>Agents</span><span class="font-mono text-gray-300">{node.agent_count ?? node.agents ?? 0}</span></div>
                      <Show when={node.cpu_usage != null}><div class="flex justify-between"><span>CPU</span><span class="font-mono text-gray-300">{node.cpu_usage}%</span></div></Show>
                      <Show when={node.memory_mb != null}><div class="flex justify-between"><span>Memory</span><span class="font-mono text-gray-300">{node.memory_mb} MB</span></div></Show>
                    </div>
                  </div>
                )}
              </For>
            </div>
          </div>
        </Match>
      </Switch>
    </aside>
  );
};

/** Per-module SpacetimeDB connection indicator. */
const ConnStatus: Component<{ label: string; connected: boolean }> = (props) => (
  <div class="flex items-center gap-2">
    <span
      class="h-1.5 w-1.5 rounded-full"
      classList={{
        "bg-green-500": props.connected,
        "bg-yellow-500 animate-pulse": !props.connected,
      }}
    />
    <span class="font-mono text-gray-300">{props.label}</span>
    <span class="ml-auto text-[10px]"
      classList={{
        "text-green-400": props.connected,
        "text-yellow-500": !props.connected,
      }}
    >
      {props.connected ? 'connected' : 'not deployed'}
    </span>
  </div>
);

export default ContextPanel;
