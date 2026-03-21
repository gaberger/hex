/**
 * hexflo-monitor.ts — Watches SpacetimeDB swarm/task changes.
 *
 * NO toasts for routine task updates — those are distracting.
 * The sidebar progress bars update reactively via SolidJS signals.
 * Only toast for errors/failures that need attention.
 */
import { createEffect, on } from "solid-js";
import { swarmTasks, registryAgents } from "./connection";
import { addToast } from "./toast";

let prevTaskMap = new Map<string, string>();
let initialized = false;

export function startHexFloMonitor() {
  // Only notify on FAILURES — everything else is visible in the sidebar
  createEffect(on(swarmTasks, (tasks) => {
    if (!initialized) {
      prevTaskMap = new Map(tasks.map((t: any) => [t.id ?? t.task_id ?? '', t.status ?? 'pending']));
      initialized = true;
      return;
    }

    for (const task of tasks) {
      const id = task.id ?? task.task_id ?? '';
      const status = task.status ?? 'pending';
      const title = task.title ?? task.name ?? 'task';
      const prevStatus = prevTaskMap.get(id);

      // Only toast on failures — success is visible in the progress bar
      if (prevStatus && prevStatus !== status && status === 'failed') {
        addToast("error", `Task failed: ${title}`);
      }
    }

    prevTaskMap = new Map(tasks.map((t: any) => [t.id ?? t.task_id ?? '', t.status ?? 'pending']));
  }, { defer: true }));
}
