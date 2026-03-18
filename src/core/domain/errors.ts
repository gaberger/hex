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

export class ValidationError extends DomainError {
  readonly field?: string;

  constructor(message: string, field?: string) {
    super('VALIDATION_ERROR', message);
    this.name = 'ValidationError';
    this.field = field;
  }
}

export class InvariantViolation extends DomainError {
  constructor(message: string) {
    super('INVARIANT_VIOLATION', message);
    this.name = 'InvariantViolation';
  }
}

export class BoundaryViolation extends DomainError {
  readonly fromLayer: string;
  readonly toLayer: string;

  constructor(fromLayer: string, toLayer: string, rule: string) {
    super('BOUNDARY_VIOLATION', `${fromLayer} -> ${toLayer}: ${rule}`);
    this.name = 'BoundaryViolation';
    this.fromLayer = fromLayer;
    this.toLayer = toLayer;
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
