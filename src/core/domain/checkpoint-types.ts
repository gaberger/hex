/**
 * Checkpoint Domain Types
 *
 * Value objects for crash-resilient state persistence.
 * These types capture point-in-time snapshots of swarm state
 * so sessions can recover after unexpected termination.
 *
 * Dependency: imports from domain only (swarm types via value-objects re-export path).
 */

// ─── Task Snapshot ──────────────────────────────────────

/** Point-in-time snapshot of a single swarm task. */
export interface TaskSnapshot {
  taskId: string;
  title: string;
  status: 'pending' | 'assigned' | 'running' | 'completed' | 'failed';
  assignee?: string;
  agentRole: string;
  adapter?: string;
  worktreeBranch?: string;
  result?: string;
  commitHash?: string;
  snapshotAt: string; // ISO 8601
}

// ─── Feature Progress ───────────────────────────────────

/** Lifecycle phase of a hex feature. */
export type FeaturePhase =
  | 'specs'
  | 'plan'
  | 'worktrees'
  | 'code'
  | 'validate'
  | 'integrate'
  | 'finalize';

/** Aggregated progress of a feature across its lifecycle phases. */
export interface FeatureProgress {
  featureId: string;
  title: string;
  phase: FeaturePhase;
  totalSteps: number;
  completedSteps: number;
  failedSteps: number;
  startedAt: string; // ISO 8601
  updatedAt: string; // ISO 8601
  taskSnapshots: TaskSnapshot[];
}

// ─── Checkpoint Entry ───────────────────────────────────

/** Top-level checkpoint envelope wrapping all recoverable state. */
export interface CheckpointEntry {
  id: string;
  projectId: string;
  projectPath: string;
  createdAt: string; // ISO 8601
  swarmStatus: {
    topology: string;
    agentCount: number;
    status: string;
  };
  features: FeatureProgress[];
  /** Tasks not associated with any feature. */
  orphanTasks: TaskSnapshot[];
  /** Optional ADR snapshot — present when ADR tracking is active. */
  adrs?: import('./adr-types.js').ADRSnapshot;
}
