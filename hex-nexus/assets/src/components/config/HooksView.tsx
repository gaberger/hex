import { Component, For, Show, createMemo, createSignal } from 'solid-js';
import { addToast } from '../../stores/toast';
import { projectConfigs, hexfloConnected, getHexfloConn } from '../../stores/connection';

interface Hook {
  name: string;
  cmd: string;
  enabled: boolean;
  matcher?: string;
  timeout?: number;
}

interface HookType {
  event: string;
  desc: string;
  hooks: Hook[];
}

const HOOK_EVENTS = [
  'PreToolUse',
  'PostToolUse',
  'UserPromptSubmit',
  'SessionStart',
  'Notification',
  'Stop',
] as const;

const EVENT_DESCRIPTIONS: Record<string, string> = {
  PreToolUse: "Runs before a tool is called",
  PostToolUse: "Runs after a tool completes",
  UserPromptSubmit: "Runs when user sends a message",
  SessionStart: "Runs at session initialization",
  Notification: "Runs when a notification is sent",
  Stop: "Runs when the agent stops",
};

/* ------------------------------------------------------------------ */
/*  SpacetimeDB helpers                                                */
/* ------------------------------------------------------------------ */

function getHooksMap(): Record<string, any[]> {
  const configs = projectConfigs();
  const hookConfig = configs.find((c: any) => (c.key ?? c.configKey) === 'hooks');
  if (!hookConfig) return {};
  try {
    return JSON.parse(hookConfig.valueJson ?? hookConfig.value_json ?? '{}');
  } catch {
    return {};
  }
}

function saveHooksMap(map: Record<string, any[]>) {
  const conn = getHexfloConn();
  if (!conn) {
    addToast('error', 'SpacetimeDB not connected.');
    return false;
  }
  conn.reducers.syncConfig(
    'hooks',
    'hex-intf',
    JSON.stringify(map),
    '.claude/settings.json',
    new Date().toISOString(),
  );
  return true;
}

function parseHookTypes(raw: Record<string, any[]>): HookType[] {
  return Object.entries(raw).map(([event, items]: [string, any]) => ({
    event,
    desc: EVENT_DESCRIPTIONS[event] || event,
    hooks: (Array.isArray(items) ? items : []).map((h: any, i: number) => ({
      name: h.matcher || (typeof h.command === 'string' ? h.command.split('/').pop() : null) || `hook-${i}`,
      cmd: typeof h.command === 'string' ? h.command : (Array.isArray(h.command) ? h.command.join(' ') : ''),
      enabled: h.enabled !== false,
      matcher: h.matcher || '',
      timeout: h.timeout ?? 10000,
    })),
  }));
}

/* ------------------------------------------------------------------ */
/*  Inline form for adding / editing a hook                           */
/* ------------------------------------------------------------------ */

interface HookFormProps {
  eventDefault?: string;
  initial?: { event: string; matcher: string; command: string; timeout: number };
  onSave: (event: string, matcher: string, command: string, timeout: number) => void;
  onCancel: () => void;
  submitLabel: string;
  eventDisabled?: boolean;
}

const HookForm: Component<HookFormProps> = (props) => {
  const [event, setEvent] = createSignal(props.initial?.event ?? props.eventDefault ?? 'PreToolUse');
  const [matcher, setMatcher] = createSignal(props.initial?.matcher ?? '');
  const [command, setCommand] = createSignal(props.initial?.command ?? '');
  const [timeout, setTimeout] = createSignal(props.initial?.timeout ?? 10000);

  const handleSubmit = (e: Event) => {
    e.preventDefault();
    const c = command().trim();
    if (!c) {
      addToast('error', 'Command is required.');
      return;
    }
    props.onSave(event(), matcher().trim(), c, timeout());
  };

  return (
    <form onSubmit={handleSubmit} class="rounded-lg border p-4 space-y-3 mt-2" style={{ "background-color": "var(--bg-elevated)", "border-color": "var(--border-subtle)" }}>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <div>
          <label class="block text-xs font-medium text-gray-400 mb-1" style={{ "font-size": "13px" }}>Event</label>
          <select
            value={event()}
            onChange={(e) => setEvent(e.currentTarget.value)}
            disabled={props.eventDisabled}
            class="w-full rounded-md border px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-[var(--accent)]"
            style={{ "background-color": "var(--bg-input)", "border-color": "var(--border-subtle)", "font-size": "14px" }}
          >
            <For each={[...HOOK_EVENTS]}>
              {(ev) => <option value={ev}>{ev}</option>}
            </For>
          </select>
        </div>
        <div>
          <label class="block text-xs font-medium text-gray-400 mb-1" style={{ "font-size": "13px" }}>Matcher (optional)</label>
          <input
            type="text"
            value={matcher()}
            onInput={(e) => setMatcher(e.currentTarget.value)}
            placeholder="e.g. Bash"
            class="w-full rounded-md border px-3 py-1.5 text-sm text-gray-200 placeholder-gray-600 outline-none focus:border-[var(--accent)]"
            style={{ "background-color": "var(--bg-input)", "border-color": "var(--border-subtle)", "font-size": "14px" }}
          />
        </div>
        <div>
          <label class="block text-xs font-medium text-gray-400 mb-1" style={{ "font-size": "13px" }}>Command</label>
          <input
            type="text"
            value={command()}
            onInput={(e) => setCommand(e.currentTarget.value)}
            placeholder="node ~/.hex/hooks/my-hook.js"
            class="w-full rounded-md border px-3 py-1.5 text-sm text-gray-200 placeholder-gray-600 outline-none focus:border-[var(--accent)]"
            style={{ "background-color": "var(--bg-input)", "border-color": "var(--border-subtle)", "font-size": "14px" }}
          />
        </div>
        <div>
          <label class="block text-xs font-medium text-gray-400 mb-1" style={{ "font-size": "13px" }}>Timeout (ms)</label>
          <input
            type="number"
            value={timeout()}
            onInput={(e) => setTimeout(parseInt(e.currentTarget.value) || 10000)}
            class="w-full rounded-md border px-3 py-1.5 text-sm text-gray-200 outline-none focus:border-[var(--accent)]"
            style={{ "background-color": "var(--bg-input)", "border-color": "var(--border-subtle)", "font-size": "14px" }}
          />
        </div>
      </div>
      <div class="flex items-center gap-2 justify-end">
        <button type="button" onClick={props.onCancel}
          class="rounded-md px-3 py-1.5 text-xs font-medium text-gray-400 hover:text-gray-200 transition-colors">
          Cancel
        </button>
        <button type="submit"
          class="rounded-md px-4 py-1.5 text-xs font-medium text-white transition-colors"
          style={{ "background-color": "var(--accent)" }}>
          {props.submitLabel}
        </button>
      </div>
    </form>
  );
};

/* ------------------------------------------------------------------ */
/*  Main view                                                         */
/* ------------------------------------------------------------------ */

const HooksView: Component = () => {
  const [addingForEvent, setAddingForEvent] = createSignal<string | null>(null);
  const [editingHook, setEditingHook] = createSignal<string | null>(null); // "event:index"
  const [confirmRemove, setConfirmRemove] = createSignal<string | null>(null); // "event:index"

  const hookTypes = createMemo((): HookType[] => {
    const raw = getHooksMap();
    if (Object.keys(raw).length === 0) return [];
    return parseHookTypes(raw);
  });

  /* -- Mutations --------------------------------------------------- */

  const hookKey = (event: string, index: number) => `${event}:${index}`;

  const handleAddHook = (event: string, matcher: string, command: string, timeout: number) => {
    const map = { ...getHooksMap() };
    if (!Array.isArray(map[event])) map[event] = [];
    const entry: any = { command, timeout };
    if (matcher) entry.matcher = matcher;
    map[event] = [...map[event], entry];
    if (saveHooksMap(map)) {
      addToast('success', `Hook added to ${event}.`);
      setAddingForEvent(null);
    }
  };

  const handleEditHook = (event: string, index: number, matcher: string, command: string, timeout: number) => {
    const map = { ...getHooksMap() };
    if (!map[event]?.[index]) return;
    const entry: any = { command, timeout };
    if (matcher) entry.matcher = matcher;
    // Preserve enabled state
    if (map[event][index].enabled === false) {
      entry.enabled = false;
    }
    const updated = [...map[event]];
    updated[index] = entry;
    map[event] = updated;
    if (saveHooksMap(map)) {
      addToast('success', 'Hook updated.');
      setEditingHook(null);
    }
  };

  const handleRemoveHook = (event: string, index: number) => {
    const map = { ...getHooksMap() };
    if (!map[event]) return;
    const updated = [...map[event]];
    updated.splice(index, 1);
    if (updated.length === 0) {
      delete map[event];
    } else {
      map[event] = updated;
    }
    if (saveHooksMap(map)) {
      addToast('success', 'Hook removed.');
      setConfirmRemove(null);
    }
  };

  const handleToggleHook = (event: string, index: number, currentlyEnabled: boolean) => {
    const map = { ...getHooksMap() };
    if (!map[event]?.[index]) return;
    const updated = [...map[event]];
    updated[index] = { ...updated[index] };
    if (currentlyEnabled) {
      updated[index].enabled = false;
    } else {
      delete updated[index].enabled;
    }
    map[event] = updated;
    if (saveHooksMap(map)) {
      addToast('success', `Hook ${currentlyEnabled ? 'disabled' : 'enabled'}.`);
    }
  };

  return (
    <div class="flex-1 overflow-auto p-6" style={{ "background-color": "var(--bg-base)" }}>
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">Hooks</h2>
          <p class="mt-1 text-sm text-gray-400">
            {hexfloConnected()
              ? 'Claude Code hooks that run at specific lifecycle events.'
              : 'Connecting to SpacetimeDB...'}
            <span class="ml-2 inline-flex items-center rounded-full bg-cyan-900/30 px-2 py-0.5 text-[10px] font-medium text-cyan-400">SpacetimeDB</span>
          </p>
        </div>
      </div>

      {/* Empty state */}
      <Show when={hookTypes().length === 0}>
        <div class="rounded-lg border px-6 py-10 text-center" style={{ "background-color": "var(--bg-surface)", "border-color": "var(--border-subtle)" }}>
          <p class="text-sm text-gray-400">No hooks configured.</p>
          <p class="text-xs text-gray-600 mt-1">Run <code class="text-cyan-400">hex nexus start</code> to sync from repo.</p>
        </div>
      </Show>

      {/* Hook type sections */}
      <div class="space-y-6">
        <For each={hookTypes()}>
          {(hookType) => {
            const isAdding = () => addingForEvent() === hookType.event;

            return (
              <div>
                {/* Section header */}
                <div class="flex items-center justify-between mb-3">
                  <div>
                    <h3
                      class="font-bold text-gray-200"
                      style={{ "font-size": "14px" }}
                    >
                      {hookType.event}
                    </h3>
                    <p class="text-xs text-gray-500 mt-0.5">{hookType.desc}</p>
                  </div>
                  <button class="rounded-md bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors border border-gray-700"
                    onClick={() => setAddingForEvent(isAdding() ? null : hookType.event)}>
                    {isAdding() ? 'Cancel' : '+ Add Hook'}
                  </button>
                </div>

                {/* Add hook form */}
                <Show when={isAdding()}>
                  <HookForm
                    eventDefault={hookType.event}
                    eventDisabled={true}
                    onSave={(ev, matcher, cmd, timeout) => handleAddHook(ev, matcher, cmd, timeout)}
                    onCancel={() => setAddingForEvent(null)}
                    submitLabel="Add Hook"
                  />
                  <div class="mb-3" />
                </Show>

                {/* Hook rows */}
                <div class="space-y-2">
                  <For each={hookType.hooks} fallback={
                    <div
                      class="rounded-lg px-4 py-3 text-xs text-gray-600 border"
                      style={{ "background-color": "var(--bg-surface)", "border-color": "var(--border-subtle)" }}
                    >
                      No hooks configured.
                    </div>
                  }>
                    {(hook, idx) => {
                      const key = () => hookKey(hookType.event, idx());
                      const isEditingThis = () => editingHook() === key();
                      const isConfirmingThis = () => confirmRemove() === key();

                      return (
                        <div>
                          <div
                            class="flex items-center gap-3 rounded-lg px-4 py-3 border"
                            style={{ "background-color": "var(--bg-surface)", "border-color": "var(--border-subtle)" }}
                          >
                            {/* Enable/disable toggle */}
                            <span
                              class="h-2.5 w-2.5 shrink-0 rounded-full cursor-pointer transition-colors"
                              classList={{
                                "bg-green-500": hook.enabled,
                                "bg-gray-600": !hook.enabled,
                              }}
                              title={hook.enabled ? "Click to disable" : "Click to enable"}
                              onClick={() => handleToggleHook(hookType.event, idx(), hook.enabled)}
                            />
                            {/* Hook name */}
                            <span class="text-sm font-bold text-gray-200 min-w-[160px]">
                              {hook.name}
                            </span>
                            {/* Command */}
                            <span
                              class="text-xs text-gray-500 truncate flex-1"
                              style={{ "font-family": "'JetBrains Mono', monospace" }}
                            >
                              {hook.cmd}
                            </span>

                            {/* Action buttons */}
                            <Show when={!isEditingThis() && !isConfirmingThis()}>
                              <button
                                class="rounded-md px-2.5 py-1 text-xs text-gray-500 hover:text-gray-200 hover:bg-gray-700/50 transition-colors shrink-0"
                                onClick={() => { setEditingHook(key()); setConfirmRemove(null); }}
                              >
                                Edit
                              </button>
                              <button
                                class="rounded-md px-2.5 py-1 text-xs text-gray-500 hover:text-red-400 hover:bg-red-900/20 transition-colors shrink-0"
                                onClick={() => { setConfirmRemove(key()); setEditingHook(null); }}
                              >
                                Remove
                              </button>
                            </Show>
                          </div>

                          {/* Confirm remove */}
                          <Show when={isConfirmingThis()}>
                            <div class="flex items-center gap-3 rounded-md border px-3 py-2 mt-2" style={{ "background-color": "var(--bg-elevated)", "border-color": "var(--border-subtle)" }}>
                              <span class="text-xs text-gray-300">Remove this hook?</span>
                              <button
                                class="rounded-md px-3 py-1 text-xs font-medium text-white bg-red-600 hover:bg-red-500 transition-colors"
                                onClick={() => handleRemoveHook(hookType.event, idx())}
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
                          <Show when={isEditingThis()}>
                            <HookForm
                              initial={{
                                event: hookType.event,
                                matcher: hook.matcher || hook.name,
                                command: hook.cmd,
                                timeout: hook.timeout ?? 10000,
                              }}
                              eventDisabled={true}
                              onSave={(_ev, matcher, cmd, timeout) => handleEditHook(hookType.event, idx(), matcher, cmd, timeout)}
                              onCancel={() => setEditingHook(null)}
                              submitLabel="Save Changes"
                            />
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
};

export default HooksView;
