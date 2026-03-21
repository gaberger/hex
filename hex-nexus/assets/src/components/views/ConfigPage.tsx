import { Component, Switch, Match, For, createMemo } from 'solid-js';
import { route, navigate } from '../../stores/router';
import { BlueprintView, MCPToolsView, ContextView, HooksView, SkillsView, AgentDefsView, SpacetimeDBView } from '../config';
import { addToast } from '../../stores/toast';

interface NavItem {
  id: string;
  label: string;
  icon: Component;
}

const HexIcon: Component = () => (
  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <polygon points="12,2 22,8.5 22,15.5 12,22 2,15.5 2,8.5" />
  </svg>
);

const WrenchIcon: Component = () => (
  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
  </svg>
);

const HookIcon: Component = () => (
  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <path d="M15.5 4.5l2.8 2.8a1 1 0 0 1 0 1.4L8.3 18.7a1 1 0 0 1-.7.3H5v-2.6a1 1 0 0 1 .3-.7L15.5 4.5z" />
  </svg>
);

const ZapIcon: Component = () => (
  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
  </svg>
);

const FileIcon: Component = () => (
  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
    <polyline points="14 2 14 8 20 8" />
    <line x1="16" y1="13" x2="8" y2="13" />
    <line x1="16" y1="17" x2="8" y2="17" />
  </svg>
);

const BotIcon: Component = () => (
  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <rect x="3" y="11" width="18" height="10" rx="2" />
    <circle cx="12" cy="5" r="2" />
    <path d="M12 7v4" />
    <line x1="8" y1="16" x2="8" y2="16" />
    <line x1="16" y1="16" x2="16" y2="16" />
  </svg>
);

const DatabaseIcon: Component = () => (
  <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <ellipse cx="12" cy="5" rx="9" ry="3" />
    <path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" />
    <path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" />
  </svg>
);

const NAV_ITEMS: NavItem[] = [
  { id: 'blueprint',  label: 'Blueprint',    icon: HexIcon },
  { id: 'tools',      label: 'MCP Tools',    icon: WrenchIcon },
  { id: 'hooks',      label: 'Hooks',        icon: HookIcon },
  { id: 'skills',     label: 'Skills',       icon: ZapIcon },
  { id: 'context',    label: 'Context',      icon: FileIcon },
  { id: 'agents',     label: 'Agent Defs',   icon: BotIcon },
  { id: 'spacetimedb', label: 'SpacetimeDB', icon: DatabaseIcon },
];

const ConfigPage: Component = () => {
  const currentSection = createMemo(() => {
    const r = route();
    return (r as any).section || 'blueprint';
  });

  const projectId = createMemo(() => (route() as any).projectId || '');
  const projectName = createMemo(() => projectId() || 'Global');

  return (
    <div class="flex flex-1 overflow-hidden">
      {/* Left nav */}
      <nav
        class="flex w-60 shrink-0 flex-col border-r border-gray-800 overflow-y-auto"
        style={{ "background-color": "var(--bg-surface)" }}
      >
        <div class="px-4 py-3">
          <div class="flex items-center gap-2 mb-1">
            <svg class="h-4 w-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
            </svg>
            <span class="text-sm font-bold text-cyan-300">{projectName()}</span>
          </div>
          <span class="text-[10px] uppercase tracking-wider text-gray-600">Project Configuration</span>
        </div>
        <div class="flex items-center justify-between px-4 py-2">
          <span class="text-xs font-bold uppercase tracking-wider text-gray-500">Configure</span>
          <button
            class="rounded-lg border border-gray-700 px-3 py-1.5 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
            onClick={async () => {
              try {
                const res = await fetch('/api/config/sync', { method: 'POST' });
                if (res.ok) addToast('success', 'Config re-synced from repo files');
                else addToast('error', 'Sync failed');
              } catch {
                addToast('error', 'Sync failed — is nexus running?');
              }
            }}
          >
            Refresh
          </button>
        </div>
        <div class="flex flex-col gap-0.5 px-2">
          <For each={NAV_ITEMS}>
            {(item) => {
              const selected = () => currentSection() === item.id;
              return (
                <button
                  class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-sm transition-colors text-left"
                  classList={{
                    "bg-gray-800": selected(),
                    "hover:bg-gray-800/50": !selected(),
                  }}
                  onClick={() => navigate({ page: 'config', section: item.id, projectId: projectId() })}
                >
                  <span
                    classList={{
                      "text-cyan-400": selected(),
                      "text-gray-500": !selected(),
                    }}
                  >
                    <item.icon />
                  </span>
                  <span
                    classList={{
                      "text-gray-200 font-semibold": selected(),
                      "text-gray-400": !selected(),
                    }}
                  >
                    {item.label}
                  </span>
                </button>
              );
            }}
          </For>
        </div>
      </nav>

      {/* Center content */}
      <div class="flex flex-1 flex-col overflow-hidden">
        <Switch fallback={
          <div class="flex-1 overflow-auto p-6">
            <h2 class="text-lg font-bold text-gray-200 mb-2">{currentSection()}</h2>
            <p class="text-sm text-gray-500">This configuration section is coming soon.</p>
          </div>
        }>
          <Match when={currentSection() === 'blueprint'}>
            <BlueprintView />
          </Match>
          <Match when={currentSection() === 'tools'}>
            <MCPToolsView />
          </Match>
          <Match when={currentSection() === 'hooks'}>
            <HooksView />
          </Match>
          <Match when={currentSection() === 'skills'}>
            <SkillsView />
          </Match>
          <Match when={currentSection() === 'context'}>
            <ContextView />
          </Match>
          <Match when={currentSection() === 'agents'}>
            <AgentDefsView />
          </Match>
          <Match when={currentSection() === 'spacetimedb'}>
            <SpacetimeDBView />
          </Match>
        </Switch>
      </div>
    </div>
  );
};

export default ConfigPage;
