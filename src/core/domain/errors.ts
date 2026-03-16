/**
 * Domain Error Types
 *
 * Typed error hierarchy for the domain layer. All domain errors extend
 * DomainError so adapters can catch them uniformly. Raw `new Error()` is
 * banned in domain/ and usecases/ by ESLint (see eslint.config.js).
 */

export class DomainError extends Error {
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
