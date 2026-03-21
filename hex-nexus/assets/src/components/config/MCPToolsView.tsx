import { Component, For, Show, createResource, createMemo } from 'solid-js';
import { addToast } from '../../stores/toast';
import { projectConfigs, hexfloConnected } from '../../stores/connection';

interface MCPServer {
  name: string;
  status: 'configured' | 'connected' | 'disconnected' | 'error';
  command?: string;
  args?: string[];
  tools: string[];
  totalTools: number;
}

const HARDCODED_SERVERS: MCPServer[] = [
  { name: 'hex',          status: 'connected', tools: ['hex_analyze', 'hex_swarm_init', 'hex_task_create', 'hex_memory_store'], totalTools: 32 },
  { name: 'pencil',       status: 'connected', tools: ['batch_design', 'get_screenshot', 'batch_get'], totalTools: 12 },
  { name: 'context-mode', status: 'connected', tools: ['ctx_execute', 'ctx_search', 'ctx_batch_execute'], totalTools: 6 },
];

const MAX_VISIBLE_TOOLS = 4;

async function discoverServers(): Promise<MCPServer[]> {
  const allServers: Record<string, any> = {};

  // Try both settings files
  for (const file of ['.claude/settings.json', '.claude/settings.local.json']) {
    try {
      const res = await fetch(`/api/files?path=${encodeURIComponent(file)}`);
      if (res.ok) {
        const data = await res.json();
        const parsed = JSON.parse(data.content || '{}');
        const mcpServers = parsed.mcpServers || {};
        // Merge — local overrides global
        Object.assign(allServers, mcpServers);
      }
    } catch {
      // ignore
    }
  }

  const entries = Object.entries(allServers);
  if (entries.length === 0) return HARDCODED_SERVERS;

  return entries.map(([name, config]: [string, any]) => ({
    name,
    status: 'configured' as const,
    command: config.command || '',
    args: config.args || [],
    tools: [],
    totalTools: 0,
  }));
}

const MCPToolsView: Component = () => {
  const [servers] = createResource(discoverServers);

  // Primary: SpacetimeDB subscription
  const stdbServers = createMemo((): MCPServer[] | null => {
    const configs = projectConfigs();
    const mcpConfig = configs.find((c: any) => (c.key ?? c.configKey) === 'mcp_servers');
    if (mcpConfig) {
      try {
        const parsed = JSON.parse(mcpConfig.valueJson ?? mcpConfig.value_json ?? '{}');
        return Object.entries(parsed).map(([name, config]: [string, any]) => ({
          name,
          status: 'configured' as const,
          command: config.command || '',
          args: config.args || [],
          tools: [],
          totalTools: 0,
        }));
      } catch { /* fall through */ }
    }
    return null;
  });

  const dataSource = createMemo(() => stdbServers() !== null ? 'stdb' as const : 'rest' as const);
  const serverList = () => stdbServers() ?? servers() ?? HARDCODED_SERVERS;

  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">MCP Tool Servers</h2>
          <p class="mt-1 text-sm text-gray-400">
            {servers.loading ? 'Discovering MCP servers...' : `${serverList().length} MCP servers from settings.`}
            <Show when={dataSource() === 'stdb'}>
              <span class="ml-2 inline-flex items-center rounded-full bg-cyan-900/30 px-2 py-0.5 text-[10px] font-medium text-cyan-400">SpacetimeDB</span>
            </Show>
          </p>
        </div>
        <button class="rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-gray-100 transition-colors border border-gray-700"
          onClick={() => addToast("info", "Add MCP servers in .claude/settings.json under mcpServers")}>
          Add Server
        </button>
      </div>

      {/* Server cards */}
      <div class="space-y-4">
        <For each={serverList()}>
          {(server) => {
            const visibleTools = server.tools.slice(0, MAX_VISIBLE_TOOLS);
            const remaining = server.totalTools - visibleTools.length;

            return (
              <div class="rounded-lg border border-gray-700/50 p-4" style={{ "background-color": "var(--bg-surface)" }}>
                {/* Server header */}
                <div class="flex items-center gap-3 mb-3">
                  <span
                    class="h-2.5 w-2.5 shrink-0 rounded-full"
                    classList={{
                      "bg-green-500": server.status === 'connected',
                      "bg-cyan-500": server.status === 'configured',
                      "bg-red-500": server.status === 'error',
                      "bg-gray-500": server.status === 'disconnected',
                    }}
                  />
                  <span class="text-base font-bold text-gray-200" style={{ "font-family": "'JetBrains Mono', monospace" }}>
                    {server.name}
                  </span>
                  <span class="rounded-full px-2.5 py-0.5 text-xs font-medium"
                    classList={{
                      "bg-green-900/30 text-green-400": server.status === 'connected',
                      "bg-cyan-900/30 text-cyan-400": server.status === 'configured',
                      "bg-red-900/30 text-red-400": server.status === 'error',
                      "bg-gray-800 text-gray-500": server.status === 'disconnected',
                    }}
                  >
                    {server.status}
                  </span>
                  <Show when={server.totalTools > 0}>
                    <span class="ml-auto text-sm text-gray-500">
                      {server.totalTools} tools
                    </span>
                  </Show>
                </div>

                {/* Command info for discovered servers */}
                <Show when={server.command}>
                  <div class="mb-2 text-xs text-gray-500 truncate" style={{ "font-family": "'JetBrains Mono', monospace" }}>
                    {server.command} {(server.args || []).join(' ')}
                  </div>
                </Show>

                {/* Tool badges */}
                <div class="flex flex-wrap items-center gap-2">
                  <For each={visibleTools}>
                    {(tool) => (
                      <span
                        class="rounded-md bg-gray-800 px-2.5 py-1 text-xs text-gray-400 border border-gray-700/50"
                        style={{ "font-family": "'JetBrains Mono', monospace" }}
                      >
                        {tool}
                      </span>
                    )}
                  </For>
                  <Show when={remaining > 0}>
                    <span class="text-xs text-gray-600">+{remaining} more</span>
                  </Show>
                  <Show when={visibleTools.length === 0 && server.status === 'configured'}>
                    <span class="text-xs text-gray-600 italic">Tools available after MCP connection</span>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
};

export default MCPToolsView;
