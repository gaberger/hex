/**
 * OrgChart.tsx — Role Hierarchy visualization
 *
 * Displays persona hierarchy parsed from YAML files:
 * - Personas are static role definitions (not live agents)
 * - Shows CEO/Executives → Leads → ICs
 * - Communication channels and reporting lines
 * - Template for how agents should organize when spawned
 */

import { Component, For, Show, createSignal, onMount, createMemo, createEffect } from "solid-js";
import { restClient } from "../../services/rest-client";
import OrgChartTree from "./OrgChartTree";
import { registryAgents } from "../../stores/connection";

interface AgentOrgNode {
  name: string;
  role: string;
  tier: string;
  status?: string;
  last_heartbeat?: string | null;
  active_agents?: number;
  reports_to: string | null;
  direct_reports: string[];
  communication?: {
    channels: string[];
    peers: string[];
    can_dm: string[];
  };
}

interface OrgChartData {
  nodes?: AgentOrgNode[];
  personas?: AgentOrgNode[];
  root: string;
}

const OrgChart: Component = () => {
  const [data, setData] = createSignal<OrgChartData | null>(null);
  const [selectedAgent, setSelectedAgent] = createSignal<AgentOrgNode | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  const fetchPersonas = async () => {
    try {
      const response = await restClient.get("/api/org/personas");
      console.log('[OrgChart] Fetched personas:', response);
      if (response.personas) {
        console.log('[OrgChart] Setting data with', response.personas.length, 'personas');
        setData({ nodes: response.personas, root: response.root });
      } else {
        setData(response);
      }
      setLoading(false);
    } catch (err) {
      console.error('[OrgChart] Error fetching personas:', err);
      setError(err instanceof Error ? err.message : "Failed to load org chart");
      setLoading(false);
    }
  };

  onMount(() => {
    fetchPersonas();
  });

  // Auto-refresh when agents change (real-time updates via SpacetimeDB subscription)
  createEffect(() => {
    const agents = registryAgents();
    if (agents.length > 0 && !loading()) {
      console.log('[OrgChart] Agents changed, refreshing personas');
      fetchPersonas();
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

    const nodes = d.nodes || d.personas || [];
    for (const node of nodes) {
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
    const nodes = d.nodes || d.personas || [];

    for (const node of nodes) {
      const manager = node.reports_to || "root";
      if (!tree.has(manager)) {
        tree.set(manager, []);
      }
      tree.get(manager)!.push(node);
    }

    return tree;
  });

  // Find all top-level nodes (no reports_to or reports_to is null/empty)
  const topLevelNodes = createMemo(() => {
    const d = data();
    if (!d) return [];

    const nodes = d.nodes || d.personas || [];
    return nodes.filter(n => !n.reports_to || n.reports_to === '');
  });

  // Recursive tree node component
  const TreeNode: Component<{ agent: AgentOrgNode | null; allNodes: AgentOrgNode[]; level: number }> = (props) => {
    if (!props.agent) return null;

    const children = createMemo(() =>
      props.allNodes.filter(n => n.reports_to === props.agent!.name)
    );

    const isSelected = () => selectedAgent()?.name === props.agent!.name;

    return (
      <div class="flex flex-col items-center">
        {/* Current node */}
        <div
          class={`
            cursor-pointer transition-all p-3 rounded border-2 min-w-[200px] relative
            ${tierColor(props.agent.tier)}
            ${isSelected() ? "ring-2 ring-cyan-400 scale-105" : "hover:scale-102"}
          `}
          onClick={() => setSelectedAgent(props.agent!)}
        >
          <Show when={props.agent.status}>
            <div class={`absolute top-2 right-2 w-2 h-2 rounded-full ${
              props.agent.status === 'online' ? 'bg-green-400 shadow-green-400/50 shadow-lg' : 'bg-gray-600'
            }`} title={props.agent.status}></div>
          </Show>
          <div class="font-semibold text-sm text-white">{props.agent.name}</div>
          <div class="text-xs text-gray-400 mt-1">{props.agent.role}</div>
          <div class="text-xs text-gray-500 mt-1">
            {tierLabel(props.agent.tier)}
          </div>
          <Show when={props.agent.active_agents && props.agent.active_agents > 0}>
            <div class="text-xs text-green-400 mt-1">
              {props.agent.active_agents} agent{props.agent.active_agents !== 1 ? 's' : ''}
            </div>
          </Show>
          <Show when={children().length > 0}>
            <div class="text-xs text-cyan-400 mt-1">
              {children().length} report{children().length !== 1 ? 's' : ''}
            </div>
          </Show>
        </div>

        {/* Connecting line and children */}
        <Show when={children().length > 0}>
          <div class="flex flex-col items-center mt-4">
            {/* Vertical line down */}
            <div class="w-0.5 h-6 bg-gray-700"></div>

            {/* Horizontal line across children */}
            <Show when={children().length > 1}>
              <div class="relative w-full h-0.5 bg-gray-700" style={`width: ${children().length * 220}px`}>
                {/* Vertical drops to each child */}
                <For each={children()}>
                  {(_, idx) => (
                    <div
                      class="absolute top-0 w-0.5 h-6 bg-gray-700"
                      style={`left: ${((idx() + 0.5) / children().length) * 100}%`}
                    ></div>
                  )}
                </For>
              </div>
            </Show>

            {/* Children nodes */}
            <div class="flex gap-4 mt-6">
              <For each={children()}>
                {(child) => <TreeNode agent={child} allNodes={props.allNodes} level={props.level + 1} />}
              </For>
            </div>
          </div>
        </Show>
      </div>
    );
  };

  const AgentCard: Component<{ agent: AgentOrgNode }> = (props) => {
    const isSelected = () => selectedAgent()?.name === props.agent.name;

    return (
      <div
        class={`
          cursor-pointer transition-all p-3 rounded border-2 relative
          ${tierColor(props.agent.tier)}
          ${isSelected() ? "ring-2 ring-cyan-400 scale-105" : "hover:scale-102"}
        `}
        onClick={() => setSelectedAgent(props.agent)}
      >
        <Show when={props.agent.status}>
          <div class={`absolute top-2 right-2 w-2 h-2 rounded-full ${
            props.agent.status === 'online' ? 'bg-green-400 shadow-green-400/50 shadow-lg' : 'bg-gray-600'
          }`} title={props.agent.status}></div>
        </Show>
        <div class="font-semibold text-sm text-white">{props.agent.name}</div>
        <div class="text-xs text-gray-400 mt-1">{props.agent.role}</div>
        <div class="text-xs text-gray-500 mt-1">
          {tierLabel(props.agent.tier)}
        </div>
        <Show when={props.agent.active_agents && props.agent.active_agents > 0}>
          <div class="text-xs text-green-400 mt-1">
            {props.agent.active_agents} agent{props.agent.active_agents !== 1 ? 's' : ''}
          </div>
        </Show>
        <Show when={props.agent.direct_reports.length > 0}>
          <div class="text-xs text-cyan-400 mt-1">
            {props.agent.direct_reports.length} reports
          </div>
        </Show>
      </div>
    );
  };

  const AgentDetails: Component = () => {
    return (
      <Show when={selectedAgent()} fallback={null}>
        {(agent) => (
          <div class="bg-gray-900 border-2 border-cyan-500 rounded-lg p-6">
            <h3 class="text-xl font-bold text-white mb-2">{agent().name}</h3>
            <p class="text-gray-400 mb-4">{agent().role}</p>

            <div class="space-y-4">
              <Show when={agent().reports_to}>
                <div>
                  <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                    Reports To
                  </div>
                  <div class="text-cyan-400">{agent().reports_to}</div>
                </div>
              </Show>

              <Show when={agent().direct_reports.length > 0}>
                <div>
                  <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                    Direct Reports ({agent().direct_reports.length})
                  </div>
                  <div class="flex flex-wrap gap-2">
                    <For each={agent().direct_reports}>
                      {(report) => (
                        <span class="px-2 py-1 bg-blue-900 text-blue-200 text-xs rounded">
                          {report}
                        </span>
                      )}
                    </For>
                  </div>
                </div>
              </Show>

              <Show when={agent().communication}>
                <div>
                  <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                    Channels
                  </div>
                  <div class="flex flex-wrap gap-2">
                    <For each={agent().communication!.channels}>
                      {(channel) => (
                        <span class="px-2 py-1 bg-purple-900 text-purple-200 text-xs rounded">
                          {channel}
                        </span>
                      )}
                    </For>
                  </div>
                </div>

                <Show when={agent().communication!.peers.length > 0}>
                  <div>
                    <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                      Peers
                    </div>
                    <div class="flex flex-wrap gap-2">
                      <For each={agent().communication!.peers}>
                        {(peer) => (
                          <span class="px-2 py-1 bg-green-900 text-green-200 text-xs rounded">
                            {peer}
                          </span>
                        )}
                      </For>
                    </div>
                  </div>
                </Show>

                <Show when={agent().communication!.can_dm.length > 0}>
                  <div>
                    <div class="text-xs text-gray-500 uppercase tracking-wider mb-1">
                      Can DM
                    </div>
                    <div class="flex flex-wrap gap-2">
                      <For each={agent().communication!.can_dm}>
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
        )}
      </Show>
    );
  };

  return (
    <div class="flex flex-col bg-gray-950 h-screen">
      <div class="p-6 border-b border-gray-800">
        <h1 class="text-2xl font-bold text-white mb-2">Role Hierarchy</h1>
        <p class="text-gray-400">
          Live agent status and organizational structure
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
        <div class="flex flex-1 gap-6 overflow-hidden">
          {/* SVG Tree Visualization */}
          <div class="flex-1 overflow-hidden">
            <OrgChartTree
              nodes={data()!.nodes || data()!.personas || []}
              selectedName={selectedAgent()?.name || null}
              onSelect={setSelectedAgent}
            />
          </div>

          {/* Agent Details */}
          <div class="w-80 shrink-0 p-6 overflow-y-auto border-l border-gray-800">
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
