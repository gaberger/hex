/**
 * Swarm Orchestrator Use Case — implements ISwarmOrchestrationPort.
 *
 * Coordinates parallel code generation across worktrees using swarm agents.
 * Takes a workplan's steps, assigns each to an agent in an isolated worktree,
 * and tracks progress through the swarm port (ruflo).
 *
 * Respects dependency ordering: steps only start after their dependencies complete.
 */

import type { WorkplanStep } from '../ports/index.js';
import type {
  ISwarmOrchestrationPort,
  ISwarmPort,
  SwarmConfig,
  SwarmStatus,
  SwarmTask,
  AgentDBProgressReport,
  AgentRole,
} from '../ports/swarm.js';
import type { IWorktreePort, WorktreePath } from '../ports/index.js';
import type { ICoordinationPort } from '../ports/coordination.js';
import { WorktreeConflictError } from '../domain/errors.js';

const DEFAULT_CONFIG: SwarmConfig = {
  topology: 'hierarchical',
  maxAgents: 4,
  strategy: 'specialized',
  consensus: 'raft',
  memoryNamespace: 'hex',
};

/** Map workplan step adapter names to swarm agent roles */
function inferRole(step: WorkplanStep): AgentRole {
  const adapter = (step.adapter ?? '').toLowerCase();
  if (adapter.includes('test')) return 'tester';
  if (adapter.includes('review') || adapter.includes('validate')) return 'reviewer';
  if (adapter.includes('plan')) return 'planner';
  if (adapter.includes('integrat')) return 'integrator';
  return 'coder';
}

export class SwarmOrchestrator implements ISwarmOrchestrationPort {
  constructor(
    private readonly swarm: ISwarmPort,
    private readonly worktree: IWorktreePort,
    private readonly coordination: ICoordinationPort | null = null,
  ) {}

  async orchestrate(
    steps: WorkplanStep[],
    config?: Partial<SwarmConfig>,
  ): Promise<SwarmStatus> {
    const mergedConfig = { ...DEFAULT_CONFIG, ...config };

    // Initialize the swarm
    await this.swarm.init(mergedConfig);

    // Register all steps as tasks
    const taskMap = new Map<string, SwarmTask>();
    for (const step of steps) {
      const task = await this.swarm.createTask({
        title: step.description,
        agentRole: inferRole(step),
        adapter: step.adapter,
        language: undefined,
      });
      taskMap.set(step.id, task);
    }

    // Build dependency graph for execution ordering
    const completed = new Set<string>();
    const failed = new Set<string>();
    const pending = new Set(steps.map((s) => s.id));

    // Execute in waves: each wave processes steps whose deps are all complete
    while (pending.size > 0 && failed.size === 0) {
      const ready = steps.filter(
        (s) => pending.has(s.id) && s.dependencies.every((d) => completed.has(d)),
      );

      if (ready.length === 0 && pending.size > 0) {
        // Deadlock — remaining steps have unresolvable dependencies
        break;
      }

      // Limit concurrency to maxAgents
      const batch = ready.slice(0, mergedConfig.maxAgents);

      // Execute batch in parallel, each in its own worktree
      const results = await Promise.allSettled(
        batch.map((step) => this.executeStep(step, taskMap.get(step.id)!)),
      );

      for (let i = 0; i < batch.length; i++) {
        const stepId = batch[i].id;
        const result = results[i];
        pending.delete(stepId);

        if (result.status === 'fulfilled') {
          completed.add(stepId);
        } else {
          failed.add(stepId);
        }
      }
    }

    return this.swarm.status();
  }

  async getProgress(): Promise<AgentDBProgressReport> {
    return this.swarm.getProgressReport();
  }

  private async executeStep(step: WorkplanStep, task: SwarmTask): Promise<void> {
    const branchName = `hex/${task.id}-${step.adapter ?? 'main'}`;

    // Acquire coordination lock before creating worktree
    let lockFeature: string | undefined;
    let lockLayer: string | undefined;
    if (this.coordination) {
      lockFeature = task.id;
      lockLayer = step.adapter ?? 'general';
      const lockResult = await this.coordination.acquireLock(lockFeature, lockLayer);
      if (!lockResult.acquired) {
        const holder = lockResult.conflict?.instanceId ?? 'unknown';
        throw new WorktreeConflictError(lockFeature, lockLayer, holder);
      }
      await this.coordination.publishActivity('worktree-create', {
        taskId: task.id, branch: branchName, layer: lockLayer,
      }).catch(() => {});
    }

    // Create isolated worktree (declared outside try so catch can clean up)
    let worktreePath: WorktreePath | undefined;
    try {
      worktreePath = await this.worktree.create(branchName);
      task.worktreeBranch = branchName;

      // Spawn an agent for this task
      const agent = await this.swarm.spawnAgent(
        `agent-${task.id}`,
        task.agentRole,
        task.id,
      );

      // Store step context in swarm memory for the agent to pick up
      await this.swarm.memoryStore({
        key: `task:${task.id}:context`,
        value: JSON.stringify({
          description: step.description,
          adapter: step.adapter,
          worktree: worktreePath.absolutePath,
          dependencies: step.dependencies,
        }),
        namespace: 'hex',
        tags: ['task-context', step.adapter ?? 'general'],
      });

      // Mark task complete (actual code execution happens via Claude Agent tool)
      await this.swarm.completeTask(task.id, `Prepared worktree at ${worktreePath.absolutePath}`);
      await this.swarm.terminateAgent(agent.id);
      // Release coordination lock after completion
      if (this.coordination && lockFeature && lockLayer) {
        await this.coordination.releaseLock(lockFeature, lockLayer).catch(() => {});
        await this.coordination.publishActivity('worktree-complete', {
          taskId: task.id, branch: branchName,
        }).catch(() => {});
      }
    } catch (err) {
      // Release coordination lock on failure
      if (this.coordination && lockFeature && lockLayer) {
        await this.coordination.releaseLock(lockFeature, lockLayer).catch(() => {});
      }
      // Clean up worktree on failure
      if (worktreePath) {
        await this.worktree.cleanup(worktreePath).catch(() => {});
      }
      throw err;
    }
  }
}
