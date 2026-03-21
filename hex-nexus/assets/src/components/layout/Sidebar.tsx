import { Component, For, createSignal, createMemo, Show } from 'solid-js';
import { registryAgents, swarms, swarmTasks, swarmAgents } from '../../stores/connection';
import { openPane, replaceActivePane } from '../../stores/panes';
import { setSpawnDialogOpen } from '../../stores/ui';
import { setPanelContent } from '../../stores/context-panel';
import { sessions, activeSessionId, setActiveSessionId, createSession } from '../../stores/session';
import { clearMessages } from '../../stores/chat';
import { viewMode, setViewMode } from '../../stores/view';
import { navigate } from '../../stores/router';
import HealthBadge from '../health/HealthBadge';

const SectionHeader: Component<{ title: string; expanded: boolean; onToggle: () => void }> = (props) => (
  <button
    class="flex w-full items-center justify-between px-4 py-3 text-sm font-bold uppercase tracking-wide text-gray-400 hover:text-gray-200 transition-colors"
    onClick={props.onToggle}
  >
    <span>{props.title}</span>
    <svg
      class="h-4 w-4 transition-transform"
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
  const [sessionsOpen, setSessionsOpen] = createSignal(true);
  const [swarmsOpen, setSwarmsOpen] = createSignal(true);

  // Filter swarms: show those with tasks or recent status, hide completed/empty
  const activeSwarms = createMemo(() => {
    const allTasks = swarmTasks();
    return swarms().filter((s: any) => {
      const sid = s.id ?? s.swarm_id ?? '';
      const status = s.status ?? 'active';
      if (status === 'completed' || status === 'archived') return false;
      const hasTasks = allTasks.some((t: any) => (t.swarmId ?? t.swarm_id) === sid);
      return hasTasks; // Show any swarm that has tasks
    });
  });

  const archivedCount = createMemo(() => swarms().length - activeSwarms().length);

  const taskSummary = createMemo(() => {
    const all = swarmTasks();
    return {
      active: all.filter((t: any) => t.status === 'in-progress' || t.status === 'in_progress' || t.status === 'pending').length,
      done: all.filter((t: any) => t.status === 'completed').length,
      failed: all.filter((t: any) => t.status === 'failed').length,
      total: all.length,
    };
  });

  return (
    <aside class="flex h-full w-72 flex-col border-r border-gray-800 bg-gray-900 overflow-y-auto">
      {/* PROJECTS */}
      <div class="border-b border-gray-800">
        <SectionHeader title="Projects" expanded={projectsOpen()} onToggle={() => setProjectsOpen(!projectsOpen())} />
        <Show when={projectsOpen()}>
          <div class="px-3 pb-3 space-y-1">
            <button
              class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-base text-gray-300 hover:bg-gray-800 transition-colors"
              onClick={() => navigate({ page: "control-plane" })}
            >
              <svg class="h-5 w-5 text-gray-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="3" y="3" width="7" height="7" />
                <rect x="14" y="3" width="7" height="7" />
                <rect x="3" y="14" width="7" height="7" />
                <rect x="14" y="14" width="7" height="7" />
              </svg>
              Overview
            </button>
            <button
              class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-base text-gray-400 hover:bg-gray-800 transition-colors"
              onClick={() => navigate({ page: "project-chat", projectId: "current" })}
            >
              <svg class="h-5 w-5 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
              </svg>
              Chat
            </button>
            <button
              class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-base text-gray-400 hover:bg-gray-800 transition-colors"
              onClick={() => navigate({ page: "adrs" })}
            >
              <svg class="h-5 w-5 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                <polyline points="14 2 14 8 20 8" />
                <line x1="16" y1="13" x2="8" y2="13" />
                <line x1="16" y1="17" x2="8" y2="17" />
              </svg>
              ADRs
            </button>
            <button
              class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-base text-gray-400 hover:bg-gray-800 transition-colors"
              onClick={() => navigate({ page: "config", section: "blueprint" })}
            >
              <svg class="h-5 w-5 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
              Config
            </button>
            <HealthBadge />
          </div>
        </Show>
      </div>

      {/* SESSIONS */}
      <div class="border-b border-gray-800">
        <div class="flex items-center justify-between pr-3">
          <SectionHeader title="Sessions" expanded={sessionsOpen()} onToggle={() => setSessionsOpen(!sessionsOpen())} />
          <button
            class="rounded-md p-2 text-gray-500 hover:bg-gray-800 hover:text-cyan-400 transition-colors"
            onClick={() => { createSession(); clearMessages(); }}
            title="New session"
          >
            <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
          </button>
        </div>
        <Show when={sessionsOpen()}>
          <div class="px-3 pb-3 space-y-1">
            <For each={sessions()}>
              {(session) => (
                <button
                  class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-base transition-colors"
                  classList={{
                    "bg-gray-800 text-gray-100": session.id === activeSessionId(),
                    "text-gray-400 hover:bg-gray-800/50 hover:text-gray-300": session.id !== activeSessionId(),
                  }}
                  onClick={() => setActiveSessionId(session.id)}
                >
                  <span
                    class="h-2.5 w-2.5 shrink-0 rounded-full"
                    classList={{
                      "bg-green-500": session.status === "active",
                      "bg-yellow-500": session.status === "paused",
                      "bg-gray-500": session.status === "completed",
                    }}
                  />
                  <span class="truncate">{session.name}</span>
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>

      {/* HEXFLO — Agents + Swarms + Tasks unified */}
      <div class="border-b border-gray-800">
        <div class="flex items-center justify-between pr-3">
          <SectionHeader title="HexFlo" expanded={swarmsOpen()} onToggle={() => setSwarmsOpen(!swarmsOpen())} />
          <button
            class="flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-sm text-gray-500 hover:bg-gray-800 hover:text-cyan-400 transition-colors"
            onClick={() => setSpawnDialogOpen(true)}
            title="Spawn agent (Ctrl+N)"
          >
            <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
            <span>Spawn</span>
          </button>
        </div>
        <Show when={swarmsOpen()}>
          <div class="px-3 pb-3 space-y-2">
            {/* Agents */}
            <Show when={registryAgents().length > 0}>
              <div class="text-sm font-semibold uppercase tracking-wide text-gray-500 px-1 pt-1">Agents</div>
              <For each={registryAgents()}>
                {(agent) => (
                  <div
                    class="flex items-center gap-3 rounded-lg px-3 py-2.5 text-base hover:bg-gray-800/50 transition-colors cursor-pointer"
                    onClick={() => {
                      const agentId = agent.id ?? agent.agent_id ?? '';
                      const agentName = agent.name ?? agent.agent_name ?? 'unnamed';
                      setPanelContent({ type: "agent-detail", agentId, agentName });
                      if (viewMode() === 'panes') {
                        openPane('agent-log', agentName, { agentId });
                      }
                    }}
                  >
                    <span class={`h-2.5 w-2.5 shrink-0 rounded-full ${agentStatusColor(agent)}`} />
                    <span class="truncate font-mono text-gray-300">{agent.name ?? agent.agent_name ?? 'unnamed'}</span>
                  </div>
                )}
              </For>
            </Show>

            <Show when={activeSwarms().length === 0 && registryAgents().length === 0}>
              <p class="px-2 py-2 text-base text-gray-600">No active swarms or tasks</p>
            </Show>

            {/* Task summary bar */}
            <Show when={taskSummary().total > 0}>
              <div class="flex flex-wrap items-center gap-3 px-2 py-2 text-sm">
                <Show when={taskSummary().active > 0}>
                  <span class="flex items-center gap-2 text-cyan-400">
                    <span class="h-2.5 w-2.5 rounded-full bg-cyan-400 animate-pulse" />
                    {taskSummary().active} active
                  </span>
                </Show>
                <Show when={taskSummary().done > 0}>
                  <span class="text-green-400">{taskSummary().done} done</span>
                </Show>
                <Show when={taskSummary().failed > 0}>
                  <span class="text-red-400">{taskSummary().failed} failed</span>
                </Show>
                <Show when={archivedCount() > 0}>
                  <span class="text-gray-600">{archivedCount()} archived</span>
                </Show>
              </div>
            </Show>

            {/* Active swarms with inline tasks */}
            <For each={activeSwarms()}>
              {(swarm) => {
                const swarmId = () => swarm.id ?? swarm.swarm_id ?? '';
                const swarmName = () => swarm.name ?? swarm.swarm_name ?? 'unnamed';
                const tasks = () => swarmTasks().filter((t: any) => (t.swarmId ?? t.swarm_id) === swarmId());
                const pendingTasks = () => tasks().filter((t: any) => t.status !== 'completed');
                const doneCount = () => tasks().filter((t: any) => t.status === 'completed').length;

                      return (
                        <div class="rounded-lg border border-gray-700/50 bg-gray-800/20">
                          {/* Swarm header */}
                          <div
                            class="flex items-center gap-2 rounded-t-lg px-3 py-2.5 text-base hover:bg-gray-800/50 transition-colors cursor-pointer"
                            onClick={() => {
                              setPanelContent({ type: "swarm-detail", swarmId: swarmId(), swarmName: swarmName() });
                              if (viewMode() === 'panes') {
                                openPane('swarm-monitor', swarmName(), { swarmId: swarmId() });
                              }
                            }}
                          >
                            <span class="font-mono font-bold text-gray-200 truncate">{swarmName()}</span>
                            <span class="ml-auto flex items-center gap-2">
                              <span class="text-sm text-gray-500">{doneCount()}/{tasks().length}</span>
                              <span class="rounded-md bg-cyan-900/40 px-2 py-0.5 text-xs font-medium text-cyan-400">{swarm.topology ?? 'mesh'}</span>
                            </span>
                          </div>

                          {/* Progress bar with percentage */}
                          <Show when={tasks().length > 0}>
                            {(() => {
                              const pct = () => Math.round((doneCount() / Math.max(tasks().length, 1)) * 100);
                              const inProgress = () => tasks().filter((t: any) => t.status === 'in-progress' || t.status === 'in_progress').length;
                              const failed = () => tasks().filter((t: any) => t.status === 'failed').length;
                              return (
                                <div class="mx-3 mb-1">
                                  <div class="flex items-center justify-between mb-0.5 text-[10px]">
                                    <span classList={{
                                      "text-gray-500": pct() === 0,
                                      "text-cyan-400": pct() > 0 && pct() < 100,
                                      "text-green-400": pct() === 100,
                                    }}>
                                      {pct()}%
                                      <Show when={inProgress() > 0}>
                                        <span class="ml-1 text-cyan-400">({inProgress()} running)</span>
                                      </Show>
                                      <Show when={failed() > 0}>
                                        <span class="ml-1 text-red-400">({failed()} failed)</span>
                                      </Show>
                                    </span>
                                  </div>
                                  <div class="h-2 rounded-full bg-gray-800 overflow-hidden">
                                    <div
                                      class="h-full rounded-full transition-all duration-500 ease-out"
                                      classList={{
                                        "bg-gray-600": pct() === 0,
                                        "bg-cyan-500": pct() > 0 && pct() < 100 && failed() === 0,
                                        "bg-green-500": pct() === 100,
                                        "bg-red-500": failed() > 0,
                                      }}
                                      style={{ width: `${pct()}%` }}
                                    />
                                  </div>
                                </div>
                              );
                            })()}
                          </Show>

                          {/* Inline task list — show only non-completed tasks, plus last 3 completed */}
                          <Show when={tasks().length > 0}>
                            <div class="px-2 py-2 space-y-1">
                              <For each={[...pendingTasks(), ...tasks().filter((t: any) => t.status === 'completed').slice(-3)]}>
                                {(task) => (
                                  <div class="flex items-center gap-2.5 px-2 py-1.5 text-sm rounded-md hover:bg-gray-800/40">
                                    <span class="h-2.5 w-2.5 shrink-0 rounded-full"
                                      classList={{
                                        "bg-green-500": task.status === "completed",
                                        "bg-cyan-400 animate-pulse": task.status === "in-progress" || task.status === "in_progress",
                                        "bg-gray-600": task.status === "pending",
                                        "bg-red-500": task.status === "failed",
                                      }}
                                    />
                                    <span class="truncate" classList={{
                                      "text-gray-300": task.status !== "completed",
                                      "text-gray-600 line-through": task.status === "completed",
                                    }}>{task.title ?? task.name ?? 'task'}</span>
                                  </div>
                                )}
                              </For>
                              <Show when={doneCount() > 3}>
                                <div class="px-2 py-1 text-xs text-gray-600">
                                  +{doneCount() - 3} completed tasks
                                </div>
                              </Show>
                            </div>
                          </Show>
                        </div>
                      );
                    }}
            </For>
          </div>
        </Show>
      </div>
    </aside>
  );
};

export default Sidebar;
