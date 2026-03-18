import { describe, it, expect, beforeEach, mock } from 'bun:test';
import { WorkplanExecutor } from '../../src/core/usecases/workplan-executor.js';
import type { ISwarmPort, SwarmStatus, SwarmTask, SwarmMemoryEntry } from '../../src/core/ports/swarm.js';
import type { Workplan, StepResult } from '../../src/core/ports/index.js';

// ─── Helpers ────────────────────────────────────────────────

function makeSwarmStatus(overrides?: Partial<SwarmStatus>): SwarmStatus {
  return {
    id: 'swarm-1',
    topology: 'hierarchical',
    agentCount: 1,
    activeTaskCount: 0,
    completedTaskCount: 0,
    status: 'running',
    ...overrides,
  };
}

/**
 * In-memory store that simulates AgentDB's hierarchical memory.
 * Keyed by `${layer}/${namespace}/${key}`.
 */
function createMemoryBackedSwarmPort() {
  const hierarchicalMemory = new Map<string, string>();
  let taskCounter = 0;

  const port: ISwarmPort = {
    status: mock(() => Promise.resolve(makeSwarmStatus())),
    listTasks: mock(() => Promise.resolve([] as SwarmTask[])),
    healthCheck: mock(() => Promise.resolve(true)),
    init: mock(() => Promise.resolve(makeSwarmStatus())),
    shutdown: mock(() => Promise.resolve()),
    createTask: mock(() => {
      taskCounter++;
      return Promise.resolve({ id: `task-${taskCounter}`, title: '', agentRole: 'coder' as const, status: 'pending' as const });
    }),
    completeTask: mock(() => Promise.resolve()),
    spawnAgent: mock(() => Promise.resolve({ id: 'a1', name: 'agent', role: 'coder' as const, status: 'active' as const })),
    terminateAgent: mock(() => Promise.resolve()),
    listAgents: mock(() => Promise.resolve([])),
    memoryStore: mock(() => Promise.resolve()),
    memoryRetrieve: mock(() => Promise.resolve(null)),
    memorySearch: mock(() => Promise.resolve([])),
    patternStore: mock(() => Promise.resolve({ id: 'p1', name: '', category: '', content: '', confidence: 0, accessCount: 0, createdAt: '', updatedAt: '' })),
    patternSearch: mock(() => Promise.resolve([])),
    patternFeedback: mock(() => Promise.resolve()),
    sessionStart: mock(() => Promise.resolve({ sessionId: 's1', agentName: '', startedAt: '', status: 'active' as const })),
    sessionEnd: mock(() => Promise.resolve()),

    // Real hierarchical memory backed by the in-memory map
    hierarchicalStore: mock((layer: string, namespace: string, key: string, value: string) => {
      hierarchicalMemory.set(`${layer}/${namespace}/${key}`, value);
      return Promise.resolve();
    }),
    hierarchicalRecall: mock((layer: string, namespace?: string, key?: string) => {
      const prefix = [layer, namespace, key].filter(Boolean).join('/');
      const results: SwarmMemoryEntry[] = [];
      for (const [k, v] of hierarchicalMemory) {
        if (k.startsWith(prefix)) {
          results.push({ key: k, value: v, namespace: layer, tags: [], createdAt: new Date().toISOString() });
        }
      }
      return Promise.resolve(results);
    }),

    consolidate: mock(() => Promise.resolve({ merged: 0, removed: 0 })),
    contextSynthesize: mock(() => Promise.resolve('')),
    getProgressReport: mock(() => Promise.resolve({ swarmId: '', tasks: [], agents: [], patterns: { total: 0, recentlyUsed: 0 }, sessions: [], overallPercent: 0, phase: '' })),
  };

  return { port, hierarchicalMemory };
}

function makePlan(overrides?: Partial<Workplan>): Workplan {
  return {
    id: 'test-plan-1',
    title: 'Test Plan',
    estimatedTokenBudget: 8000,
    steps: [
      { id: 'step-1', description: 'Create domain types', adapter: 'domain', dependencies: [] },
      { id: 'step-2', description: 'Create port interface', adapter: 'ports', dependencies: ['step-1'] },
      { id: 'step-3', description: 'Implement adapter', adapter: 'adapters/secondary', dependencies: ['step-2'] },
    ],
    ...overrides,
  };
}

async function collectResults(gen: AsyncGenerator<StepResult>): Promise<StepResult[]> {
  const results: StepResult[] = [];
  for await (const r of gen) results.push(r);
  return results;
}

// ─── Stubs for required constructor args ────────────────────

const stubLLM = { prompt: mock(() => Promise.resolve({ content: '{}' })), streamPrompt: mock() } as any;
const stubAST = { extractSummary: mock(() => Promise.resolve({ exports: [], imports: [], dependencies: [], lineCount: 0, tokenEstimate: 0 })), diffStructural: mock() } as any;
const stubFS = { read: mock(() => Promise.resolve('')), write: mock(() => Promise.resolve()), exists: mock(() => Promise.resolve(false)), glob: mock(() => Promise.resolve([])), mtime: mock(() => Promise.resolve(0)), streamFiles: mock() } as any;

// ─── Tests ──────────────────────────────────────────────────

describe('Workplan Resume (crash recovery via AgentDB)', () => {
  it('persists step progress after each completed step', async () => {
    const { port, hierarchicalMemory } = createMemoryBackedSwarmPort();
    const executor = new WorkplanExecutor(stubLLM, stubAST, stubFS, port);
    const plan = makePlan();

    await collectResults(executor.executePlan(plan));

    // Progress should be stored in hierarchical memory
    const progressKey = `workplan/${plan.id}/progress`;
    expect(hierarchicalMemory.has(progressKey)).toBe(true);

    const progress = JSON.parse(hierarchicalMemory.get(progressKey)!);
    expect(progress.completed).toContain('step-1');
    expect(progress.completed).toContain('step-2');
    expect(progress.completed).toContain('step-3');
    expect(progress.completed.length).toBe(3);
  });

  it('skips completed steps when resuming after crash', async () => {
    const { port, hierarchicalMemory } = createMemoryBackedSwarmPort();

    // Simulate prior progress: step-1 and step-2 were completed before crash
    const progressKey = `workplan/test-plan-1/progress`;
    hierarchicalMemory.set(progressKey, JSON.stringify({
      completed: ['step-1', 'step-2'],
      failed: [],
      updatedAt: new Date().toISOString(),
    }));

    const executor = new WorkplanExecutor(stubLLM, stubAST, stubFS, port);
    const plan = makePlan();

    const results = await collectResults(executor.executePlan(plan));

    // step-1 and step-2 should be yielded as 'passed' (skip) without creating tasks
    const step1Results = results.filter(r => r.stepId === 'step-1');
    const step2Results = results.filter(r => r.stepId === 'step-2');
    const step3Results = results.filter(r => r.stepId === 'step-3');

    expect(step1Results).toEqual([{ stepId: 'step-1', status: 'passed' }]);
    expect(step2Results).toEqual([{ stepId: 'step-2', status: 'passed' }]);

    // step-3 should run normally (running → passed)
    expect(step3Results.length).toBe(2);
    expect(step3Results[0].status).toBe('running');
    expect(step3Results[1].status).toBe('passed');

    // Only 1 task should have been created (step-3), not 3
    expect(port.createTask).toHaveBeenCalledTimes(1);
  });

  it('re-attempts previously failed steps on resume', async () => {
    const { port, hierarchicalMemory } = createMemoryBackedSwarmPort();

    // Simulate: step-1 completed, step-2 failed (user may have fixed the issue)
    const progressKey = `workplan/test-plan-1/progress`;
    hierarchicalMemory.set(progressKey, JSON.stringify({
      completed: ['step-1'],
      failed: ['step-2'],
      updatedAt: new Date().toISOString(),
    }));

    const executor = new WorkplanExecutor(stubLLM, stubAST, stubFS, port);
    const plan = makePlan();

    const results = await collectResults(executor.executePlan(plan));

    // step-1: skipped (already completed)
    const step1Results = results.filter(r => r.stepId === 'step-1');
    expect(step1Results).toEqual([{ stepId: 'step-1', status: 'passed' }]);

    // step-2: should be re-attempted (not skipped)
    const step2Results = results.filter(r => r.stepId === 'step-2');
    expect(step2Results.length).toBe(2); // running → passed
    expect(step2Results[0].status).toBe('running');

    // step-3: should also run (deps met after step-2 re-attempt succeeds)
    const step3Results = results.filter(r => r.stepId === 'step-3');
    expect(step3Results.length).toBe(2);
  });

  it('handles fresh plan with no prior progress', async () => {
    const { port } = createMemoryBackedSwarmPort();
    const executor = new WorkplanExecutor(stubLLM, stubAST, stubFS, port);
    const plan = makePlan();

    const results = await collectResults(executor.executePlan(plan));

    // All 3 steps should run: each yields running + passed = 6 results
    expect(results.length).toBe(6);
    expect(results.filter(r => r.status === 'running').length).toBe(3);
    expect(results.filter(r => r.status === 'passed').length).toBe(3);
  });

  it('persists failure state so next resume knows what failed', async () => {
    const { port, hierarchicalMemory } = createMemoryBackedSwarmPort();

    // Make step-2's createTask throw to simulate a failure
    let callCount = 0;
    (port.createTask as ReturnType<typeof mock>).mockImplementation(() => {
      callCount++;
      if (callCount === 2) return Promise.reject(new Error('swarm timeout'));
      return Promise.resolve({ id: `task-${callCount}`, title: '', agentRole: 'coder', status: 'pending' });
    });

    const executor = new WorkplanExecutor(stubLLM, stubAST, stubFS, port);
    const plan = makePlan();

    await collectResults(executor.executePlan(plan));

    const progressKey = `workplan/${plan.id}/progress`;
    const progress = JSON.parse(hierarchicalMemory.get(progressKey)!);
    expect(progress.completed).toContain('step-1');
    expect(progress.failed).toContain('step-2');
    // step-3 fails due to unmet dependencies (step-2 failed)
    expect(progress.failed).toContain('step-3');
  });
});
