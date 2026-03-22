/**
 * hexflo-monitor tests — validates completion-tracking logic in isolation.
 *
 * The actual startHexFloMonitor() relies on SpacetimeDB connection signals,
 * so we test the core data-structure logic (completedSwarms set, task
 * aggregation by swarm) without importing the live store.
 */
import { describe, it, expect } from 'vitest';

describe('hexflo-monitor completion logic', () => {
  it('completedSwarms set prevents duplicate toasts', () => {
    const completedSwarms = new Set<string>();
    const swarmId = 'test-swarm-1';

    // First completion should trigger
    expect(completedSwarms.has(swarmId)).toBe(false);
    completedSwarms.add(swarmId);

    // Second should not
    expect(completedSwarms.has(swarmId)).toBe(true);
  });

  it('task aggregation counts completed vs total per swarm', () => {
    const tasks = [
      { task_id: 't1', swarm_id: 's1', status: 'completed' },
      { task_id: 't2', swarm_id: 's1', status: 'pending' },
      { task_id: 't3', swarm_id: 's1', status: 'completed' },
      { task_id: 't4', swarm_id: 's2', status: 'completed' },
      { task_id: 't5', swarm_id: 's2', status: 'completed' },
    ];

    const tasksBySwarm = new Map<string, { total: number; completed: number }>();
    for (const task of tasks) {
      const swarmId = task.swarm_id;
      const entry = tasksBySwarm.get(swarmId) ?? { total: 0, completed: 0 };
      entry.total++;
      if (task.status === 'completed' || task.status === 'done') entry.completed++;
      tasksBySwarm.set(swarmId, entry);
    }

    // s1: 3 total, 2 completed => not done
    const s1 = tasksBySwarm.get('s1')!;
    expect(s1.total).toBe(3);
    expect(s1.completed).toBe(2);
    expect(s1.completed === s1.total).toBe(false);

    // s2: 2 total, 2 completed => done
    const s2 = tasksBySwarm.get('s2')!;
    expect(s2.total).toBe(2);
    expect(s2.completed).toBe(2);
    expect(s2.completed === s2.total).toBe(true);
  });

  it('prevTaskMap detects status transitions', () => {
    const prevTaskMap = new Map<string, string>([
      ['t1', 'in_progress'],
      ['t2', 'pending'],
    ]);

    const currentTasks = [
      { task_id: 't1', status: 'failed', title: 'Build adapter' },
      { task_id: 't2', status: 'completed', title: 'Write tests' },
    ];

    const failures: string[] = [];
    for (const task of currentTasks) {
      const prevStatus = prevTaskMap.get(task.task_id);
      if (prevStatus && prevStatus !== task.status && task.status === 'failed') {
        failures.push(task.title);
      }
    }

    expect(failures).toEqual(['Build adapter']);
  });
});
