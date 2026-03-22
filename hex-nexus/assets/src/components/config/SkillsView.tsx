import { Component, For, Show, createMemo, createSignal } from 'solid-js';
import { addToast } from '../../stores/toast';
import { skillRegistry, getHexfloConn, hexfloConnected } from '../../stores/connection';
import { route } from '../../stores/router';
import { projects } from '../../stores/projects';
import { MarkdownEditor } from '../editor';
import { restClient } from '../../services/rest-client';

interface Skill {
  skillId: string;
  name: string;
  trigger: string;
  desc: string;
  path: string;
  syncedAt: string;
}

type TabId = 'global' | 'project';

function slugify(s: string): string {
  return s.trim().replace(/\s+/g, '-').toLowerCase();
}

function isGlobalSkill(path: string): boolean {
  return path.startsWith('skills/') && !path.startsWith('.claude/');
}

async function readSkillContent(path: string): Promise<string> {
  try {
    const data = await restClient.get(`/api/files?path=${encodeURIComponent(path)}`);
    return data.content ?? '';
  } catch { return ''; }
}

async function writeSkillContent(path: string, content: string): Promise<boolean> {
  try {
    await restClient.post('/api/files', { path, content });
    return true;
  } catch { return false; }
}

async function deleteSkillFile(path: string): Promise<boolean> {
  try {
    await restClient.delete(`/api/files?path=${encodeURIComponent(path)}`);
    return true;
  } catch { return false; }
}

const SkillsView: Component = () => {
  const projectId = createMemo(() => (route() as any).projectId ?? '');
  const project = createMemo(() => projects().find(p => p.id === projectId()));

  const [activeTab, setActiveTab] = createSignal<TabId>('project');
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [editorContent, setEditorContent] = createSignal('');
  const [editorPath, setEditorPath] = createSignal('');
  const [loadingContent, setLoadingContent] = createSignal(false);
  const [showCreateForm, setShowCreateForm] = createSignal(false);
  const [newName, setNewName] = createSignal('');
  const [newDesc, setNewDesc] = createSignal('');
  const [newTrigger, setNewTrigger] = createSignal('');
  const [creating, setCreating] = createSignal(false);
  const [actionLoading, setActionLoading] = createSignal<string | null>(null);
  const [confirmDelete, setConfirmDelete] = createSignal<string | null>(null);

  const allSkills = createMemo((): Skill[] =>
    skillRegistry()
      .map((s: any) => ({
        skillId: s.skillId ?? s.skill_id ?? '',
        name: s.name ?? '',
        trigger: s.triggerCmd ?? s.trigger_cmd ?? s.trigger ?? '',
        desc: s.description ?? '',
        path: s.path ?? s.sourcePath ?? s.source_path ?? '',
        syncedAt: s.syncedAt ?? s.synced_at ?? s.updatedAt ?? s.updated_at ?? '',
      }))
      .filter(s => s.desc !== '[DELETED]' && s.name.trim() !== '' && s.skillId.trim() !== '')
  );

  const globalSkills = createMemo(() => allSkills().filter(s => isGlobalSkill(s.path)));
  const projectSkills = createMemo(() => allSkills().filter(s => !isGlobalSkill(s.path)));
  const visibleSkills = createMemo(() =>
    activeTab() === 'global' ? globalSkills() : projectSkills()
  );

  const selectedSkill = createMemo(() =>
    visibleSkills().find(s => s.skillId === selectedId())
  );

  async function handleSelect(skill: Skill) {
    if (selectedId() === skill.skillId) {
      setSelectedId(null);
      return;
    }
    setSelectedId(skill.skillId);
    setEditorPath(skill.path);
    setLoadingContent(true);
    const content = await readSkillContent(skill.path);
    setEditorContent(content);
    setLoadingContent(false);
  }

  async function handleCopy(skill: Skill) {
    setActionLoading(skill.skillId);
    const isGlobal = isGlobalSkill(skill.path);
    const destPath = isGlobal
      ? `.claude/skills/${slugify(skill.name)}/SKILL.md`
      : `skills/${slugify(skill.name)}/SKILL.md`;
    const content = await readSkillContent(skill.path);
    if (!content) {
      addToast('error', `Could not read ${skill.path}`);
      setActionLoading(null);
      return;
    }
    const ok = await writeSkillContent(destPath, content);
    if (!ok) {
      addToast('error', `Failed to write ${destPath}`);
      setActionLoading(null);
      return;
    }
    const conn = getHexfloConn();
    if (conn) {
      try {
        conn.reducers.syncSkill(
          `${slugify(skill.name)}-${isGlobal ? 'project' : 'global'}`,
          projectId() || 'hex-intf',
          skill.name, skill.trigger, skill.desc,
          destPath, new Date().toISOString(),
        );
      } catch { /* best effort */ }
    }
    addToast('success', `Copied "${skill.name}" to ${isGlobal ? 'project' : 'global'}`);
    setActionLoading(null);
  }

  function handleDelete(skill: Skill) {
    if (confirmDelete() !== skill.skillId) {
      setConfirmDelete(skill.skillId);
      return;
    }
    setConfirmDelete(null);
    setActionLoading(skill.skillId);

    // Remove file
    deleteSkillFile(skill.path).then((ok) => {
      if (!ok) addToast('info', `File removal may require manual cleanup: ${skill.path}`);
    });

    // Remove from SpacetimeDB — mark as deleted if no delete reducer
    const conn = getHexfloConn();
    if (conn) {
      try {
        if (typeof (conn.reducers as any).removeSkill === 'function') {
          (conn.reducers as any).removeSkill(skill.skillId);
        } else {
          conn.reducers.syncSkill(
            skill.skillId, projectId() || 'hex-intf',
            skill.name, skill.trigger, '[DELETED]',
            skill.path, new Date().toISOString(),
          );
        }
      } catch { /* ignore */ }
    }
    addToast('success', `Deleted "${skill.name}"`);
    if (selectedId() === skill.skillId) setSelectedId(null);
    setActionLoading(null);
  }

  function handleCreate() {
    const name = slugify(newName());
    if (!name) { addToast('error', 'Name is required'); return; }
    const conn = getHexfloConn();
    if (!conn) { addToast('error', 'SpacetimeDB not connected'); return; }
    const trigger = newTrigger().trim() || `/${name}`;
    const desc = newDesc().trim() || 'A custom skill';
    const isGlobal = activeTab() === 'global';
    const path = isGlobal ? `skills/${name}/SKILL.md` : `.claude/skills/${name}/SKILL.md`;
    const template = `---\nname: ${name}\ndescription: ${desc}\ntrigger: ${trigger}\n---\n\n# ${name}\n\n${desc}\n\n## Instructions\n\n[Add skill instructions here]\n`;

    setCreating(true);
    writeSkillContent(path, template).then((ok) => {
      if (ok) {
        conn.reducers.syncSkill(name, projectId() || 'hex-intf', name, trigger, desc, path, new Date().toISOString());
        addToast('success', `Created "${name}"`);
        setShowCreateForm(false);
        setNewName(''); setNewDesc(''); setNewTrigger('');
      } else {
        addToast('error', 'Failed to create skill file');
      }
      setCreating(false);
    });
  }

  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-4">
        <div>
          <h2 class="text-lg font-bold text-[var(--text-primary)]">Skills</h2>
          <p class="mt-0.5 text-sm text-[var(--text-muted)]">
            {globalSkills().length} global, {projectSkills().length} project
            <Show when={hexfloConnected()}>
              <span class="ml-2 inline-flex items-center rounded-full bg-[var(--accent-dim)] px-2 py-0.5 text-[10px] font-medium text-[var(--accent)]">SpacetimeDB</span>
            </Show>
          </p>
        </div>
        <button
          class="rounded-lg border border-[var(--border)] px-3 py-1.5 text-sm text-[var(--accent)] transition-colors"
          onClick={() => setShowCreateForm(!showCreateForm())}
        >
          {showCreateForm() ? 'Cancel' : '+ New Skill'}
        </button>
      </div>

      {/* Tabs */}
      <div class="flex border-b border-[var(--border-subtle)] mb-4">
        <button
          class="px-4 py-2 text-sm font-medium border-b-2 transition-colors"
          classList={{
            "text-[var(--accent-hover)] border-[var(--accent)]": activeTab() === 'global',
            "text-[var(--text-faint)] border-transparent": activeTab() !== 'global',
          }}
          onClick={() => { setActiveTab('global'); setSelectedId(null); }}
        >
          Global Catalog ({globalSkills().length})
        </button>
        <button
          class="px-4 py-2 text-sm font-medium border-b-2 transition-colors"
          classList={{
            "text-[var(--accent-hover)] border-[var(--accent)]": activeTab() === 'project',
            "text-[var(--text-faint)] border-transparent": activeTab() !== 'project',
          }}
          onClick={() => { setActiveTab('project'); setSelectedId(null); }}
        >
          Project Skills ({projectSkills().length})
        </button>
      </div>

      {/* Create form */}
      <Show when={showCreateForm()}>
        <div class="mb-4 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] p-4 space-y-3">
          <div class="grid grid-cols-3 gap-3">
            <div>
              <label class="block text-xs text-[var(--text-faint)] mb-1">Name</label>
              <input type="text" placeholder="my-skill" value={newName()} onInput={(e) => setNewName(e.currentTarget.value)}
                class="w-full rounded border border-[var(--border-subtle)] bg-[var(--bg-input)] px-3 py-2 text-sm outline-none focus:border-cyan-600" />
            </div>
            <div>
              <label class="block text-xs text-[var(--text-faint)] mb-1">Trigger</label>
              <input type="text" placeholder="/my-skill" value={newTrigger()} onInput={(e) => setNewTrigger(e.currentTarget.value)}
                class="w-full rounded border border-[var(--border-subtle)] bg-[var(--bg-input)] px-3 py-2 text-sm outline-none focus:border-cyan-600" />
            </div>
            <div>
              <label class="block text-xs text-[var(--text-faint)] mb-1">Description</label>
              <input type="text" placeholder="What it does" value={newDesc()} onInput={(e) => setNewDesc(e.currentTarget.value)}
                class="w-full rounded border border-[var(--border-subtle)] bg-[var(--bg-input)] px-3 py-2 text-sm outline-none focus:border-cyan-600" />
            </div>
          </div>
          <div class="flex justify-end gap-2">
            <button class="rounded border border-[var(--border)] px-3 py-1.5 text-xs text-[var(--text-muted)] transition-colors" onClick={() => setShowCreateForm(false)}>Cancel</button>
            <button class="rounded bg-[var(--accent)] px-4 py-1.5 text-xs font-medium text-white transition-colors disabled:opacity-50"
              disabled={creating() || !newName().trim()} onClick={handleCreate}>{creating() ? 'Creating...' : 'Create'}</button>
          </div>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={visibleSkills().length === 0}>
        <div class="rounded-lg border border-dashed border-[var(--border)] p-8 text-center">
          <p class="text-sm text-[var(--text-faint)]">
            No {activeTab() === 'global' ? 'global' : 'project'} skills.{' '}
            {activeTab() === 'global'
              ? 'Run hex nexus start to sync from catalog.'
              : 'Copy from Global Catalog or create a new one.'}
          </p>
        </div>
      </Show>

      {/* Skill table — clean rows, actions only when selected */}
      <Show when={visibleSkills().length > 0}>
        <div class="rounded-lg border border-[var(--border-subtle)] overflow-hidden">
          {/* Table header */}
          <div
            class="grid gap-4 px-4 py-2 text-[11px] font-semibold uppercase text-[var(--text-dim)] bg-[var(--bg-elevated)] tracking-wide"
            style={{ "grid-template-columns": "24px 1fr 140px 2fr 100px" }}
          >
            <span />
            <span>Name</span>
            <span>Trigger</span>
            <span>Description</span>
            <span>Synced</span>
          </div>

          {/* Table rows */}
          <For each={visibleSkills()}>
            {(skill) => {
              const isSelected = () => selectedId() === skill.skillId;
              const isConfirmingDelete = () => confirmDelete() === skill.skillId;

              return (
                <div class="border-t border-[var(--border-subtle)]">
                  {/* Row */}
                  <button
                    class="grid w-full gap-4 px-4 py-2.5 text-left transition-colors"
                    classList={{
                      "bg-[var(--accent-dim)]": isSelected(),
                      "bg-[var(--bg-surface)]": !isSelected(),
                    }}
                    style={{ "grid-template-columns": "24px 1fr 140px 2fr 100px" }}
                    onClick={() => handleSelect(skill)}
                  >
                    <span class="flex items-center justify-center">
                      <span
                        class="h-2 w-2 rounded-full"
                        classList={{
                          "bg-blue-400": isGlobalSkill(skill.path),
                          "bg-green-400": !isGlobalSkill(skill.path),
                        }}
                      />
                    </span>
                    <span class="text-sm font-medium truncate text-[var(--text-primary)]">
                      {skill.name}
                    </span>
                    <span class="font-mono text-sm truncate text-[var(--accent-hover)]">
                      {skill.trigger}
                    </span>
                    <span class="text-sm truncate text-[var(--text-muted)]">
                      {skill.desc}
                    </span>
                    <span class="text-[11px] truncate text-gray-500 font-mono" title={skill.syncedAt}>
                      {skill.syncedAt ? new Date(skill.syncedAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) : '\u2014'}
                    </span>
                  </button>

                  {/* Expanded: action bar + editor */}
                  <Show when={isSelected()}>
                    {/* Action bar */}
                    <div
                      class="flex items-center gap-2 border-t border-[var(--border-subtle)] bg-[var(--bg-elevated)] px-4 py-2"
                    >
                      <span class="text-[11px] font-medium text-[var(--text-faint)] font-mono" title={`Source: ${skill.path}`}>
                        {skill.path}
                      </span>
                      <Show when={skill.syncedAt}>
                        <span class="text-[10px] text-gray-600" title={skill.syncedAt}>
                          synced {new Date(skill.syncedAt).toLocaleString()}
                        </span>
                      </Show>
                      <button
                        class="rounded px-2 py-0.5 text-[11px] font-medium text-cyan-400 border border-gray-700 hover:border-cyan-600 hover:bg-cyan-900/20 transition-colors"
                        onClick={async (e) => {
                          e.stopPropagation();
                          try {
                            await restClient.post('/api/config/sync');
                            addToast('success', `Re-synced skill "${skill.name}"`);
                          } catch {
                            addToast('error', 'Resync failed');
                          }
                        }}
                      >
                        Resync
                      </button>
                      <div class="flex-1" />

                      {/* Copy */}
                      <button
                        class="rounded px-2.5 py-1 text-[11px] font-medium transition-colors disabled:opacity-50 border border-[var(--border)]"
                        classList={{
                          "text-green-400": activeTab() === 'global',
                          "text-blue-400": activeTab() !== 'global',
                        }}
                        disabled={actionLoading() === skill.skillId}
                        onClick={(e) => { e.stopPropagation(); handleCopy(skill); }}
                      >
                        {actionLoading() === skill.skillId
                          ? 'Copying...'
                          : activeTab() === 'global' ? 'Copy to Project' : 'Copy to Global'}
                      </button>

                      {/* Delete (project only) */}
                      <Show when={activeTab() === 'project'}>
                        <button
                          class="rounded px-2.5 py-1 text-[11px] font-medium transition-colors border border-[var(--border)]"
                          classList={{
                            "text-white bg-red-900": isConfirmingDelete(),
                            "text-red-400 bg-transparent": !isConfirmingDelete(),
                          }}
                          onClick={(e) => { e.stopPropagation(); handleDelete(skill); }}
                        >
                          {isConfirmingDelete() ? 'Confirm Delete' : 'Delete'}
                        </button>
                      </Show>
                    </div>

                    {/* Content editor */}
                    <Show when={loadingContent()}>
                      <div class="flex items-center justify-center bg-[var(--bg-surface)] py-8">
                        <span class="text-sm text-[var(--text-faint)]">Loading...</span>
                      </div>
                    </Show>
                    <Show when={!loadingContent()}>
                      <div class="h-[400px]">
                        <MarkdownEditor
                          content={editorContent()}
                          filePath={skill.path}
                          title={skill.name}
                          initialMode="view"
                          editable={true}
                          onSave={async (newContent) => {
                            const ok = await writeSkillContent(editorPath(), newContent);
                            if (ok) {
                              setEditorContent(newContent);
                              addToast('success', `Saved ${editorPath()}`);
                            } else {
                              addToast('error', `Failed to save ${editorPath()}`);
                            }
                          }}
                          metadata={[
                            { label: "Trigger", value: skill.trigger, color: "#22d3ee" },
                            { label: "Scope", value: isGlobalSkill(skill.path) ? "Global" : "Project", color: isGlobalSkill(skill.path) ? "#60a5fa" : "#4ade80" },
                          ]}
                        />
                      </div>
                    </Show>
                  </Show>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default SkillsView;
