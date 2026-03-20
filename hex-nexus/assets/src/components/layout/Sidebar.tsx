import { Component, For, createSignal, Show } from 'solid-js';
import { registryAgents, swarms, swarmTasks } from '../../stores/connection';
import { openPane, replaceActivePane } from '../../stores/panes';
import { setSpawnDialogOpen } from '../../stores/ui';

const SectionHeader: Component<{ title: string; expanded: boolean; onToggle: () => void }> = (props) => (
  <button
    class="flex w-full items-center justify-between px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-gray-300 hover:text-gray-300 transition-colors"
    onClick={props.onToggle}
  >
    <span>{props.title}</span>
    <svg
      class="h-3 w-3 transition-transform"
      classList={{ 'rotate-90': props.expanded }}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.5"
    >
      <polyline points="9 18 15 12 9 6" />
    </svg>
  </button>
);

function agentStatusColor(agent: any): string {
  const status = agent?.status ?? agent?.state ?? '';
  if (status === 'active' || status === 'online') return 'bg-green-500';
  if (status === 'stale' || status === 'warning') return 'bg-yellow-500';
  return 'bg-red-500';
}

const Sidebar: Component = () => {
  const [projectsOpen, setProjectsOpen] = createSignal(true);
  const [agentsOpen, setAgentsOpen] = createSignal(true);
  const [swarmsOpen, setSwarmsOpen] = createSignal(true);

  const taskCountForSwarm = (swarmId: string) => {
    return swarmTasks().filter((t: any) => t.swarmId === swarmId || t.swarm_id === swarmId).length;
  };

  return (
    <aside class="flex h-full w-60 flex-col border-r border-gray-800 bg-gray-900 overflow-y-auto">
      {/* PROJECTS */}
      <div class="border-b border-gray-800">
        <SectionHeader title="Projects" expanded={projectsOpen()} onToggle={() => setProjectsOpen(!projectsOpen())} />
        <Show when={projectsOpen()}>
          <div class="px-2 pb-2">
            <button
              class="flex w-full items-center gap-2 rounded px-2 py-1.5 text-xs text-gray-300 hover:bg-gray-800 transition-colors"
              onClick={() => replaceActivePane('project-overview', 'Projects')}
            >
              <svg class="h-3.5 w-3.5 text-gray-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="3" y="3" width="7" height="7" />
                <rect x="14" y="3" width="7" height="7" />
                <rect x="3" y="14" width="7" height="7" />
                <rect x="14" y="14" width="7" height="7" />
              </svg>
              Overview
            </button>
          </div>
        </Show>
      </div>

      {/* AGENTS */}
      <div class="border-b border-gray-800">
        <div class="flex items-center justify-between pr-2">
          <SectionHeader title="Agents" expanded={agentsOpen()} onToggle={() => setAgentsOpen(!agentsOpen())} />
          <button
            class="rounded p-1 text-gray-300 hover:bg-gray-800 hover:text-cyan-400 transition-colors"
            onClick={() => setSpawnDialogOpen(true)}
            title="Spawn agent (Ctrl+N)"
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
          </button>
        </div>
        <Show when={agentsOpen()}>
          <div class="px-2 pb-2 space-y-0.5">
            <Show when={registryAgents().length === 0}>
              <p class="px-1 text-xs text-gray-300">No agents registered</p>
            </Show>
            <For each={registryAgents()}>
              {(agent) => (
                <div
                  class="flex items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-gray-800 transition-colors cursor-pointer"
                  onClick={() => openPane('agent-log', agent.name ?? agent.agent_name ?? 'agent', { agentId: agent.id ?? agent.agent_id ?? '' })}
                >
                  <span class={`h-2 w-2 shrink-0 rounded-full ${agentStatusColor(agent)}`} />
                  <span class="truncate font-mono text-gray-300">{agent.name ?? agent.agent_name ?? 'unnamed'}</span>
                  <span class="ml-auto truncate text-[10px] text-gray-300">{agent.project ?? ''}</span>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>

      {/* SWARMS */}
      <div>
        <SectionHeader title="Swarms" expanded={swarmsOpen()} onToggle={() => setSwarmsOpen(!swarmsOpen())} />
        <Show when={swarmsOpen()}>
          <div class="px-2 pb-2 space-y-0.5">
            <Show when={swarms().length === 0}>
              <p class="px-1 text-xs text-gray-300">No active swarms</p>
            </Show>
            <For each={swarms()}>
              {(swarm) => (
                <div
                  class="flex items-center gap-2 rounded px-2 py-1.5 text-xs hover:bg-gray-800 transition-colors cursor-pointer"
                  onClick={() => openPane('swarm-monitor', swarm.name ?? swarm.swarm_name ?? 'swarm', { swarmId: swarm.id ?? swarm.swarm_id ?? '' })}
                >
                  <span class="truncate font-mono text-gray-300">{swarm.name ?? swarm.swarm_name ?? 'unnamed'}</span>
                  <span class="ml-auto flex items-center gap-1.5">
                    <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300">
                      {taskCountForSwarm(swarm.id ?? swarm.swarm_id ?? '')} tasks
                    </span>
                    <span class="rounded bg-cyan-900/40 px-1.5 py-0.5 text-[10px] text-cyan-400">
                      {swarm.topology ?? 'mesh'}
                    </span>
                  </span>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </aside>
  );
};

export default Sidebar;
