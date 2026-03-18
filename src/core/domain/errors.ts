/**
 * Domain Error Types
 *
 * Typed error hierarchy for the domain layer. All domain errors extend
 * DomainError so adapters can catch them uniformly. Raw `new Error()` is
 * banned in domain/ and usecases/ by ESLint (see eslint.config.js).
 */

class DomainError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'DomainError';
    this.code = code;
  }
}

export class WorktreeConflictError extends DomainError {
  readonly feature: string;
  readonly layer: string;
  readonly holder: string;

  constructor(feature: string, layer: string, holder: string) {
    super('WORKTREE_CONFLICT', `Worktree lock conflict on ${feature}/${layer} — held by ${holder}`);
    this.name = 'WorktreeConflictError';
    this.feature = feature;
    this.layer = layer;
    this.holder = holder;
  }
}

export class TaskConflictError extends DomainError {
  readonly taskId: string;
  readonly holder: string;

  constructor(taskId: string, holder: string) {
    super('TASK_CONFLICT', `Task ${taskId} already claimed by ${holder}`);
    this.name = 'TaskConflictError';
    this.taskId = taskId;
    this.holder = holder;
  }
}
