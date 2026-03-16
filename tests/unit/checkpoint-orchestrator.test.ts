import { describe, it, expect, beforeEach, mock } from 'bun:test';
import { CheckpointOrchestrator } from '../../src/core/usecases/checkpoint-orchestrator.js';
import type { ICheckpointPort } from '../../src/core/ports/checkpoint.js';
import type { ISwarmPort, SwarmTask, SwarmStatus } from '../../src/core/ports/swarm.js';
import type { CheckpointEntry, TaskSnapshot } from '../../src/core/domain/checkpoint-types.js';

// ─── Factory Helpers ────────────────────────────────────────

function makeTaskSnapshot(overrides?: Partial<TaskSnapshot>): TaskSnapshot {
  return {
    taskId: 'task-1',
    title: 'feat/auth/implement login',
    status: 'running',
    agentRole: 'coder',
    snapshotAt: '2025-06-01T00:00:00.000Z',
    ...overrides,
  };
}

function makeCheckpointEntry(overrides?: Partial<CheckpointEntry>): CheckpointEntry {
  return {
    id: 'ckpt-1',
    projectId: 'test-project',
    projectPath: '/tmp/test-project',
    createdAt: '2025-06-01T00:00:00.000Z',
    swarmStatus: { topology: 'hierarchical', agentCount: 3, status: 'running' },
    features: [],
    orphanTasks: [makeTaskSnapshot()],
    ...overrides,
  };
}

function makeSwarmTask(overrides?: Partial<SwarmTask>): SwarmTask {
  return {
    id: 'task-1',
    title: 'feat/auth/implement login',
    agentRole: 'coder',
    status: 'running',
    ...overrides,
  };
}

function makeSwarmStatus(overrides?: Partial<SwarmStatus>): SwarmStatus {
  return {
    id: 'swarm-1',
    topology: 'hierarchical',
    agentCount: 2,
    activeTaskCount: 1,
    completedTaskCount: 0,
    status: 'running',
    ...overrides,
  };
}

// ─── Mock Builders ──────────────────────────────────────────

function makeMockCheckpointPort(): ICheckpointPort {
  return {
    checkpoint: mock(() => Promise.resolve()),
    recover: mock(() => Promise.resolve(null)),
    list: mock(() => Promise.resolve([])),
    prune: mock(() => Promise.resolve(0)),
  };
}

function makeMockSwarmPort(): Pick<ISwarmPort, 'status' | 'listTasks'> & Record<string, any> {
  return {
    status: mock(() => Promise.resolve(makeSwarmStatus())),
    listTasks: mock(() => Promise.resolve([] as SwarmTask[])),
    // Stubs for remaining ISwarmPort methods (not used by orchestrator)
    init: mock(() => Promise.resolve(makeSwarmStatus())),
    shutdown: mock(() => Promise.resolve()),
    createTask: mock(() => Promise.resolve(makeSwarmTask())),
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
    hierarchicalStore: mock(() => Promise.resolve()),
    hierarchicalRecall: mock(() => Promise.resolve([])),
    consolidate: mock(() => Promise.resolve({ merged: 0, removed: 0 })),
    contextSynthesize: mock(() => Promise.resolve('')),
    getProgressReport: mock(() => Promise.resolve({ swarmId: '', tasks: [], agents: [], patterns: { total: 0, recentlyUsed: 0 }, sessions: [], overallPercent: 0, phase: '' })),
  };
}

// ─── Tests ──────────────────────────────────────────────────

describe('CheckpointOrchestrator', () => {
  let ckptPort: ICheckpointPort;
  let swarmPort: ISwarmPort;
  let orchestrator: CheckpointOrchestrator;

  beforeEach(() => {
    ckptPort = makeMockCheckpointPort();
    swarmPort = makeMockSwarmPort() as unknown as ISwarmPort;
    orchestrator = new CheckpointOrchestrator(ckptPort, swarmPort, 'test-project', '/tmp/test-project');
  });

  describe('manualCheckpoint', () => {
    it('builds a valid CheckpointEntry from swarm state', async () => {
      const tasks: SwarmTask[] = [
        makeSwarmTask({ id: 't1', title: 'feat/auth/login', status: 'running', agentRole: 'coder' }),
        makeSwarmTask({ id: 't2', title: 'standalone task', status: 'pending', agentRole: 'planner' }),
      ];
      (swarmPort.listTasks as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(tasks));
      (swarmPort.status as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(makeSwarmStatus({ agentCount: 4 })));

      const entry = await orchestrator.manualCheckpoint();

      expect(entry.projectId).toBe('test-project');
      expect(entry.projectPath).toBe('/tmp/test-project');
      expect(entry.id).toBeTruthy();
      expect(entry.createdAt).toBeTruthy();
      expect(entry.swarmStatus.agentCount).toBe(4);
      expect(entry.swarmStatus.topology).toBe('hierarchical');
      // feat/auth task should be grouped into a feature
      expect(entry.features.length).toBe(1);
      expect(entry.features[0].featureId).toBe('auth');
      // standalone task has no feature prefix → orphan
      expect(entry.orphanTasks.length).toBe(1);
      expect(entry.orphanTasks[0].taskId).toBe('t2');
    });

    it('persists entry via checkpoint port', async () => {
      (swarmPort.listTasks as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([]));

      await orchestrator.manualCheckpoint();

      expect(ckptPort.checkpoint).toHaveBeenCalledTimes(1);
    });
  });

  describe('recover', () => {
    it('delegates to checkpoint port and returns result', async () => {
      const stored = makeCheckpointEntry();
      (ckptPort.recover as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(stored));

      const result = await orchestrator.recover();

      expect(ckptPort.recover).toHaveBeenCalledWith('test-project');
      expect(result).toEqual(stored);
    });

    it('returns null when no checkpoint exists', async () => {
      (ckptPort.recover as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(null));

      const result = await orchestrator.recover();
      expect(result).toBeNull();
    });
  });

  describe('onTaskTransition', () => {
    it('auto-checkpoints on task transition', async () => {
      (swarmPort.listTasks as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([]));

      await orchestrator.onTaskTransition('t1', 'completed');

      expect(ckptPort.checkpoint).toHaveBeenCalledTimes(1);
    });

    it('does not throw when checkpoint port fails', async () => {
      (swarmPort.status as ReturnType<typeof mock>).mockReturnValue(Promise.reject(new Error('swarm down')));

      // Should not throw
      await orchestrator.onTaskTransition('t1', 'failed');
    });
  });

  describe('pruneOld', () => {
    it('delegates with default keepCount of 20', async () => {
      (ckptPort.prune as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(5));

      const deleted = await orchestrator.pruneOld();

      expect(ckptPort.prune).toHaveBeenCalledWith('test-project', 20);
      expect(deleted).toBe(5);
    });

    it('delegates with custom keepCount', async () => {
      (ckptPort.prune as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(2));

      const deleted = await orchestrator.pruneOld(5);

      expect(ckptPort.prune).toHaveBeenCalledWith('test-project', 5);
      expect(deleted).toBe(2);
    });
  });

  describe('buildFeatureMap (via manualCheckpoint)', () => {
    it('groups tasks by feat/ prefix correctly', async () => {
      const tasks: SwarmTask[] = [
        makeSwarmTask({ id: 't1', title: 'feat/dashboard/build UI', agentRole: 'coder', status: 'running' }),
        makeSwarmTask({ id: 't2', title: 'feat/dashboard/add tests', agentRole: 'tester', status: 'completed' }),
        makeSwarmTask({ id: 't3', title: 'feat/auth/login port', agentRole: 'coder', status: 'pending' }),
      ];
      (swarmPort.listTasks as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(tasks));

      const entry = await orchestrator.manualCheckpoint();

      expect(entry.features.length).toBe(2);
      const dashboard = entry.features.find((f) => f.featureId === 'dashboard');
      const auth = entry.features.find((f) => f.featureId === 'auth');
      expect(dashboard).toBeDefined();
      expect(dashboard!.taskSnapshots.length).toBe(2);
      expect(dashboard!.completedSteps).toBe(1);
      expect(auth).toBeDefined();
      expect(auth!.taskSnapshots.length).toBe(1);
    });

    it('groups tasks by [bracket] prefix', async () => {
      const tasks: SwarmTask[] = [
        makeSwarmTask({ id: 't1', title: '[payments] stripe adapter', agentRole: 'coder', status: 'running' }),
        makeSwarmTask({ id: 't2', title: '[payments] port interface', agentRole: 'coder', status: 'completed' }),
      ];
      (swarmPort.listTasks as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(tasks));

      const entry = await orchestrator.manualCheckpoint();

      expect(entry.features.length).toBe(1);
      expect(entry.features[0].featureId).toBe('payments');
      expect(entry.features[0].totalSteps).toBe(2);
    });

    it('groups tasks by worktree branch when title has no prefix', async () => {
      const tasks: SwarmTask[] = [
        makeSwarmTask({ id: 't1', title: 'implement caching', agentRole: 'coder', status: 'running', worktreeBranch: 'feat/cache/secondary' }),
      ];
      (swarmPort.listTasks as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(tasks));

      const entry = await orchestrator.manualCheckpoint();

      expect(entry.features.length).toBe(1);
      expect(entry.features[0].featureId).toBe('cache');
      expect(entry.orphanTasks.length).toBe(0);
    });

    it('leaves unrecognized tasks as orphans', async () => {
      const tasks: SwarmTask[] = [
        makeSwarmTask({ id: 't1', title: 'random maintenance task', agentRole: 'coder', status: 'running' }),
      ];
      (swarmPort.listTasks as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(tasks));

      const entry = await orchestrator.manualCheckpoint();

      expect(entry.features.length).toBe(0);
      expect(entry.orphanTasks.length).toBe(1);
    });
  });
});
