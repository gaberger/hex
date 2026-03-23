/**
 * hexflo-monitor.ts — Watches SpacetimeDB swarm/task changes.
 *
 * NO toasts for routine task updates — those are distracting.
 * The sidebar progress bars update reactively via SolidJS signals.
 * Only toast for errors/failures that need attention.
 *
 * createEffect is wrapped in createRoot inside startHexFloMonitor()
 * to ensure proper reactive ownership (ADR-2603231000).
 */
import { createEffect, createRoot, on } from "solid-js";
import { swarmTasks, swarms } from "./connection";
import { addToast } from "./toast";

let prevTaskMap = new Map<string, string>();
let initialized = false;
const completedSwarms = new Set<string>();
let _started = false;

export function startHexFloMonitor() {
  if (_started) return;
  _started = true;

  // Wrap in createRoot so the createEffect has a proper reactive owner
  createRoot(() => {
    // Notify on failures and swarm completion
    createEffect(on(swarmTasks, (tasks) => {
      if (!initialized) {
        prevTaskMap = new Map(tasks.map((t: any) => [t.id ?? t.task_id ?? '', t.status ?? 'pending']));
        // Seed completedSwarms from swarms already marked done in SpacetimeDB
        for (const s of swarms()) {
          const status = s.status ?? s.swarm_status ?? 'active';
          if (status === 'completed' || status === 'done') {
            completedSwarms.add(s.id ?? s.swarm_id ?? '');
          }
        }
        initialized = true;
        return;
      }

      for (const task of tasks) {
        const id = task.id ?? task.task_id ?? '';
        const status = task.status ?? 'pending';
        const title = task.title ?? task.name ?? 'task';
        const prevStatus = prevTaskMap.get(id);

        // Toast on failures
        if (prevStatus && prevStatus !== status && status === 'failed') {
          addToast("error", `Task failed: ${title}`);
        }
      }

      prevTaskMap = new Map(tasks.map((t: any) => [t.id ?? t.task_id ?? '', t.status ?? 'pending']));

      // Check if all tasks in any swarm are now completed
      const tasksBySwarm = new Map<string, { total: number; completed: number }>();
      for (const task of tasks) {
        const swarmId = task.swarmId ?? task.swarm_id ?? '';
        if (!swarmId) continue;
        const entry = tasksBySwarm.get(swarmId) ?? { total: 0, completed: 0 };
        entry.total++;
        const status = task.status ?? 'pending';
        if (status === 'completed' || status === 'done') entry.completed++;
        tasksBySwarm.set(swarmId, entry);
      }

      for (const [swarmId, counts] of tasksBySwarm) {
        if (counts.total > 0 && counts.completed === counts.total && !completedSwarms.has(swarmId)) {
          completedSwarms.add(swarmId);
          const swarm = swarms().find((s: any) => (s.id ?? s.swarm_id ?? '') === swarmId);
          // Don't toast for swarms already marked completed in SpacetimeDB —
          // they fire on subscription re-delivery after reconnect (ADR-055 fix)
          const swarmStatus = swarm?.status ?? swarm?.swarm_status ?? 'active';
          if (swarmStatus === 'completed' || swarmStatus === 'done') continue;
          const name = swarm?.name ?? swarm?.swarm_name ?? swarmId;
          addToast("success", `Swarm "${name}" completed — all ${counts.total} tasks done`);
        }
      }
    }, { defer: true }));
  });
}
