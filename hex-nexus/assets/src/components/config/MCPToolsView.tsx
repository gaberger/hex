import { Component, For, Show } from 'solid-js';

interface MCPServer {
  name: string;
  status: 'connected' | 'disconnected' | 'error';
  tools: string[];
  totalTools: number;
}

const SERVERS: MCPServer[] = [
  { name: 'hex',          status: 'connected', tools: ['hex_analyze', 'hex_swarm_init', 'hex_task_create', 'hex_memory_store'], totalTools: 32 },
  { name: 'pencil',       status: 'connected', tools: ['batch_design', 'get_screenshot', 'batch_get'], totalTools: 12 },
  { name: 'context-mode', status: 'connected', tools: ['ctx_execute', 'ctx_search', 'ctx_batch_execute'], totalTools: 6 },
];

const MAX_VISIBLE_TOOLS = 4;

const MCPToolsView: Component = () => {
  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">MCP Tool Servers</h2>
          <p class="mt-1 text-sm text-gray-400">
            Connected MCP servers and their available tools.
          </p>
        </div>
        <button class="rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-gray-100 transition-colors border border-gray-700">
          Add Server
        </button>
      </div>

      {/* Server cards */}
      <div class="space-y-4">
        <For each={SERVERS}>
          {(server) => {
            const visibleTools = server.tools.slice(0, MAX_VISIBLE_TOOLS);
            const remaining = server.totalTools - visibleTools.length;

            return (
              <div class="rounded-lg border border-gray-700/50 p-4" style={{ "background-color": "#111827" }}>
                {/* Server header */}
                <div class="flex items-center gap-3 mb-3">
                  <span
                    class="h-2.5 w-2.5 shrink-0 rounded-full"
                    classList={{
                      "bg-green-500": server.status === 'connected',
                      "bg-red-500": server.status === 'error',
                      "bg-gray-500": server.status === 'disconnected',
                    }}
                  />
                  <span class="text-base font-bold text-gray-200" style={{ "font-family": "'JetBrains Mono', monospace" }}>
                    {server.name}
                  </span>
                  <span class="rounded-full bg-green-900/30 px-2.5 py-0.5 text-xs font-medium text-green-400">
                    {server.status}
                  </span>
                  <span class="ml-auto text-sm text-gray-500">
                    {server.totalTools} tools
                  </span>
                </div>

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
