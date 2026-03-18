/**
 * Integration Tests: Checkpoint Lifecycle
 *
 * Exercises the full checkpoint/recover pipeline using a real
 * FileCheckpointAdapter backed by a temp directory on disk.
 * ISwarmPort is stubbed via dependency injection (no mock.module).
 */
import { describe, it, expect, afterEach, mock } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { FileCheckpointAdapter } from '../../src/adapters/secondary/file-checkpoint-adapter.js';
import { FileSystemAdapter } from '../../src/adapters/secondary/filesystem-adapter.js';
import { CheckpointOrchestrator } from '../../src/core/usecases/checkpoint-orchestrator.js';
import type { ISwarmPort, SwarmTask, SwarmStatus } from '../../src/core/ports/swarm.js';
import type { CheckpointEntry, TaskSnapshot, FeatureProgress } from '../../src/core/domain/checkpoint-types.js';

// ─── Factory Helpers ────────────────────────────────────────

function makeSwarmStatus(overrides?: Partial<SwarmStatus>): SwarmStatus {
  return {
    id: 'swarm-1',
    topology: 'hierarchical',
    agentCount: 3,
    activeTaskCount: 2,
    completedTaskCount: 1,
    status: 'running',
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

function makeMockSwarmPort(tasks: SwarmTask[] = [], status?: Partial<SwarmStatus>): ISwarmPort {
  return {
    status: mock(() => Promise.resolve(makeSwarmStatus(status))),
    listTasks: mock(() => Promise.resolve(tasks)),
    healthCheck: mock(() => Promise.resolve(true)),
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
  } as unknown as ISwarmPort;
}

function makeCheckpointEntry(overrides?: Partial<CheckpointEntry>): CheckpointEntry {
  return {
    id: crypto.randomUUID(),
    projectId: 'test-project',
    projectPath: '/tmp/test-project',
    createdAt: new Date().toISOString(),
    swarmStatus: { topology: 'hierarchical', agentCount: 3, status: 'running' },
    features: [
      {
        featureId: 'auth',
        title: 'auth',
        phase: 'code',
        totalSteps: 3,
        completedSteps: 1,
        failedSteps: 0,
        startedAt: '2025-06-01T10:00:00.000Z',
        updatedAt: '2025-06-01T12:00:00.000Z',
        taskSnapshots: [
          {
            taskId: 't1',
            title: 'feat/auth/login port',
            status: 'completed',
            assignee: 'agent-1',
            agentRole: 'coder',
            adapter: 'secondary',
            worktreeBranch: 'feat/auth/secondary',
            result: 'implemented',
            commitHash: 'abc1234',
            snapshotAt: '2025-06-01T12:00:00.000Z',
          },
        ],
      },
    ],
    orphanTasks: [
      {
        taskId: 'orphan-1',
        title: 'random cleanup',
        status: 'pending',
        agentRole: 'coder',
        snapshotAt: '2025-06-01T12:00:00.000Z',
      },
    ],
    ...overrides,
  };
}

// ─── Temp Directory Management ──────────────────────────────

const tempDirs: string[] = [];

async function createTempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'hex-ckpt-test-'));
  tempDirs.push(dir);
  return dir;
}

afterEach(async () => {
  for (const dir of tempDirs) {
    try {
      await rm(dir, { recursive: true, force: true });
    } catch {
      // best effort cleanup
    }
  }
  tempDirs.length = 0;
});

// ─── Tests ──────────────────────────────────────────────────

describe('Checkpoint lifecycle (integration)', () => {
  it('checkpoint/recover round-trip preserves all fields', async () => {
    const tmpDir = await createTempDir();
    const fs = new FileSystemAdapter(tmpDir);
    const adapter = new FileCheckpointAdapter('.', fs);

    const tasks: SwarmTask[] = [
      makeSwarmTask({ id: 't1', title: 'feat/auth/login port', status: 'completed', agentRole: 'coder', assignee: 'agent-1', adapter: 'secondary', worktreeBranch: 'feat/auth/secondary', result: 'done', commitHash: 'abc1234' }),
      makeSwarmTask({ id: 't2', title: 'feat/auth/add tests', status: 'running', agentRole: 'tester', assignee: 'agent-2' }),
      makeSwarmTask({ id: 't3', title: 'standalone maintenance', status: 'pending', agentRole: 'planner' }),
    ];
    const swarm = makeMockSwarmPort(tasks, { agentCount: 5, topology: 'hierarchical' });
    const orchestrator = new CheckpointOrchestrator(adapter, swarm, 'test-project', '/tmp/test-project');

    // Checkpoint
    const entry = await orchestrator.manualCheckpoint();
    expect(entry.projectId).toBe('test-project');
    expect(entry.projectPath).toBe('/tmp/test-project');
    expect(entry.swarmStatus.agentCount).toBe(5);
    expect(entry.swarmStatus.topology).toBe('hierarchical');

    // Recover
    const recovered = await orchestrator.recover();
    expect(recovered).not.toBeNull();
    expect(recovered!.id).toBe(entry.id);
    expect(recovered!.projectId).toBe(entry.projectId);
    expect(recovered!.projectPath).toBe(entry.projectPath);
    expect(recovered!.createdAt).toBe(entry.createdAt);
    expect(recovered!.swarmStatus).toEqual(entry.swarmStatus);

    // Verify features are preserved
    expect(recovered!.features.length).toBe(entry.features.length);
    const recoveredFeature = recovered!.features[0];
    const originalFeature = entry.features[0];
    expect(recoveredFeature.featureId).toBe(originalFeature.featureId);
    expect(recoveredFeature.phase).toBe(originalFeature.phase);
    expect(recoveredFeature.totalSteps).toBe(originalFeature.totalSteps);
    expect(recoveredFeature.completedSteps).toBe(originalFeature.completedSteps);
    expect(recoveredFeature.failedSteps).toBe(originalFeature.failedSteps);
    expect(recoveredFeature.startedAt).toBe(originalFeature.startedAt);
    expect(recoveredFeature.updatedAt).toBe(originalFeature.updatedAt);

    // Verify task snapshots inside features
    expect(recoveredFeature.taskSnapshots.length).toBe(originalFeature.taskSnapshots.length);
    for (let i = 0; i < recoveredFeature.taskSnapshots.length; i++) {
      const rs = recoveredFeature.taskSnapshots[i];
      const os = originalFeature.taskSnapshots[i];
      expect(rs.taskId).toBe(os.taskId);
      expect(rs.title).toBe(os.title);
      expect(rs.status).toBe(os.status);
      expect(rs.assignee).toBe(os.assignee);
      expect(rs.agentRole).toBe(os.agentRole);
      expect(rs.adapter).toBe(os.adapter);
      expect(rs.worktreeBranch).toBe(os.worktreeBranch);
      expect(rs.result).toBe(os.result);
      expect(rs.commitHash).toBe(os.commitHash);
      expect(rs.snapshotAt).toBe(os.snapshotAt);
    }

    // Verify orphan tasks
    expect(recovered!.orphanTasks.length).toBe(entry.orphanTasks.length);
    for (let i = 0; i < recovered!.orphanTasks.length; i++) {
      expect(recovered!.orphanTasks[i].taskId).toBe(entry.orphanTasks[i].taskId);
      expect(recovered!.orphanTasks[i].title).toBe(entry.orphanTasks[i].title);
      expect(recovered!.orphanTasks[i].status).toBe(entry.orphanTasks[i].status);
    }
  });

  it('prune keeps only N most recent checkpoints', async () => {
    const tmpDir = await createTempDir();
    const fs = new FileSystemAdapter(tmpDir);
    const adapter = new FileCheckpointAdapter('.', fs);

    // Create 10 checkpoints with staggered timestamps
    for (let i = 0; i < 10; i++) {
      const hour = String(i).padStart(2, '0');
      const entry = makeCheckpointEntry({
        id: `ckpt-${i}`,
        projectId: 'prune-test',
        createdAt: `2025-06-01T${hour}:00:00.000Z`,
      });
      await adapter.checkpoint(entry);
    }

    // Verify all 10 exist
    const beforePrune = await adapter.list('prune-test');
    expect(beforePrune.length).toBe(10);

    // Prune, keeping only 3
    const deleted = await adapter.prune('prune-test', 3);
    expect(deleted).toBe(7);

    // Verify exactly 3 remain
    const afterPrune = await adapter.list('prune-test');
    expect(afterPrune.length).toBe(3);

    // Verify the 3 remaining are the most recent (hours 07, 08, 09)
    const remainingIds = afterPrune.map((e) => e.id);
    expect(remainingIds).toContain('ckpt-7');
    expect(remainingIds).toContain('ckpt-8');
    expect(remainingIds).toContain('ckpt-9');

    // Verify order is newest-first
    expect(afterPrune[0].id).toBe('ckpt-9');
    expect(afterPrune[1].id).toBe('ckpt-8');
    expect(afterPrune[2].id).toBe('ckpt-7');
  });

  it('recover returns null when directory is empty', async () => {
    const tmpDir = await createTempDir();
    const fs = new FileSystemAdapter(tmpDir);
    const adapter = new FileCheckpointAdapter('.', fs);

    const result = await adapter.recover('nonexistent-project');
    expect(result).toBeNull();
  });

  it('crash simulation — new instance recovers previous checkpoint', async () => {
    const tmpDir = await createTempDir();

    // Instance 1: create checkpoint and "crash"
    const tasks: SwarmTask[] = [
      makeSwarmTask({ id: 'crash-t1', title: 'feat/payments/stripe', status: 'running', agentRole: 'coder', assignee: 'worker-1' }),
      makeSwarmTask({ id: 'crash-t2', title: '[payments] port', status: 'completed', agentRole: 'coder', result: 'done', commitHash: 'def5678' }),
    ];
    {
      const fs1 = new FileSystemAdapter(tmpDir);
      const adapter1 = new FileCheckpointAdapter('.', fs1);
      const swarm1 = makeMockSwarmPort(tasks, { agentCount: 2, status: 'running' });
      const orchestrator1 = new CheckpointOrchestrator(adapter1, swarm1, 'crash-project', '/opt/crash-project');

      const written = await orchestrator1.manualCheckpoint();
      expect(written.projectId).toBe('crash-project');
      // orchestrator1 goes out of scope — simulating crash
    }

    // Instance 2: brand new adapter + orchestrator (simulating restart)
    {
      const fs2 = new FileSystemAdapter(tmpDir);
      const adapter2 = new FileCheckpointAdapter('.', fs2);
      const freshSwarm = makeMockSwarmPort([]); // empty swarm — process just restarted
      const orchestrator2 = new CheckpointOrchestrator(adapter2, freshSwarm, 'crash-project', '/opt/crash-project');

      const recovered = await orchestrator2.recover();
      expect(recovered).not.toBeNull();
      expect(recovered!.projectId).toBe('crash-project');
      expect(recovered!.projectPath).toBe('/opt/crash-project');
      expect(recovered!.swarmStatus.agentCount).toBe(2);
      expect(recovered!.swarmStatus.status).toBe('running');

      // Verify feature data survived the "crash"
      expect(recovered!.features.length).toBe(1);
      expect(recovered!.features[0].featureId).toBe('payments');
      expect(recovered!.features[0].taskSnapshots.length).toBe(2);

      // Verify individual task snapshots
      const snap1 = recovered!.features[0].taskSnapshots.find((s) => s.taskId === 'crash-t1');
      expect(snap1).toBeDefined();
      expect(snap1!.status).toBe('running');
      expect(snap1!.assignee).toBe('worker-1');

      const snap2 = recovered!.features[0].taskSnapshots.find((s) => s.taskId === 'crash-t2');
      expect(snap2).toBeDefined();
      expect(snap2!.status).toBe('completed');
      expect(snap2!.commitHash).toBe('def5678');
    }
  });

  it('CheckpointEntry round-trips through JSON', () => {
    const entry = makeCheckpointEntry({
      id: 'json-rt-1',
      projectId: 'json-test',
      projectPath: '/home/user/project',
      createdAt: '2025-06-15T08:30:00.000Z',
      swarmStatus: { topology: 'mesh', agentCount: 7, status: 'idle' },
      features: [
        {
          featureId: 'search',
          title: 'search',
          phase: 'validate',
          totalSteps: 5,
          completedSteps: 4,
          failedSteps: 1,
          startedAt: '2025-06-14T00:00:00.000Z',
          updatedAt: '2025-06-15T08:30:00.000Z',
          taskSnapshots: [
            {
              taskId: 'st-1',
              title: 'feat/search/index adapter',
              status: 'completed',
              assignee: 'agent-x',
              agentRole: 'coder',
              adapter: 'secondary',
              worktreeBranch: 'feat/search/secondary',
              result: 'indexed 1000 docs',
              commitHash: 'aaa1111',
              snapshotAt: '2025-06-15T08:30:00.000Z',
            },
            {
              taskId: 'st-2',
              title: 'feat/search/query port',
              status: 'failed',
              agentRole: 'coder',
              snapshotAt: '2025-06-15T08:30:00.000Z',
            },
          ],
        },
      ],
      orphanTasks: [
        {
          taskId: 'ot-1',
          title: 'migrate config',
          status: 'pending',
          agentRole: 'planner',
          snapshotAt: '2025-06-15T08:30:00.000Z',
        },
      ],
    });

    const json = JSON.stringify(entry);
    const parsed = JSON.parse(json) as CheckpointEntry;

    // Top-level fields
    expect(parsed.id).toBe(entry.id);
    expect(parsed.projectId).toBe(entry.projectId);
    expect(parsed.projectPath).toBe(entry.projectPath);
    expect(parsed.createdAt).toBe(entry.createdAt);
    expect(parsed.swarmStatus).toEqual(entry.swarmStatus);

    // Features
    expect(parsed.features.length).toBe(1);
    const pf = parsed.features[0];
    const ef = entry.features[0];
    expect(pf.featureId).toBe(ef.featureId);
    expect(pf.title).toBe(ef.title);
    expect(pf.phase).toBe(ef.phase);
    expect(pf.totalSteps).toBe(ef.totalSteps);
    expect(pf.completedSteps).toBe(ef.completedSteps);
    expect(pf.failedSteps).toBe(ef.failedSteps);
    expect(pf.startedAt).toBe(ef.startedAt);
    expect(pf.updatedAt).toBe(ef.updatedAt);

    // Task snapshots within features
    expect(pf.taskSnapshots.length).toBe(2);
    expect(pf.taskSnapshots[0]).toEqual(ef.taskSnapshots[0]);
    expect(pf.taskSnapshots[1]).toEqual(ef.taskSnapshots[1]);

    // Orphan tasks
    expect(parsed.orphanTasks.length).toBe(1);
    expect(parsed.orphanTasks[0]).toEqual(entry.orphanTasks[0]);

    // Optional fields that are undefined should not appear in JSON
    expect(pf.taskSnapshots[1].assignee).toBeUndefined();
    expect(pf.taskSnapshots[1].adapter).toBeUndefined();
    expect(pf.taskSnapshots[1].worktreeBranch).toBeUndefined();
    expect(pf.taskSnapshots[1].result).toBeUndefined();
    expect(pf.taskSnapshots[1].commitHash).toBeUndefined();
  });
});
