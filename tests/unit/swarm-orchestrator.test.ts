import { describe, it, expect } from 'bun:test';
import { SwarmOrchestrator } from '../../src/core/usecases/swarm-orchestrator.js';
import type { ISwarmPort, SwarmConfig, SwarmTask, SwarmAgent, SwarmStatus } from '../../src/core/ports/swarm.js';
import type { IWorktreePort, WorktreePath } from '../../src/core/ports/index.js';
import type { WorkplanStep } from '../../src/core/ports/index.js';

// ── Mock Factories ─────────────────────────────────────

function mockSwarm(): ISwarmPort {
  const tasks: SwarmTask[] = [];
  const agents: SwarmAgent[] = [];
  let taskCounter = 0;
  let agentCounter = 0;

  return {
    async init() {
      return { id: 'swarm-1', topology: 'hierarchical', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'running' } as SwarmStatus;
    },
    async status() {
      return { id: 'swarm-1', topology: 'hierarchical', agentCount: agents.length, activeTaskCount: tasks.filter((t) => t.status === 'running').length, completedTaskCount: tasks.filter((t) => t.status === 'completed').length, status: 'running' } as SwarmStatus;
    },
    async shutdown() {},
    async createTask(t) {
      const task: SwarmTask = { ...t, id: `task-${++taskCounter}`, status: 'pending' };
      tasks.push(task);
      return task;
    },
    async completeTask(id, result, commitHash) {
      const task = tasks.find((t) => t.id === id);
      if (task) { task.status = 'completed'; task.result = result; task.commitHash = commitHash; }
    },
    async listTasks() { return tasks; },
    async spawnAgent(name, role, taskId) {
      const agent: SwarmAgent = { id: `agent-${++agentCounter}`, name, role, status: 'active', currentTask: taskId };
      agents.push(agent);
      return agent;
    },
    async terminateAgent(id) {
      const agent = agents.find((a) => a.id === id);
      if (agent) agent.status = 'terminated';
    },
    async listAgents() { return agents; },
    async memoryStore() {},
    async memoryRetrieve() { return null; },
    async memorySearch() { return []; },
    async patternStore(p) { return { ...p, id: 'pat-1', accessCount: 0, createdAt: '', updatedAt: '' }; },
    async patternSearch() { return []; },
    async patternFeedback() {},
    async sessionStart(name) { return { sessionId: 's1', agentName: name, startedAt: '', status: 'active' }; },
    async sessionEnd() {},
    async hierarchicalStore() {},
    async hierarchicalRecall() { return []; },
    async consolidate() { return { merged: 0, removed: 0 }; },
    async contextSynthesize() { return ''; },
    async getProgressReport() {
      return { swarmId: 'swarm-1', tasks, agents, patterns: { total: 0, recentlyUsed: 0 }, sessions: [], overallPercent: 0, phase: 'executing' };
    },
  };
}

function mockWorktree(): IWorktreePort {
  return {
    async create(branch) { return `/tmp/worktrees/${branch}` as WorktreePath; },
    async merge() { return { success: true, conflicts: [] }; },
    async cleanup() {},
    async list() { return []; },
  };
}

// ── Tests ──────────────────────────────────────────────

describe('SwarmOrchestrator', () => {
  it('orchestrates independent steps in parallel', async () => {
    const swarm = mockSwarm();
    const orchestrator = new SwarmOrchestrator(swarm, mockWorktree());

    const steps: WorkplanStep[] = [
      { id: 's1', description: 'Build HTTP adapter', adapter: 'primary/http', dependencies: [] },
      { id: 's2', description: 'Build DB adapter', adapter: 'secondary/db', dependencies: [] },
    ];

    const status = await orchestrator.orchestrate(steps);
    expect(status.status).toBe('running');

    const tasks = await swarm.listTasks();
    expect(tasks).toHaveLength(2);
    expect(tasks.every((t) => t.status === 'completed')).toBe(true);
  });

  it('respects dependency ordering', async () => {
    const swarm = mockSwarm();
    const completionOrder: string[] = [];
    const origComplete = swarm.completeTask.bind(swarm);
    swarm.completeTask = async (id, result, hash) => {
      completionOrder.push(id);
      return origComplete(id, result, hash);
    };

    const orchestrator = new SwarmOrchestrator(swarm, mockWorktree());

    const steps: WorkplanStep[] = [
      { id: 's1', description: 'Define ports', adapter: 'ports', dependencies: [] },
      { id: 's2', description: 'Implement adapter', adapter: 'secondary/db', dependencies: ['s1'] },
    ];

    await orchestrator.orchestrate(steps);

    // s1 must complete before s2 starts
    expect(completionOrder.indexOf('task-1')).toBeLessThan(completionOrder.indexOf('task-2'));
  });

  it('limits concurrency to maxAgents', async () => {
    const swarm = mockSwarm();
    let maxConcurrent = 0;
    let current = 0;

    const origSpawn = swarm.spawnAgent.bind(swarm);
    swarm.spawnAgent = async (name, role, taskId) => {
      current++;
      maxConcurrent = Math.max(maxConcurrent, current);
      const result = await origSpawn(name, role, taskId);
      return result;
    };
    const origTerminate = swarm.terminateAgent.bind(swarm);
    swarm.terminateAgent = async (id) => {
      current--;
      return origTerminate(id);
    };

    const orchestrator = new SwarmOrchestrator(swarm, mockWorktree());

    const steps: WorkplanStep[] = Array.from({ length: 8 }, (_, i) => ({
      id: `s${i}`, description: `Step ${i}`, adapter: 'coder', dependencies: [],
    }));

    await orchestrator.orchestrate(steps, { maxAgents: 2 });

    // Should never exceed maxAgents concurrent
    expect(maxConcurrent).toBeLessThanOrEqual(2);
  });

  it('getProgress returns progress report', async () => {
    const orchestrator = new SwarmOrchestrator(mockSwarm(), mockWorktree());
    const report = await orchestrator.getProgress();
    expect(report.swarmId).toBe('swarm-1');
  });

  it('infers coder role for generic adapters', async () => {
    const swarm = mockSwarm();
    const orchestrator = new SwarmOrchestrator(swarm, mockWorktree());

    await orchestrator.orchestrate([
      { id: 's1', description: 'Build something', adapter: 'secondary/redis', dependencies: [] },
    ]);

    const agents = await swarm.listAgents();
    expect(agents[0].role).toBe('coder');
  });

  it('infers tester role for test-related adapters', async () => {
    const swarm = mockSwarm();
    const orchestrator = new SwarmOrchestrator(swarm, mockWorktree());

    await orchestrator.orchestrate([
      { id: 's1', description: 'Write tests', adapter: 'tests/unit', dependencies: [] },
    ]);

    const agents = await swarm.listAgents();
    expect(agents[0].role).toBe('tester');
  });
});
