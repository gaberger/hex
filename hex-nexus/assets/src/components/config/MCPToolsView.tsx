import { Component, For, Show, createMemo, createSignal } from 'solid-js';
import { addToast } from '../../stores/toast';
import { projectConfigs, hexfloConnected, getHexfloConn } from '../../stores/connection';

interface MCPServer {
  name: string;
  status: 'configured' | 'connected' | 'disconnected' | 'error';
  command?: string;
  args?: string[];
  tools: string[];
  totalTools: number;
}

const MAX_VISIBLE_TOOLS = 4;

/* ------------------------------------------------------------------ */
/*  SpacetimeDB helpers                                                */
/* ------------------------------------------------------------------ */

function getServerMap(): Record<string, any> {
  const configs = projectConfigs();
  const mcpConfig = configs.find((c: any) => (c.key ?? c.configKey) === 'mcp_servers');
  if (!mcpConfig) return {};
  try {
    return JSON.parse(mcpConfig.valueJson ?? mcpConfig.value_json ?? '{}');
  } catch {
    return {};
  }
}

function saveServerMap(map: Record<string, any>) {
  const conn = getHexfloConn();
  if (!conn) {
    addToast('error', 'SpacetimeDB not connected.');
    return false;
  }
  conn.reducers.syncConfig(
    'mcp_servers',
    'hex-intf',
    JSON.stringify(map),
    '.claude/settings.json',
    new Date().toISOString(),
  );
  return true;
}

/* ------------------------------------------------------------------ */
/*  Inline form for adding / editing an MCP server                    */
/* ------------------------------------------------------------------ */

interface ServerFormProps {
  initial?: { name: string; command: string; args: string };
  onSave: (name: string, command: string, args: string[]) => void;
  onCancel: () => void;
  submitLabel: string;
  nameDisabled?: boolean;
}

const ServerForm: Component<ServerFormProps> = (props) => {
  const [name, setName] = createSignal(props.initial?.name ?? '');
  const [command, setCommand] = createSignal(props.initial?.command ?? '');
  const [args, setArgs] = createSignal(props.initial?.args ?? '');

  const handleSubmit = (e: Event) => {
    e.preventDefault();
    const n = name().trim();
    const c = command().trim();
    if (!n || !c) {
      addToast('error', 'Name and command are required.');
      return;
    }
    const parsedArgs = args().trim() ? args().split(',').map(a => a.trim()).filter(Boolean) : [];
    props.onSave(n, c, parsedArgs);
  };

  return (
    <form onSubmit={handleSubmit} class="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-elevated)] p-4 space-y-3 mt-2">
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div>
          <label class="block text-[13px] font-medium text-gray-400 mb-1">Name</label>
          <input
            type="text"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            disabled={props.nameDisabled}
            placeholder="my-server"
            class="w-full rounded-md border px-3 py-1.5 text-sm text-gray-200 placeholder-gray-600 outline-none focus:border-[var(--accent)]"
            class="bg-[var(--bg-input)] border-[var(--border-subtle)] text-sm"
          />
        </div>
        <div>
          <label class="block text-[13px] font-medium text-gray-400 mb-1">Command</label>
          <input
            type="text"
            value={command()}
            onInput={(e) => setCommand(e.currentTarget.value)}
            placeholder="npx -y @my/server"
            class="w-full rounded-md border px-3 py-1.5 text-sm text-gray-200 placeholder-gray-600 outline-none focus:border-[var(--accent)]"
            class="bg-[var(--bg-input)] border-[var(--border-subtle)] text-sm"
          />
        </div>
        <div>
          <label class="block text-[13px] font-medium text-gray-400 mb-1">Args (comma-separated)</label>
          <input
            type="text"
            value={args()}
            onInput={(e) => setArgs(e.currentTarget.value)}
            placeholder="--port, 3000"
            class="w-full rounded-md border px-3 py-1.5 text-sm text-gray-200 placeholder-gray-600 outline-none focus:border-[var(--accent)]"
            class="bg-[var(--bg-input)] border-[var(--border-subtle)] text-sm"
          />
        </div>
      </div>
      <div class="flex items-center gap-2 justify-end">
        <button type="button" onClick={props.onCancel}
          class="rounded-md px-3 py-1.5 text-xs font-medium text-gray-400 hover:text-gray-200 transition-colors">
          Cancel
        </button>
        <button type="submit"
          class="rounded-md bg-[var(--accent)] px-4 py-1.5 text-xs font-medium text-white transition-colors">
          {props.submitLabel}
        </button>
      </div>
    </form>
  );
};

/* ------------------------------------------------------------------ */
/*  Main view                                                         */
/* ------------------------------------------------------------------ */

const MCPToolsView: Component = () => {
  const [showAddForm, setShowAddForm] = createSignal(false);
  const [editingServer, setEditingServer] = createSignal<string | null>(null);
  const [confirmRemove, setConfirmRemove] = createSignal<string | null>(null);

  const serverList = createMemo((): MCPServer[] => {
    const map = getServerMap();
    const entries = Object.entries(map);
    if (entries.length === 0) return [];
    return entries.map(([name, config]: [string, any]) => ({
      name,
      status: 'configured' as const,
      command: config.command || '',
      args: config.args || [],
      tools: [],
      totalTools: 0,
    }));
  });

  /* -- Mutations --------------------------------------------------- */

  const handleAdd = (name: string, command: string, args: string[]) => {
    const map = { ...getServerMap() };
    map[name] = { command, args };
    if (saveServerMap(map)) {
      addToast('success', `Server "${name}" added.`);
      setShowAddForm(false);
    }
  };

  const handleEdit = (name: string, command: string, args: string[]) => {
    const map = { ...getServerMap() };
    map[name] = { command, args };
    if (saveServerMap(map)) {
      addToast('success', `Server "${name}" updated.`);
      setEditingServer(null);
    }
  };

  const handleRemove = (name: string) => {
    const map = { ...getServerMap() };
    delete map[name];
    if (saveServerMap(map)) {
      addToast('success', `Server "${name}" removed.`);
      setConfirmRemove(null);
    }
  };

  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">MCP Tool Servers</h2>
          <p class="mt-1 text-sm text-gray-400">
            {hexfloConnected()
              ? `${serverList().length} MCP servers from SpacetimeDB.`
              : 'Connecting to SpacetimeDB...'}
            <span class="ml-2 inline-flex items-center rounded-full bg-cyan-900/30 px-2 py-0.5 text-[10px] font-medium text-cyan-400">SpacetimeDB</span>
          </p>
        </div>
        <button class="rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-gray-100 transition-colors border border-gray-700"
          onClick={() => setShowAddForm(!showAddForm())}>
          {showAddForm() ? 'Cancel' : 'Add Server'}
        </button>
      </div>

      {/* Add server form */}
      <Show when={showAddForm()}>
        <ServerForm
          onSave={handleAdd}
          onCancel={() => setShowAddForm(false)}
          submitLabel="Add Server"
        />
        <div class="mb-4" />
      </Show>

      {/* Empty state */}
      <Show when={serverList().length === 0}>
        <div class="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] px-6 py-10 text-center">
          <p class="text-sm text-gray-400">No MCP servers configured.</p>
          <p class="text-xs text-gray-600 mt-1">Run <code class="text-cyan-400">hex nexus start</code> to sync from repo.</p>
        </div>
      </Show>

      {/* Server cards */}
      <div class="space-y-4">
        <For each={serverList()}>
          {(server) => {
            const visibleTools = server.tools.slice(0, MAX_VISIBLE_TOOLS);
            const remaining = server.totalTools - visibleTools.length;
            const isEditing = () => editingServer() === server.name;
            const isConfirming = () => confirmRemove() === server.name;

            return (
              <div class="rounded-lg border border-gray-700/50 bg-[var(--bg-surface)] p-4">
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
                  <span class="font-mono text-base font-bold text-gray-200">
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

                  {/* Action buttons */}
                  <div class="ml-auto flex items-center gap-2">
                    <Show when={!isEditing() && !isConfirming()}>
                      <button
                        class="rounded-md px-2.5 py-1 text-xs text-gray-500 hover:text-gray-200 hover:bg-gray-700/50 transition-colors"
                        onClick={() => { setEditingServer(server.name); setConfirmRemove(null); }}
                      >
                        Edit
                      </button>
                      <button
                        class="rounded-md px-2.5 py-1 text-xs text-gray-500 hover:text-red-400 hover:bg-red-900/20 transition-colors"
                        onClick={() => { setConfirmRemove(server.name); setEditingServer(null); }}
                      >
                        Remove
                      </button>
                    </Show>
                  </div>
                </div>

                {/* Confirm remove */}
                <Show when={isConfirming()}>
                  <div class="flex items-center gap-3 rounded-md border border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-3 py-2 mb-3">
                    <span class="text-xs text-gray-300">Remove "{server.name}"?</span>
                    <button
                      class="rounded-md px-3 py-1 text-xs font-medium text-white bg-red-600 hover:bg-red-500 transition-colors"
                      onClick={() => handleRemove(server.name)}
                    >
                      Confirm
                    </button>
                    <button
                      class="rounded-md px-3 py-1 text-xs text-gray-400 hover:text-gray-200 transition-colors"
                      onClick={() => setConfirmRemove(null)}
                    >
                      Cancel
                    </button>
                  </div>
                </Show>

                {/* Inline edit form */}
                <Show when={isEditing()}>
                  <ServerForm
                    initial={{
                      name: server.name,
                      command: server.command || '',
                      args: (server.args || []).join(', '),
                    }}
                    nameDisabled={true}
                    onSave={handleEdit}
                    onCancel={() => setEditingServer(null)}
                    submitLabel="Save Changes"
                  />
                </Show>

                {/* Command info for discovered servers */}
                <Show when={server.command && !isEditing()}>
                  <div class="mb-2 font-mono text-xs text-gray-500 truncate">
                    {server.command} {(server.args || []).join(' ')}
                  </div>
                </Show>

                {/* Tool badges */}
                <Show when={!isEditing()}>
                  <div class="flex flex-wrap items-center gap-2">
                    <For each={visibleTools}>
                      {(tool) => (
                        <span
                          class="rounded-md bg-gray-800 px-2.5 py-1 text-xs text-gray-400 border border-gray-700/50"
                          class="font-mono"
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
                </Show>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
};

export default MCPToolsView;
