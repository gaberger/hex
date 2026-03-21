import { Component, For, createResource } from 'solid-js';
import { addToast } from '../../stores/toast';

interface Hook {
  name: string;
  cmd: string;
  enabled: boolean;
}

interface HookType {
  event: string;
  desc: string;
  hooks: Hook[];
}

const EVENT_DESCRIPTIONS: Record<string, string> = {
  PreToolUse: "Runs before a tool is called",
  PostToolUse: "Runs after a tool completes",
  UserPromptSubmit: "Runs when user sends a message",
  SessionStart: "Runs at session initialization",
  Notification: "Runs when a notification is sent",
  Stop: "Runs when the agent stops",
};

const HARDCODED_HOOKS: HookType[] = [
  { event: "PreToolUse", desc: "Runs before a tool is called", hooks: [
    { name: "context-mode router", cmd: "node ~/.context-mode/hook.js", enabled: true },
    { name: "intelligence patterns", cmd: "node ~/.hex/hooks/intelligence.js", enabled: true },
  ]},
  { event: "PostToolUse", desc: "Runs after a tool completes", hooks: [] },
  { event: "UserPromptSubmit", desc: "Runs when user sends a message", hooks: [
    { name: "intelligence router", cmd: "node ~/.hex/hooks/prompt-submit.js", enabled: true },
  ]},
  { event: "SessionStart", desc: "Runs at session initialization", hooks: [
    { name: "auto-memory import", cmd: "node ~/.hex/hooks/session-start.js", enabled: true },
    { name: "clear hook", cmd: "node ~/.hex/hooks/clear.js", enabled: true },
  ]},
];

async function discoverHooks(): Promise<HookType[]> {
  const allHooks: Record<string, any[]> = {};

  for (const file of ['.claude/settings.json', '.claude/settings.local.json']) {
    try {
      const res = await fetch(`/api/files?path=${encodeURIComponent(file)}`);
      if (res.ok) {
        const data = await res.json();
        const parsed = JSON.parse(data.content || '{}');
        const hooks = parsed.hooks || {};
        // Merge — local overrides/extends global
        for (const [event, items] of Object.entries(hooks)) {
          if (Array.isArray(items)) {
            allHooks[event] = [...(allHooks[event] || []), ...items];
          }
        }
      }
    } catch {
      // ignore
    }
  }

  const entries = Object.entries(allHooks);
  if (entries.length === 0) return HARDCODED_HOOKS;

  return entries.map(([event, items]: [string, any[]]) => ({
    event,
    desc: EVENT_DESCRIPTIONS[event] || event,
    hooks: items.map((h: any, i: number) => ({
      name: h.matcher || (typeof h.command === 'string' ? h.command.split('/').pop() : null) || `hook-${i}`,
      cmd: typeof h.command === 'string' ? h.command : (Array.isArray(h.command) ? h.command.join(' ') : ''),
      enabled: true,
    })),
  }));
}

const HooksView: Component = () => {
  const [hookData] = createResource(discoverHooks);

  const hookTypes = () => hookData() ?? HARDCODED_HOOKS;

  return (
    <div class="flex-1 overflow-auto p-6" style={{ "background-color": "#0a0e14" }}>
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">Hooks</h2>
          <p class="mt-1 text-sm text-gray-400">
            {hookData.loading ? 'Discovering hooks...' : 'Claude Code hooks that run at specific lifecycle events.'}
          </p>
        </div>
      </div>

      {/* Hook type sections */}
      <div class="space-y-6">
        <For each={hookTypes()}>
          {(hookType) => (
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
                  onClick={() => addToast("info", "Add hooks in .claude/settings.json under hooks." + hookType.event.toLowerCase().replace(/\s/g, ""))}>
                  + Add Hook
                </button>
              </div>

              {/* Hook rows */}
              <div class="space-y-2">
                <For each={hookType.hooks} fallback={
                  <div
                    class="rounded-lg px-4 py-3 text-xs text-gray-600 border"
                    style={{ "background-color": "#111827", "border-color": "#1f2937" }}
                  >
                    No hooks configured.
                  </div>
                }>
                  {(hook) => (
                    <div
                      class="flex items-center gap-3 rounded-lg px-4 py-3 border"
                      style={{ "background-color": "#111827", "border-color": "#1f2937" }}
                    >
                      {/* Enable/disable dot — TODO: toggle won't persist until hooks use reactive state */}
                      <span
                        class="h-2.5 w-2.5 shrink-0 rounded-full cursor-pointer"
                        classList={{
                          "bg-green-500": hook.enabled,
                          "bg-gray-600": !hook.enabled,
                        }}
                        title={hook.enabled ? "Enabled" : "Disabled"}
                        onClick={() => {
                          hook.enabled = !hook.enabled;
                          addToast("info", `Hook ${hook.name} ${hook.enabled ? 'enabled' : 'disabled'}`);
                        }}
                      />
                      {/* Hook name */}
                      <span class="text-sm font-bold text-gray-200 min-w-[160px]">
                        {hook.name}
                      </span>
                      {/* Command */}
                      <span
                        class="text-xs text-gray-500 truncate"
                        style={{ "font-family": "'JetBrains Mono', monospace" }}
                      >
                        {hook.cmd}
                      </span>
                    </div>
                  )}
                </For>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default HooksView;
