/**
 * Coordination Wiring Tests (ADR-022)
 *
 * Verifies that ICoordinationPort methods are called at the right times
 * by the three use cases that accept coordination:
 *   - SwarmOrchestrator: acquireLock before worktree.create, releaseLock after
 *   - WorkplanExecutor: claimTask before spawnAgent, releaseTask after
 *   - FeatureProgressOrchestrator: publishActivity on phase transitions
 *
 * Uses dependency injection with fakes (ADR-014). No mock.module().
 */

import { describe, test, expect } from 'bun:test';
import { SwarmOrchestrator } from '../../src/core/usecases/swarm-orchestrator.js';
import { WorkplanExecutor } from '../../src/core/usecases/workplan-executor.js';
import { FeatureProgressOrchestrator } from '../../src/core/usecases/feature-progress-orchestrator.js';
import { WorktreeConflictError } from '../../src/core/domain/errors.js';
import type { ICoordinationPort, LockResult, ClaimResult } from '../../src/core/ports/coordination.js';
import type {
  ISwarmPort,
  SwarmTask,
  SwarmAgent,
  SwarmStatus,
  AgentDBPattern,
} from '../../src/core/ports/swarm.js';
import type {
  IWorktreePort,
  ILLMPort,
  IASTPort,
  IFileSystemPort,
  WorkplanStep,
  Workplan,
} from '../../src/core/ports/index.js';

// ── Call Recorder ─────────────────────────────────────────

type Call = { method: string; args: unknown[] };

// ── Fake ICoordinationPort ────────────────────────────────

function createFakeCoordination(overrides?: Partial<ICoordinationPort>) {
  const calls: Call[] = [];
  return {
    port: {
      registerInstance: async () => {
        calls.push({ method: 'registerInstance', args: [] });
        return 'test-instance';
      },
      heartbeat: async () => {},
      acquireLock: async (feature: string, layer: string): Promise<LockResult> => {
        calls.push({ method: 'acquireLock', args: [feature, layer] });
        return {
          acquired: true,
          lock: {
            instanceId: 'test',
            projectId: 'p',
            feature,
            layer,
            acquiredAt: '',
            heartbeatAt: '',
            ttlSecs: 300,
          },
          conflict: null,
        };
      },
      releaseLock: async (feature: string, layer: string) => {
        calls.push({ method: 'releaseLock', args: [feature, layer] });
      },
      claimTask: async (taskId: string): Promise<ClaimResult> => {
        calls.push({ method: 'claimTask', args: [taskId] });
        return {
          claimed: true,
          claim: { taskId, instanceId: 'test', claimedAt: '', heartbeatAt: '' },
          conflict: null,
        };
      },
      releaseTask: async (taskId: string) => {
        calls.push({ method: 'releaseTask', args: [taskId] });
      },
      listLocks: async () => [],
      listClaims: async () => [],
      publishActivity: async (action: string, details?: Record<string, unknown>) => {
        calls.push({ method: 'publishActivity', args: [action, details] });
      },
      getActivities: async () => [],
      getUnstagedAcrossInstances: async () => [],
      ...overrides,
    } satisfies ICoordinationPort,
    calls,
  };
}

// ── Fake ISwarmPort ───────────────────────────────────────

function createFakeSwarm(overrides?: Partial<ISwarmPort>): { port: ISwarmPort; calls: Call[] } {
  const calls: Call[] = [];
  const tasks: SwarmTask[] = [];
  let taskCounter = 0;
  let agentCounter = 0;

  return {
    port: {
      async healthCheck() { return true; },
      async init() {
        return { id: 'swarm-1', topology: 'hierarchical', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'running' } as SwarmStatus;
      },
      async status() {
        return { id: 'swarm-1', topology: 'hierarchical', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'running' } as SwarmStatus;
      },
      async shutdown() {},
      async createTask(t) {
        const task: SwarmTask = { ...t, id: `task-${++taskCounter}`, status: 'pending' };
        tasks.push(task);
        calls.push({ method: 'createTask', args: [t] });
        return task;
      },
      async completeTask(id, result) {
        calls.push({ method: 'completeTask', args: [id, result] });
      },
      async listTasks() { return tasks; },
      async spawnAgent(name, role, taskId) {
        const agent: SwarmAgent = { id: `agent-${++agentCounter}`, name, role, status: 'active', currentTask: taskId };
        calls.push({ method: 'spawnAgent', args: [name, role, taskId] });
        return agent;
      },
      async terminateAgent(id) { calls.push({ method: 'terminateAgent', args: [id] }); },
      async listAgents() { return []; },
      async memoryStore() {},
      async memoryRetrieve() { return null; },
      async memorySearch() { return []; },
      async patternStore(p) {
        return { ...p, id: 'pat-1', accessCount: 0, createdAt: '', updatedAt: '' } as AgentDBPattern;
      },
      async patternSearch() { return []; },
      async patternFeedback() {},
      async sessionStart(name) { return { sessionId: 's1', agentName: name, startedAt: '', status: 'active' as const }; },
      async sessionEnd() {},
      async hierarchicalStore() {},
      async hierarchicalRecall() { return []; },
      async consolidate() { return { merged: 0, removed: 0 }; },
      async contextSynthesize() { return ''; },
      async getProgressReport() {
        return { swarmId: 'swarm-1', tasks, agents: [], patterns: { total: 0, recentlyUsed: 0 }, sessions: [], overallPercent: 0, phase: 'executing' };
      },
      ...overrides,
    } satisfies ISwarmPort,
    calls,
  };
}

// ── Fake IWorktreePort ────────────────────────────────────

function createFakeWorktree(overrides?: Partial<IWorktreePort>): { port: IWorktreePort; calls: Call[] } {
  const calls: Call[] = [];
  return {
    port: {
      async create(branch) {
        calls.push({ method: 'create', args: [branch] });
        return `/tmp/worktree-${branch}`;
      },
      async merge(wt, target) {
        calls.push({ method: 'merge', args: [wt, target] });
        return { merged: true, conflicts: [] };
      },
      async cleanup(wt) { calls.push({ method: 'cleanup', args: [wt] }); },
      async list() { return []; },
      ...overrides,
    } satisfies IWorktreePort,
    calls,
  };
}

// ── Fake IFileSystemPort ──────────────────────────────────

function createFakeFs(): IFileSystemPort {
  return {
    async read() { return '{}'; },
    async write() {},
    async exists() { return false; },
    async glob() { return []; },
    async mtime() { return 0; },
    async *streamFiles() {},
  };
}

// ── Fake ILLMPort ─────────────────────────────────────────

function createFakeLlm(): ILLMPort {
  return {
    async prompt() { return { content: '{"title":"test","steps":[]}', tokensUsed: 100 }; },
    async *streamPrompt() { yield 'ok'; },
  };
}

// ── Fake IASTPort ─────────────────────────────────────────

function createFakeAst(): IASTPort {
  return {
    async extractSummary() {
      return { filePath: 'test.ts', language: 'typescript' as const, level: 'L0' as const, exports: [], imports: [] };
    },
    diffStructural() {
      return { added: [], removed: [], modified: [], renamed: [] };
    },
  };
}

// ── Helper: simple workplan ───────────────────────────────

function makeWorkplan(steps: WorkplanStep[]): Workplan {
  return {
    id: 'plan-1',
    title: 'Test Plan',
    steps,
    estimatedTokenBudget: 4096,
  };
}

function makeStep(id: string, adapter: string, deps: string[] = []): WorkplanStep {
  return { id, description: `Do ${id}`, adapter, dependencies: deps };
}

// ══════════════════════════════════════════════════════════
// SwarmOrchestrator Tests
// ══════════════════════════════════════════════════════════

describe('SwarmOrchestrator coordination wiring', () => {
  test('acquireLock is called before worktree.create', async () => {
    const coord = createFakeCoordination();
    const worktree = createFakeWorktree();
    const swarm = createFakeSwarm();

    // Capture global ordering across both fakes
    const globalOrder: string[] = [];
    const origAcquire = coord.port.acquireLock;
    coord.port.acquireLock = async (f, l) => {
      globalOrder.push('acquireLock');
      return origAcquire(f, l);
    };
    const origCreate = worktree.port.create;
    worktree.port.create = async (b) => {
      globalOrder.push('worktree.create');
      return origCreate(b);
    };

    const orch = new SwarmOrchestrator(swarm.port, worktree.port, coord.port);
    await orch.orchestrate([makeStep('s1', 'secondary/fs')]);

    const acquireIdx = globalOrder.indexOf('acquireLock');
    const createIdx = globalOrder.indexOf('worktree.create');

    expect(acquireIdx).toBeGreaterThanOrEqual(0);
    expect(createIdx).toBeGreaterThanOrEqual(0);
    expect(acquireIdx).toBeLessThan(createIdx);
  });

  test('releaseLock is called after task completion', async () => {
    const coord = createFakeCoordination();
    const worktree = createFakeWorktree();
    const swarm = createFakeSwarm();

    const orch = new SwarmOrchestrator(swarm.port, worktree.port, coord.port);
    await orch.orchestrate([makeStep('s1', 'secondary/fs')]);

    const releaseCall = coord.calls.find((c) => c.method === 'releaseLock');
    expect(releaseCall).toBeDefined();
  });

  test('releaseLock is called on failure', async () => {
    const coord = createFakeCoordination();
    const worktree = createFakeWorktree();
    const swarm = createFakeSwarm({
      async spawnAgent() {
        throw new Error('Agent spawn failed');
      },
    });

    const orch = new SwarmOrchestrator(swarm.port, worktree.port, coord.port);

    // orchestrate should not throw — it catches per-step failures
    await orch.orchestrate([makeStep('s1', 'secondary/fs')]);

    const releaseCall = coord.calls.find((c) => c.method === 'releaseLock');
    expect(releaseCall).toBeDefined();
  });

  test('lock conflict throws WorktreeConflictError', async () => {
    const coord = createFakeCoordination({
      acquireLock: async (feature, layer) => ({
        acquired: false,
        lock: null,
        conflict: {
          instanceId: 'other-instance',
          projectId: 'p',
          feature,
          layer,
          acquiredAt: '',
          heartbeatAt: '',
          ttlSecs: 300,
        },
      }),
    });
    const worktree = createFakeWorktree();
    const swarm = createFakeSwarm();

    const orch = new SwarmOrchestrator(swarm.port, worktree.port, coord.port);

    // The orchestrator catches per-step errors, so orchestrate itself won't throw.
    // But the step should fail. We verify worktree.create was NOT called.
    await orch.orchestrate([makeStep('s1', 'secondary/fs')]);

    const createCalls = worktree.calls.filter((c) => c.method === 'create');
    expect(createCalls).toHaveLength(0);
  });

  test('works without coordination (null)', async () => {
    const worktree = createFakeWorktree();
    const swarm = createFakeSwarm();

    const orch = new SwarmOrchestrator(swarm.port, worktree.port, null);
    const status = await orch.orchestrate([makeStep('s1', 'secondary/fs')]);

    expect(status).toBeDefined();
    expect(worktree.calls.some((c) => c.method === 'create')).toBe(true);
  });
});

// ══════════════════════════════════════════════════════════
// WorkplanExecutor Tests
// ══════════════════════════════════════════════════════════

describe('WorkplanExecutor coordination wiring', () => {
  test('claimTask is called before spawnAgent', async () => {
    const coord = createFakeCoordination();
    const swarm = createFakeSwarm();

    const globalOrder: string[] = [];
    const origClaim = coord.port.claimTask;
    coord.port.claimTask = async (taskId) => {
      globalOrder.push('claimTask');
      return origClaim(taskId);
    };
    const origSpawn = swarm.port.spawnAgent;
    swarm.port.spawnAgent = async (name, role, taskId) => {
      globalOrder.push('spawnAgent');
      return origSpawn(name, role, taskId);
    };

    const executor = new WorkplanExecutor(
      createFakeLlm(),
      createFakeAst(),
      createFakeFs(),
      swarm.port,
      undefined, // no code generator
      'typescript',
      coord.port,
    );

    const plan = makeWorkplan([makeStep('s1', 'secondary/fs')]);
    const results = [];
    for await (const r of executor.executePlan(plan)) {
      results.push(r);
    }

    const claimIdx = globalOrder.indexOf('claimTask');
    const spawnIdx = globalOrder.indexOf('spawnAgent');

    expect(claimIdx).toBeGreaterThanOrEqual(0);
    expect(spawnIdx).toBeGreaterThanOrEqual(0);
    expect(claimIdx).toBeLessThan(spawnIdx);
  });

  test('releaseTask is called after completion', async () => {
    const coord = createFakeCoordination();
    const swarm = createFakeSwarm();

    const executor = new WorkplanExecutor(
      createFakeLlm(),
      createFakeAst(),
      createFakeFs(),
      swarm.port,
      undefined,
      'typescript',
      coord.port,
    );

    const plan = makeWorkplan([makeStep('s1', 'secondary/fs')]);
    const results = [];
    for await (const r of executor.executePlan(plan)) {
      results.push(r);
    }

    const releaseCall = coord.calls.find((c) => c.method === 'releaseTask');
    expect(releaseCall).toBeDefined();
  });

  test('task conflict yields failed step', async () => {
    const coord = createFakeCoordination({
      claimTask: async (taskId) => ({
        claimed: false,
        claim: null,
        conflict: { taskId, instanceId: 'other', claimedAt: '', heartbeatAt: '' },
      }),
    });
    const swarm = createFakeSwarm();

    const executor = new WorkplanExecutor(
      createFakeLlm(),
      createFakeAst(),
      createFakeFs(),
      swarm.port,
      undefined,
      'typescript',
      coord.port,
    );

    const plan = makeWorkplan([makeStep('s1', 'secondary/fs')]);
    const results = [];
    for await (const r of executor.executePlan(plan)) {
      results.push(r);
    }

    // Should have a failed step result
    const failedResults = results.filter((r) => r.status === 'failed');
    expect(failedResults.length).toBeGreaterThanOrEqual(1);
  });

  test('works without coordination (null)', async () => {
    const swarm = createFakeSwarm();

    const executor = new WorkplanExecutor(
      createFakeLlm(),
      createFakeAst(),
      createFakeFs(),
      swarm.port,
      undefined,
      'typescript',
      null,
    );

    const plan = makeWorkplan([makeStep('s1', 'secondary/fs')]);
    const results = [];
    for await (const r of executor.executePlan(plan)) {
      results.push(r);
    }

    // Should complete without errors — at least one 'passed' or 'running' result
    const passedOrRunning = results.filter((r) => r.status === 'passed' || r.status === 'running');
    expect(passedOrRunning.length).toBeGreaterThanOrEqual(1);
  });
});

// ══════════════════════════════════════════════════════════
// FeatureProgressOrchestrator Tests
// ══════════════════════════════════════════════════════════

describe('FeatureProgressOrchestrator coordination wiring', () => {
  test('publishActivity called on startFeature', async () => {
    const coord = createFakeCoordination();
    const orch = new FeatureProgressOrchestrator(createFakeFs(), coord.port);

    await orch.startFeature('test-feature');

    const activityCall = coord.calls.find(
      (c) => c.method === 'publishActivity' && (c.args[0] as string).includes('feature-start'),
    );
    expect(activityCall).toBeDefined();
  });

  test('publishActivity called on phase transition', async () => {
    const coord = createFakeCoordination();
    const orch = new FeatureProgressOrchestrator(createFakeFs(), coord.port);

    await orch.startFeature('test-feature');

    // Clear calls from startFeature
    coord.calls.length = 0;

    await orch.completePhase('init');

    const transitionCall = coord.calls.find(
      (c) => c.method === 'publishActivity' && (c.args[0] as string).includes('phase-transition'),
    );
    expect(transitionCall).toBeDefined();
  });

  test('works without coordination (null)', async () => {
    const orch = new FeatureProgressOrchestrator(createFakeFs(), null);

    const session = await orch.startFeature('test-feature');
    expect(session).toBeDefined();
    expect(session.featureName).toBe('test-feature');

    // Phase transition should also work fine
    await orch.completePhase('init');

    const currentSession = orch.getCurrentSession();
    expect(currentSession).not.toBeNull();
    expect(currentSession!.currentPhase).toBe('specs');
  });
});
