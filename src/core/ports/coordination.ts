/**
 * Coordination Port
 *
 * Enables multi-instance coordination through the hex-hub daemon.
 * Multiple Claude Code sessions can safely share worktrees, claim tasks,
 * and publish activity without stomping on each other.
 */

// ── Value Types ──────────────────────────────────────────

export interface UnstagedFile {
  path: string;
  status: 'modified' | 'added' | 'deleted';
  layer: string;
}

export interface WorktreeLock {
  instanceId: string;
  projectId: string;
  feature: string;
  layer: string;
  acquiredAt: string;
  heartbeatAt: string;
  ttlSecs: number;
}

export interface LockResult {
  acquired: boolean;
  lock: WorktreeLock | null;
  /** When acquired=false, this is the conflicting lock holder */
  conflict: WorktreeLock | null;
}

export interface TaskClaim {
  taskId: string;
  instanceId: string;
  claimedAt: string;
  heartbeatAt: string;
}

export interface ClaimResult {
  claimed: boolean;
  claim: TaskClaim | null;
  /** When claimed=false, this is the existing claim holder */
  conflict: TaskClaim | null;
}

export interface ActivityEntry {
  instanceId: string;
  projectId: string;
  action: string;
  details: Record<string, unknown>;
  timestamp: string;
}

export interface InstanceInfo {
  instanceId: string;
  projectId: string;
  pid: number;
  sessionLabel: string;
  registeredAt: string;
  lastSeen: string;
}

export interface UnstagedState {
  instanceId: string;
  projectId: string;
  files: UnstagedFile[];
  capturedAt: string;
}

// ── Port Interface ───────────────────────────────────────

export interface ICoordinationPort {
  // Instance lifecycle
  registerInstance(sessionLabel?: string): Promise<string>; // returns instanceId
  heartbeat(unstagedFiles?: UnstagedFile[]): Promise<void>;

  // Worktree locks
  acquireLock(feature: string, layer: string): Promise<LockResult>;
  releaseLock(feature: string, layer: string): Promise<void>;
  listLocks(): Promise<WorktreeLock[]>;

  // Task ownership
  claimTask(taskId: string): Promise<ClaimResult>;
  releaseTask(taskId: string): Promise<void>;
  listClaims(): Promise<TaskClaim[]>;

  // Activity stream
  publishActivity(action: string, details?: Record<string, unknown>): Promise<void>;
  getActivities(limit?: number): Promise<ActivityEntry[]>;

  // Unstaged tracking
  getUnstagedAcrossInstances(): Promise<UnstagedState[]>;
}
