import { Component, createResource, createMemo, For, Show } from 'solid-js';
import { navigate, route } from '../../stores/router';
import { swarms, swarmTasks } from '../../stores/connection';
import { restClient } from '../../services/rest-client';

// ── Types ────────────────────────────────────────────────────────────────────

interface ADRItem {
  id: string;
  title: string;
  status: string;
  date?: string;
}

interface WorkPlanItem {
  id: string;
  name?: string;
  title?: string;
  adr?: string;
  adrRef?: string;
  status?: string;
  createdAt?: string;
}

interface TimelineNode {
  type: 'adr' | 'workplan' | 'swarm' | 'task';
  id: string;
  title: string;
  status: string;
  timestamp: string;
  route?: Parameters<typeof navigate>[0];
}

// ── Color map by node type ───────────────────────────────────────────────────

const NODE_COLORS: Record<TimelineNode['type'], { dot: string; text: string; badge: string }> = {
  adr:      { dot: 'bg-amber-400',  text: 'text-hex-primary', badge: 'bg-amber-500/15 text-amber-400 border-amber-500/30' },
  workplan: { dot: 'bg-purple-400', text: 'text-hex-ports',   badge: 'bg-purple-500/15 text-purple-400 border-purple-500/30' },
  swarm:    { dot: 'bg-green-400',  text: 'text-green-400',   badge: 'bg-green-500/15 text-green-400 border-green-500/30' },
  task:     { dot: 'bg-cyan-400',   text: 'text-cyan-400',    badge: 'bg-cyan-500/15 text-cyan-400 border-cyan-500/30' },
};

const TYPE_LABELS: Record<TimelineNode['type'], string> = {
  adr: 'ADR',
  workplan: 'WorkPlan',
  swarm: 'Swarm',
  task: 'Task',
};

// ── Helpers ──────────────────────────────────────────────────────────────────

function parseTimestamp(raw?: string): string {
  if (!raw) return '';
  try {
    const d = new Date(raw);
    if (isNaN(d.getTime())) return raw;
    return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
  } catch {
    return raw;
  }
}

// ── Component ────────────────────────────────────────────────────────────────

const GovernanceTimeline: Component = () => {
  const projectId = createMemo(() => {
    const r = route();
    return (r as { projectId?: string }).projectId ?? '';
  });

  // 1. Fetch ADRs from REST
  const [adrList] = createResource(
    () => projectId(),
    async (pid) => {
      try {
        const url = pid ? `/api/projects/${encodeURIComponent(pid)}/adrs` : '/api/adrs';
        return await restClient.get<ADRItem[]>(url);
      } catch {
        return [] as ADRItem[];
      }
    },
  );

  // 2. Fetch workplans from REST
  const [workplanList] = createResource(
    () => projectId(),
    async () => {
      try {
        return await restClient.get<WorkPlanItem[]>('/api/workplan/list');
      } catch {
        return [] as WorkPlanItem[];
      }
    },
  );

  // 3 & 4. Swarms and tasks come from the SpacetimeDB connection store (reactive)

  // Build unified timeline
  const timeline = createMemo<TimelineNode[]>(() => {
    const nodes: TimelineNode[] = [];
    const pid = projectId();

    // ADRs
    for (const adr of adrList() ?? []) {
      nodes.push({
        type: 'adr',
        id: adr.id,
        title: `ADR-${adr.id}: ${adr.title}`,
        status: adr.status,
        timestamp: adr.date ?? '',
        route: pid ? { page: 'project-adr-detail', projectId: pid, adrId: adr.id } : undefined,
      });
    }

    // WorkPlans
    for (const wp of workplanList() ?? []) {
      const wpTitle = wp.title ?? wp.name ?? wp.id;
      const adrRef = wp.adr ?? wp.adrRef ?? '';
      nodes.push({
        type: 'workplan',
        id: wp.id,
        title: `${wpTitle}${adrRef ? ` (ADR ${adrRef})` : ''}`,
        status: wp.status ?? 'draft',
        timestamp: wp.createdAt ?? '',
        route: pid ? { page: 'project-workplan-detail', projectId: pid, workplanId: wp.id } : undefined,
      });
    }

    // Swarms (from SpacetimeDB)
    for (const sw of swarms()) {
      const swarmId = sw.swarmId ?? sw.swarm_id ?? sw.id ?? '';
      const swarmName = sw.name ?? swarmId;
      nodes.push({
        type: 'swarm',
        id: swarmId,
        title: `Swarm: ${swarmName}`,
        status: sw.status ?? 'active',
        timestamp: sw.createdAt ?? sw.created_at ?? '',
        route: pid ? { page: 'project-swarm-detail', projectId: pid, swarmId } : undefined,
      });
    }

    // Tasks (from SpacetimeDB)
    for (const t of swarmTasks()) {
      const taskId = t.taskId ?? t.task_id ?? t.id ?? '';
      const swarmId = t.swarmId ?? t.swarm_id ?? '';
      nodes.push({
        type: 'task',
        id: taskId,
        title: t.title ?? t.name ?? `Task ${taskId.slice(0, 8)}`,
        status: t.status ?? 'pending',
        timestamp: t.createdAt ?? t.created_at ?? t.completedAt ?? t.completed_at ?? '',
        route: pid && swarmId
          ? { page: 'project-swarm-task', projectId: pid, swarmId, taskId }
          : undefined,
      });
    }

    // Sort by timestamp descending (most recent first), undated at end
    nodes.sort((a, b) => {
      if (!a.timestamp && !b.timestamp) return 0;
      if (!a.timestamp) return 1;
      if (!b.timestamp) return -1;
      return new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime();
    });

    return nodes;
  });

  const isLoading = () => adrList.loading || workplanList.loading;

  return (
    <div class="flex-1 overflow-auto p-6">
      <div class="mb-4">
        <h2 class="text-lg font-bold text-gray-100">Governance Timeline</h2>
        <p class="mt-0.5 text-sm text-gray-500">
          ADR → WorkPlan → Swarm → Task lifecycle
        </p>
      </div>

      <Show when={isLoading()}>
        <div class="flex items-center justify-center py-12 text-sm text-gray-500">
          Loading timeline...
        </div>
      </Show>

      <Show when={!isLoading() && timeline().length === 0}>
        <div class="rounded-lg border border-dashed border-gray-700 p-8 text-center">
          <p class="text-sm text-gray-500">No governance events found for this project.</p>
          <p class="mt-1 text-xs text-gray-600">
            Create an ADR or initialize a swarm to begin tracking.
          </p>
        </div>
      </Show>

      <Show when={!isLoading() && timeline().length > 0}>
        <div class="relative ml-4">
          {/* Vertical line */}
          <div class="absolute left-0 top-0 bottom-0 w-0.5 border-l-2 border-gray-700" />

          <For each={timeline()}>
            {(node) => {
              const colors = NODE_COLORS[node.type];
              return (
                <div class="relative pl-8 pb-6 group">
                  {/* Dot on the timeline */}
                  <div
                    class={`absolute left-0 top-1 -translate-x-1/2 h-3 w-3 rounded-full border-2 border-gray-900 ${colors.dot} group-hover:ring-2 group-hover:ring-gray-600 transition-all`}
                  />

                  {/* Content */}
                  <div
                    class="rounded-lg border border-gray-800 bg-gray-900/50 px-4 py-3 hover:border-gray-700 transition-colors"
                    classList={{ 'cursor-pointer': !!node.route }}
                    onClick={() => {
                      if (node.route) navigate(node.route);
                    }}
                  >
                    {/* Top row: type label + timestamp */}
                    <div class="flex items-center gap-2 mb-1">
                      <span class={`text-[10px] font-bold uppercase tracking-wide ${colors.text}`}>
                        {TYPE_LABELS[node.type]}
                      </span>
                      <Show when={node.timestamp}>
                        <span class="text-[10px] text-gray-600 font-mono">
                          {parseTimestamp(node.timestamp)}
                        </span>
                      </Show>
                      <div class="flex-1" />
                      <span class={`rounded px-1.5 py-0.5 text-[10px] font-medium border ${colors.badge}`}>
                        {node.status}
                      </span>
                    </div>

                    {/* Title */}
                    <p class="text-sm text-gray-300 leading-snug">
                      {node.title}
                    </p>
                  </div>
                </div>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default GovernanceTimeline;
