/**
 * OrgChart.tsx — Role Hierarchy visualization
 *
 * Displays persona hierarchy parsed from YAML files:
 * - Personas are static role definitions (not live agents)
 * - Shows CEO/Executives → Leads → ICs
 * - Communication channels and reporting lines
 * - Template for how agents should organize when spawned
 */

import { Component, For, Show, createSignal, onMount, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";

interface AgentOrgNode {
  name: string;
  role: string;
  tier: string;
  reports_to: string | null;
  direct_reports: string[];
  communication?: {
    channels: string[];
    peers: string[];
    can_dm: string[];
  };
}

interface OrgChartData {
  nodes: AgentOrgNode[];
  root: string;
}

const OrgChart: Component = () => {
  const [data, setData] = createSignal<OrgChartData | null>(null);
  const [selectedAgent, setSelectedAgent] = createSignal<AgentOrgNode | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  onMount(async () => {
    try {
      const response = await restClient.get("/api/org/chart");
      setData(response);
      setLoading(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load org chart");
      setLoading(false);
    }
  });

  const tierColor = (tier: string) => {
    switch (tier) {
      case "executive":
        return "bg-purple-900 border-purple-600";
      case "lead":
        return "bg-blue-900 border-blue-600";
      case "ic":
        return "bg-green-900 border-green-600";
      default:
        return "bg-gray-900 border-gray-600";
    }
  };

  const tierLabel = (tier: string) => {
    switch (tier) {
      case "executive":
        return "C-Suite";
      case "lead":
        return "Lead";
      case "ic":
        return "IC";
      default:
        return tier;
    }
  };

  // Group agents by tier for vertical layout
  const agentsByTier = createMemo(() => {
    const d = data();
    if (!d) return { executive: [], lead: [], ic: [] };

    const groups: Record<string, AgentOrgNode[]> = {
      executive: [],
      lead: [],
      ic: [],
    };

    for (const node of d.nodes) {
      const tier = node.tier || "ic";
      if (groups[tier]) {
        groups[tier].push(node);
      }
    }

    return groups;
  });

  // Build reporting tree
  const reportingTree = createMemo(() => {
    const d = data();
    if (!d) return new Map<string, AgentOrgNode[]>();

    const tree = new Map<string, AgentOrgNode[]>();

    for (const node of d.nodes) {
      const manager = node.reports_to || "ceo";
      if (!tree.has(manager)) {
        tree.set(manager, []);
      }
      tree.get(manager)!.push(node);
    }

    return tree;
  });

  const AgentCard: Component<{ agent: AgentOrgNode }> = (props) => {
    const isSelected = () => selectedAgent()?.name === props.agent.name;

    return (
      <div
        class={`
          cursor-pointer transition-all p-3 rounded border-2
          ${tierColor(props.agent.tier)}
          ${isSelected() ? "ring-2 ring-cyan-400 scale-105" : "hover:scale-102"}
        `}
        onClick={() => setSelectedAgent(props.agent)}
      >
        <div class="font-semibold text-sm text-white">{props.agent.name}</div>
        <div class="text-xs text-gray-400 mt-1">{props.agent.role}</div>
        <div class="text-xs text-gray-500 mt-1">
          {tierLabel(props.agent.tier)}
        </div>
        <Show when={props.agent.direct_reports.length > 0}>
          <div class="text-xs text-cyan-400 mt-1">
            {props.agent.direct_reports.length} reports
          </div>
        </Show>
      </div>
    );
  };

  const AgentDetails: Component = () => {
    const agent = selectedAgent();
    if (!agent) return null;

    return (
      <div class="bg-gray-900 border-2 border-cyan-500 rounded-lg p-6">
        <h3 class="text-xl font-bold text-white mb-2">{agent.name}</h3>
        <p class="text-gray-400 mb-4">{agent.role}</p>

        <div class="space-y-4">
          <Show when={agent.reports_to}>
            <div>
              <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                Reports To
              </div>
              <div class="text-cyan-400">{agent.reports_to}</div>
            </div>
          </Show>

          <Show when={agent.direct_reports.length > 0}>
            <div>
              <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                Direct Reports ({agent.direct_reports.length})
              </div>
              <div class="flex flex-wrap gap-2">
                <For each={agent.direct_reports}>
                  {(report) => (
                    <span class="px-2 py-1 bg-blue-900 text-blue-200 text-xs rounded">
                      {report}
                    </span>
                  )}
                </For>
              </div>
            </div>
          </Show>

          <Show when={agent.communication}>
            <div>
              <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                Channels
              </div>
              <div class="flex flex-wrap gap-2">
                <For each={agent.communication!.channels}>
                  {(channel) => (
                    <span class="px-2 py-1 bg-purple-900 text-purple-200 text-xs rounded">
                      {channel}
                    </span>
                  )}
                </For>
              </div>
            </div>

            <Show when={agent.communication!.peers.length > 0}>
              <div>
                <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                  Peers
                </div>
                <div class="flex flex-wrap gap-2">
                  <For each={agent.communication!.peers}>
                    {(peer) => (
                      <span class="px-2 py-1 bg-green-900 text-green-200 text-xs rounded">
                        {peer}
                      </span>
                    )}
                  </For>
                </div>
              </div>
            </Show>

            <Show when={agent.communication!.can_dm.length > 0}>
              <div>
                <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                  Can DM
                </div>
                <div class="flex flex-wrap gap-2">
                  <For each={agent.communication!.can_dm}>
                    {(dm) => (
                      <span class="px-2 py-1 bg-gray-800 text-gray-300 text-xs rounded">
                        {dm}
                      </span>
                    )}
                  </For>
                </div>
              </div>
            </Show>
          </Show>
        </div>
      </div>
    );
  };

  return (
    <div class="p-6 bg-gray-950 min-h-screen">
      <div class="mb-6">
        <h1 class="text-2xl font-bold text-white mb-2">Role Hierarchy</h1>
        <p class="text-gray-400">
          Persona definitions and communication channels (not live agents)
        </p>
      </div>

      <Show when={loading()}>
        <div class="text-center py-12 text-gray-400">Loading org chart...</div>
      </Show>

      <Show when={error()}>
        <div class="bg-red-900/20 border border-red-600 rounded p-4 text-red-400">
          {error()}
        </div>
      </Show>

      <Show when={!loading() && !error() && data()}>
        <div class="grid grid-cols-1 lg:grid-cols-3 gap-6">
          {/* Org Chart */}
          <div class="lg:col-span-2 space-y-8">
            {/* Executives */}
            <Show when={agentsByTier().executive.length > 0}>
              <div>
                <h2 class="text-lg font-semibold text-purple-400 mb-3">
                  Executives
                </h2>
                <div class="grid grid-cols-2 md:grid-cols-3 gap-4">
                  <For each={agentsByTier().executive}>
                    {(agent) => <AgentCard agent={agent} />}
                  </For>
                </div>
              </div>
            </Show>

            {/* Leads */}
            <Show when={agentsByTier().lead.length > 0}>
              <div>
                <h2 class="text-lg font-semibold text-blue-400 mb-3">Leads</h2>
                <div class="grid grid-cols-2 md:grid-cols-4 gap-4">
                  <For each={agentsByTier().lead}>
                    {(agent) => <AgentCard agent={agent} />}
                  </For>
                </div>
              </div>
            </Show>

            {/* ICs */}
            <Show when={agentsByTier().ic.length > 0}>
              <div>
                <h2 class="text-lg font-semibold text-green-400 mb-3">
                  Individual Contributors
                </h2>
                <div class="grid grid-cols-2 md:grid-cols-4 gap-4">
                  <For each={agentsByTier().ic}>
                    {(agent) => <AgentCard agent={agent} />}
                  </For>
                </div>
              </div>
            </Show>
          </div>

          {/* Agent Details */}
          <div class="lg:col-span-1">
            <Show
              when={selectedAgent()}
              fallback={
                <div class="bg-gray-900 border border-gray-700 rounded-lg p-6 text-center text-gray-500">
                  Select an agent to view details
                </div>
              }
            >
              <AgentDetails />
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default OrgChart;
