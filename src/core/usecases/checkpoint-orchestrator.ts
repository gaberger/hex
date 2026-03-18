/**
 * Checkpoint Orchestrator Use Case
 *
 * Composes ICheckpointPort and ISwarmPort to provide crash-resilient
 * state persistence. Captures point-in-time snapshots of swarm state
 * so sessions can recover after unexpected termination.
 *
 * Imports: domain/ and ports/ only (hex architecture rule).
 */

import type { ICheckpointPort, ICheckpointOrchestrator } from '../ports/checkpoint.js';
import type { ISwarmPort, SwarmTask } from '../ports/swarm.js';
import type {
  CheckpointEntry,
  TaskSnapshot,
  FeatureProgress,
  FeaturePhase,
} from '../domain/checkpoint-types.js';

export class CheckpointOrchestrator implements ICheckpointOrchestrator {
  constructor(
    private readonly checkpoint: ICheckpointPort,
    private readonly swarm: ISwarmPort,
    private readonly projectId: string,
    private readonly projectPath: string,
  ) {}

  /**
   * Capture a manual checkpoint of current swarm state.
   * Reads swarm status and tasks, builds a CheckpointEntry, persists it.
   */
  async manualCheckpoint(): Promise<CheckpointEntry> {
    const [swarmStatus, tasks] = await Promise.all([
      this.swarm.status(),
      this.swarm.listTasks(),
    ]);

    const now = new Date().toISOString();
    const features = this.buildFeatureMap(tasks, now);

    // Tasks not matched to any feature go into orphanTasks
    const featureTaskIds = new Set(
      features.flatMap((f) => f.taskSnapshots.map((ts) => ts.taskId)),
    );
    const orphanTasks: TaskSnapshot[] = tasks
      .filter((t) => !featureTaskIds.has(t.id))
      .map((t) => this.taskToSnapshot(t, now));

    const entry: CheckpointEntry = {
      id: crypto.randomUUID(),
      projectId: this.projectId,
      projectPath: this.projectPath,
      createdAt: now,
      swarmStatus: {
        topology: swarmStatus.topology,
        agentCount: swarmStatus.agentCount,
        status: swarmStatus.status,
      },
      features,
      orphanTasks,
    };

    await this.checkpoint.checkpoint(entry);
    return entry;
  }

  /**
   * Recover the most recent checkpoint for this project.
   * READ-ONLY — does not modify swarm state.
   * The caller decides how to reconcile.
   */
  async recover(): Promise<CheckpointEntry | null> {
    return this.checkpoint.recover(this.projectId);
  }

  /**
   * Called when a task changes status (completed, failed).
   * Auto-checkpoints current state. Failures are swallowed
   * so checkpoint issues never break the task flow.
   */
  async onTaskTransition(
    _taskId: string,
    _newStatus: SwarmTask['status'],
  ): Promise<void> {
    try {
      await this.manualCheckpoint();
    } catch {
      // Checkpoint failures must not break task flow
    }
  }

  /**
   * Remove old checkpoints, keeping only the most recent N.
   * Returns the number of checkpoints deleted.
   */
  async pruneOld(keepCount?: number): Promise<number> {
    return this.checkpoint.prune(this.projectId, keepCount ?? 20);
  }

  // ─── Private Helpers ──────────────────────────────────────

  /**
   * Group tasks by feature name and build FeatureProgress entries.
   * Feature name is extracted from the task title using a "feat/<name>" or
   * "[<name>]" prefix convention. Tasks without a recognizable feature
   * prefix are left as orphans (not included here).
   */
  private buildFeatureMap(tasks: SwarmTask[], now: string): FeatureProgress[] {
    const groups = new Map<string, SwarmTask[]>();

    for (const task of tasks) {
      const featureName = this.extractFeatureName(task);
      if (featureName === null) continue;

      const existing = groups.get(featureName);
      if (existing) {
        existing.push(task);
      } else {
        groups.set(featureName, [task]);
      }
    }

    const features: FeatureProgress[] = [];
    for (const [name, featureTasks] of groups) {
      const snapshots = featureTasks.map((t) => this.taskToSnapshot(t, now));
      const completed = featureTasks.filter((t) => t.status === 'completed').length;
      const failed = featureTasks.filter((t) => t.status === 'failed').length;

      features.push({
        featureId: name,
        title: name,
        phase: this.inferPhase(featureTasks),
        totalSteps: featureTasks.length,
        completedSteps: completed,
        failedSteps: failed,
        startedAt: now,
        updatedAt: now,
        taskSnapshots: snapshots,
      });
    }

    return features;
  }

  /**
   * Extract a feature name from a task's title or adapter field.
   * Recognizes patterns: "feat/<name>/...", "[<name>] ...", or adapter-based grouping.
   * Returns null if no feature can be identified (task becomes an orphan).
   */
  private extractFeatureName(task: SwarmTask): string | null {
    const title = task.title;

    // Pattern: "feat/<name>/..." or "feat/<name>:..."
    const featMatch = title.match(/^feat\/([^/:\s]+)/);
    if (featMatch) return featMatch[1];

    // Pattern: "[<name>] ..."
    const bracketMatch = title.match(/^\[([^\]]+)\]/);
    if (bracketMatch) return bracketMatch[1];

    // Pattern: worktree branch "feat/<name>/..."
    if (task.worktreeBranch) {
      const branchMatch = task.worktreeBranch.match(/^feat\/([^/]+)/);
      if (branchMatch) return branchMatch[1];
    }

    return null;
  }

  /** Convert a SwarmTask to a TaskSnapshot. */
  private taskToSnapshot(task: SwarmTask, now: string): TaskSnapshot {
    return {
      taskId: task.id,
      title: task.title,
      status: task.status,
      assignee: task.assignee,
      agentRole: task.agentRole,
      adapter: task.adapter,
      worktreeBranch: task.worktreeBranch,
      result: task.result,
      commitHash: task.commitHash,
      snapshotAt: now,
    };
  }

  /**
   * Infer the current feature phase based on task statuses and roles.
   * Follows the hex feature lifecycle: specs → plan → worktrees → code → validate → integrate → finalize.
   */
  private inferPhase(tasks: SwarmTask[]): FeaturePhase {
    const allCompleted = tasks.every((t) => t.status === 'completed');
    const anyFailed = tasks.some((t) => t.status === 'failed');
    const anyRunning = tasks.some((t) => t.status === 'running' || t.status === 'assigned');

    if (allCompleted) return 'finalize';
    if (anyFailed) return 'validate'; // validation caught a failure

    // Check for integrator tasks running
    const hasIntegrator = tasks.some(
      (t) => t.agentRole === 'integrator' && (t.status === 'running' || t.status === 'completed'),
    );
    if (hasIntegrator) return 'integrate';

    // Check for reviewer/validator tasks
    const hasReviewer = tasks.some(
      (t) => t.agentRole === 'reviewer' && (t.status === 'running' || t.status === 'completed'),
    );
    if (hasReviewer) return 'validate';

    // Check for coder tasks
    const hasCoder = tasks.some(
      (t) => t.agentRole === 'coder' && (t.status === 'running' || t.status === 'completed'),
    );
    if (hasCoder) return 'code';

    // Check for planner tasks
    const hasPlanner = tasks.some(
      (t) => t.agentRole === 'planner' && (t.status === 'running' || t.status === 'completed'),
    );
    if (hasPlanner) return 'plan';

    if (anyRunning) return 'code'; // default active phase

    return 'specs'; // nothing started yet
  }
}
