import { Component, For, Show, createMemo, createSignal } from 'solid-js';
import { addToast } from '../../stores/toast';
import { setSpawnDialogOpen } from '../../stores/ui';
import { agentDefinitions, getHexfloConn, hexfloConnected } from '../../stores/connection';
import { CodeEditor } from '../editor';

interface AgentDef {
  agentId: string;
  name: string;
  role: string;
  model: string;
  desc: string;
  tools: string[];
  path: string;
  color: string;
}

const ROLE_COLORS: Record<string, string> = {
  coder: '#4ade80',
  planner: '#60a5fa',
  integrator: '#22d3ee',
  reviewer: '#a78bfa',
  tester: '#eab308',
};

const ROLES = ['coder', 'planner', 'integrator', 'reviewer', 'tester'];
const MODELS = ['opus', 'sonnet', 'haiku'];

const modelBadgeColor: Record<string, string> = {
  opus: "bg-purple-900/50 text-purple-300 border-purple-700/50",
  sonnet: "bg-blue-900/50 text-blue-300 border-blue-700/50",
  haiku: "bg-yellow-900/50 text-yellow-300 border-yellow-700/50",
};

/** Slugify a name for use as an ID. */
function slugify(s: string): string {
  return s.trim().replace(/\s+/g, '-').toLowerCase();
}

/* ------------------------------------------------------------------ */
/*  File I/O helpers (REST /api/files)                                 */
/* ------------------------------------------------------------------ */

async function readAgentContent(path: string): Promise<string> {
  try {
    const res = await fetch(`/api/files?path=${encodeURIComponent(path)}`);
    if (!res.ok) return '';
    const data = await res.json();
    return data.content ?? '';
  } catch {
    return '';
  }
}

async function writeAgentContent(path: string, content: string): Promise<boolean> {
  try {
    const res = await fetch('/api/files', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path, content }),
    });
    return res.ok;
  } catch {
    return false;
  }
}

const AgentDefsView: Component = () => {
  // Edit state
  const [editingAgent, setEditingAgent] = createSignal<string | null>(null);
  const [editName, setEditName] = createSignal('');
  const [editRole, setEditRole] = createSignal('coder');
  const [editModel, setEditModel] = createSignal('sonnet');
  const [editDesc, setEditDesc] = createSignal('');
  const [saving, setSaving] = createSignal(false);

  // Create form state
  const [showCreateForm, setShowCreateForm] = createSignal(false);
  const [newName, setNewName] = createSignal('');
  const [newRole, setNewRole] = createSignal('coder');
  const [newModel, setNewModel] = createSignal('sonnet');
  const [newDesc, setNewDesc] = createSignal('');
  const [creating, setCreating] = createSignal(false);

  // YAML content editor state
  const [editorAgentId, setEditorAgentId] = createSignal<string | null>(null);
  const [editorContent, setEditorContent] = createSignal('');
  const [editorPath, setEditorPath] = createSignal('');
  const [loadingContent, setLoadingContent] = createSignal(false);

  // Map SpacetimeDB rows to AgentDef objects
  const agentList = createMemo((): AgentDef[] => {
    return agentDefinitions().map((d: any) => {
      const role = d.role ?? '';
      let tools: string[] = [];
      try { tools = JSON.parse(d.toolsJson ?? d.tools_json ?? '[]'); } catch { /* empty */ }
      return {
        agentId: d.agentId ?? d.agent_id ?? '',
        name: d.name ?? '',
        role,
        model: d.model ?? '',
        desc: d.description ?? '',
        tools,
        path: d.path ?? '',
        color: ROLE_COLORS[role] || '#6b7280',
      };
    });
  });

  function handleEdit(agent: AgentDef) {
    setEditingAgent(agent.agentId);
    setEditName(agent.name);
    setEditRole(agent.role);
    setEditModel(agent.model);
    setEditDesc(agent.desc);
  }

  function handleCancelEdit() {
    setEditingAgent(null);
    setEditName('');
    setEditRole('coder');
    setEditModel('sonnet');
    setEditDesc('');
  }

  function handleSave(agent: AgentDef) {
    const conn = getHexfloConn();
    if (!conn) {
      addToast('error', 'SpacetimeDB not connected');
      return;
    }
    setSaving(true);
    try {
      conn.reducers.syncAgentDef(
        agent.agentId,
        'hex-intf',
        editName().trim() || agent.name,
        editRole() || agent.role,
        editModel() || agent.model,
        JSON.stringify(agent.tools),
        '[]',
        agent.path || `.claude/agents/${slugify(editName().trim() || agent.name)}.yml`,
        new Date().toISOString(),
      );
      addToast('success', `Updated agent "${editName().trim() || agent.name}"`);
      setEditingAgent(null);
    } catch (err: any) {
      addToast('error', `Failed to update: ${err.message}`);
    } finally {
      setSaving(false);
    }
  }

  function handleCreate() {
    const name = newName().trim().replace(/\s+/g, '-').toLowerCase();
    if (!name) { addToast('error', 'Name is required'); return; }
    const conn = getHexfloConn();
    if (!conn) {
      addToast('error', 'SpacetimeDB not connected');
      return;
    }

    setCreating(true);
    try {
      conn.reducers.syncAgentDef(
        slugify(name),
        'hex-intf',
        name,
        newRole(),
        newModel(),
        JSON.stringify(['Read', 'Write', 'Edit', 'Bash', 'Grep']),
        '[]',
        `.claude/agents/${slugify(name)}.yml`,
        new Date().toISOString(),
      );
      addToast('success', `Created agent "${name}"`);
      setShowCreateForm(false);
      setNewName(''); setNewDesc('');
    } catch (err: any) {
      addToast('error', `Failed to create: ${err.message}`);
    } finally {
      setCreating(false);
    }
  }

  function handleDelete(agent: AgentDef) {
    const conn = getHexfloConn();
    if (!conn) {
      addToast('error', 'SpacetimeDB not connected');
      return;
    }
    try {
      if (typeof conn.reducers.removeAgentDef === 'function') {
        conn.reducers.removeAgentDef(agent.agentId);
        addToast('success', `Removed agent "${agent.name}"`);
      } else if (typeof conn.reducers.deleteAgentDef === 'function') {
        conn.reducers.deleteAgentDef(agent.agentId);
        addToast('success', `Removed agent "${agent.name}"`);
      } else {
        // No delete reducer — mark as deleted via description
        conn.reducers.syncAgentDef(
          agent.agentId,
          'hex-intf',
          agent.name,
          agent.role,
          agent.model,
          JSON.stringify(agent.tools),
          '[]',
          agent.path,
          new Date().toISOString(),
        );
        addToast('info', `No delete reducer available. Use syncAgentDef to update instead.`);
      }
      if (editingAgent() === agent.agentId) setEditingAgent(null);
      if (editorAgentId() === agent.agentId) { setEditorAgentId(null); setEditorContent(''); setEditorPath(''); }
    } catch (err: any) {
      addToast('error', `Delete failed: ${err.message}`);
    }
  }

  /* ---------------------------------------------------------------- */
  /*  YAML content editor                                              */
  /* ---------------------------------------------------------------- */

  async function openContentEditor(agent: AgentDef) {
    if (editorAgentId() === agent.agentId) {
      // Toggle off
      setEditorAgentId(null);
      return;
    }
    const path = agent.path || `.claude/agents/${slugify(agent.name)}.yml`;
    setEditorAgentId(agent.agentId);
    setEditorPath(path);
    setLoadingContent(true);
    const content = await readAgentContent(path);
    setEditorContent(content);
    setLoadingContent(false);
  }

  const inputClass = "w-full rounded-lg border px-3 py-2 text-sm text-gray-200 placeholder-gray-600 focus:outline-none focus:border-cyan-600";
  const selectClass = "w-full rounded-lg border px-3 py-2 text-sm text-gray-200 focus:outline-none focus:border-cyan-600 appearance-none";
  const inputStyle = { "background-color": "var(--bg-input, var(--bg-elevated))", "border-color": "var(--border-subtle)" };

  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">Agent Definitions</h2>
          <p class="mt-1 text-sm text-gray-400">
            {agentList().length === 0
              ? 'No agents registered'
              : `${agentList().length} agent definitions`}
            <Show when={hexfloConnected()}>
              <span class="ml-2 inline-flex items-center rounded-full bg-cyan-900/30 px-2 py-0.5 text-[10px] font-medium text-cyan-400">SpacetimeDB</span>
            </Show>
          </p>
        </div>
        <div class="flex gap-2">
          <button class="rounded-lg border border-gray-700 px-4 py-2 text-sm text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
            onClick={() => setShowCreateForm(!showCreateForm())}>
            {showCreateForm() ? 'Cancel' : '+ Define Agent'}
          </button>
          <button class="rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-gray-100 transition-colors border border-gray-700"
            onClick={() => setSpawnDialogOpen(true)}>
            Spawn Agent
          </button>
        </div>
      </div>

      {/* Create Agent Form */}
      <Show when={showCreateForm()}>
        <div class="mb-6 rounded-xl border p-4 space-y-3" style={{ "background-color": "var(--bg-surface)", "border-color": "var(--border-subtle)" }}>
          <h3 class="text-sm font-bold text-gray-200">Define New Agent</h3>
          <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-4 gap-3">
            <div>
              <label class="block text-xs text-gray-500 mb-1">Name</label>
              <input type="text" placeholder="my-agent" value={newName()} onInput={(e) => setNewName(e.currentTarget.value)}
                class={inputClass} style={inputStyle} />
            </div>
            <div>
              <label class="block text-xs text-gray-500 mb-1">Role</label>
              <select value={newRole()} onChange={(e) => setNewRole(e.currentTarget.value)}
                class={selectClass} style={inputStyle}>
                <For each={ROLES}>{(r) => <option value={r}>{r}</option>}</For>
              </select>
            </div>
            <div>
              <label class="block text-xs text-gray-500 mb-1">Model</label>
              <select value={newModel()} onChange={(e) => setNewModel(e.currentTarget.value)}
                class={selectClass} style={inputStyle}>
                <For each={MODELS}>{(m) => <option value={m}>{m}</option>}</For>
              </select>
            </div>
            <div>
              <label class="block text-xs text-gray-500 mb-1">Description</label>
              <input type="text" placeholder="What this agent does" value={newDesc()} onInput={(e) => setNewDesc(e.currentTarget.value)}
                class={inputClass} style={inputStyle} />
            </div>
          </div>
          <div class="flex justify-end gap-2">
            <button class="rounded-lg border border-gray-700 px-3 py-1.5 text-xs text-gray-400 hover:text-gray-200 transition-colors"
              onClick={() => setShowCreateForm(false)}>Cancel</button>
            <button class="rounded-lg bg-cyan-700 px-4 py-1.5 text-xs font-medium text-white hover:bg-cyan-600 transition-colors disabled:opacity-50"
              disabled={creating() || !newName().trim()} onClick={handleCreate}>
              {creating() ? 'Creating...' : 'Create Agent'}
            </button>
          </div>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={agentList().length === 0}>
        <div class="rounded-xl border border-dashed border-gray-700 p-8 text-center">
          <p class="text-sm text-gray-500 mb-2">No agents registered.</p>
          <p class="text-xs text-gray-600">
            Run <code class="font-mono text-gray-500">hex nexus start</code> to sync from repo catalog,
            or define an agent above.
          </p>
        </div>
      </Show>

      {/* Agent cards grid */}
      <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
        <For each={agentList()}>
          {(agent) => (
            <div
              class="rounded-xl p-4 border"
              style={{
                "background-color": "var(--bg-surface)",
                "border-color": agent.color + "40",
              }}
            >
              {/* Name + colored dot */}
              <div class="flex items-center gap-2 mb-3">
                <span
                  class="h-2.5 w-2.5 rounded-full shrink-0"
                  style={{ "background-color": agent.color }}
                />
                <span class="font-bold font-mono text-sm text-gray-100">{agent.name}</span>
              </div>

              {/* Role + model badges */}
              <div class="flex items-center gap-2 mb-3">
                <span
                  class="inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium"
                  style={{
                    "background-color": agent.color + "18",
                    "border-color": agent.color + "40",
                    color: agent.color,
                  }}
                >
                  {agent.role}
                </span>
                <span
                  class={`inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium ${modelBadgeColor[agent.model] ?? "bg-gray-800 text-gray-400 border-gray-700"}`}
                >
                  {agent.model}
                </span>
              </div>

              {/* Description */}
              <p class="text-sm text-gray-400 mb-4">{agent.desc}</p>

              {/* Tool chips */}
              <div class="flex flex-wrap gap-1.5 mb-4">
                <For each={agent.tools}>
                  {(tool) => (
                    <span class="rounded-full bg-gray-800 border border-gray-700 px-2.5 py-0.5 text-xs font-mono text-gray-400">
                      {tool}
                    </span>
                  )}
                </For>
              </div>

              {/* Action buttons */}
              <div class="flex gap-2">
                <button class="rounded-lg bg-gray-800 px-3 py-1.5 text-xs font-medium text-cyan-400 hover:bg-cyan-900/30 hover:text-cyan-300 transition-colors border border-gray-700"
                  onClick={() => openContentEditor(agent)}>
                  {editorAgentId() === agent.agentId ? 'Close YAML' : 'View/Edit YAML'}
                </button>
                <button class="rounded-lg bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors border border-gray-700"
                  onClick={() => editingAgent() === agent.agentId ? handleCancelEdit() : handleEdit(agent)}>
                  {editingAgent() === agent.agentId ? 'Close' : 'Edit Meta'}
                </button>
                <button class="rounded-lg bg-gray-800 px-3 py-1.5 text-xs font-medium text-red-400 hover:bg-red-900/30 hover:text-red-300 transition-colors border border-gray-700"
                  onClick={() => handleDelete(agent)}>
                  Delete
                </button>
              </div>

              {/* YAML Content Editor */}
              <Show when={editorAgentId() === agent.agentId}>
                <div class="mt-3 pt-3 border-t" style={{ "border-color": "var(--border-subtle)" }}>
                  <Show when={loadingContent()}>
                    <div class="flex items-center justify-center py-8">
                      <span class="text-sm text-gray-500">Loading YAML...</span>
                    </div>
                  </Show>
                  <Show when={!loadingContent()}>
                    <CodeEditor
                      content={editorContent()}
                      filePath={editorPath()}
                      title={agent.name}
                      language="yaml"
                      editable={true}
                      onSave={async (newContent) => {
                        const ok = await writeAgentContent(editorPath(), newContent);
                        if (ok) {
                          setEditorContent(newContent);
                          addToast('success', `Saved ${editorPath()}`);
                        } else {
                          addToast('error', `Failed to save ${editorPath()}`);
                        }
                      }}
                      onCancel={() => { setEditorAgentId(null); setEditorContent(''); setEditorPath(''); }}
                      minHeight="300px"
                    />
                  </Show>
                </div>
              </Show>

              {/* Inline metadata editor */}
              <Show when={editingAgent() === agent.agentId}>
                <div class="mt-3 pt-3 border-t space-y-3" style={{ "border-color": "var(--border-subtle)" }}>
                  <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
                    <div>
                      <label class="block text-xs text-gray-500 mb-1">Name</label>
                      <input type="text" value={editName()} onInput={(e) => setEditName(e.currentTarget.value)}
                        class={inputClass} style={inputStyle} />
                    </div>
                    <div>
                      <label class="block text-xs text-gray-500 mb-1">Role</label>
                      <select value={editRole()} onChange={(e) => setEditRole(e.currentTarget.value)}
                        class={selectClass} style={inputStyle}>
                        <For each={ROLES}>{(r) => <option value={r}>{r}</option>}</For>
                      </select>
                    </div>
                    <div>
                      <label class="block text-xs text-gray-500 mb-1">Model</label>
                      <select value={editModel()} onChange={(e) => setEditModel(e.currentTarget.value)}
                        class={selectClass} style={inputStyle}>
                        <For each={MODELS}>{(m) => <option value={m}>{m}</option>}</For>
                      </select>
                    </div>
                    <div>
                      <label class="block text-xs text-gray-500 mb-1">Description</label>
                      <input type="text" value={editDesc()} onInput={(e) => setEditDesc(e.currentTarget.value)}
                        class={inputClass} style={inputStyle} />
                    </div>
                  </div>
                  <div class="flex justify-end gap-2">
                    <button class="rounded-lg border border-gray-700 px-3 py-1.5 text-xs text-gray-400 hover:text-gray-200 transition-colors"
                      onClick={handleCancelEdit}>Cancel</button>
                    <button class="rounded-lg bg-cyan-700 px-4 py-1.5 text-xs font-medium text-white hover:bg-cyan-600 transition-colors disabled:opacity-50"
                      disabled={saving()} onClick={() => handleSave(agent)}>
                      {saving() ? 'Saving...' : 'Save'}
                    </button>
                  </div>
                </div>
              </Show>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default AgentDefsView;
