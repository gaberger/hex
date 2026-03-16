import { describe, it, expect } from 'bun:test';
import {
  DomainError,
  NotFoundError,
  ValidationError,
  ConflictError,
} from '../../src/core/domain/errors.js';

describe('Domain Errors', () => {
  it('NotFoundError has correct code and message', () => {
    const err = new NotFoundError('Todo', 'abc-123');
    expect(err.message).toBe('Todo not found: abc-123');
    expect(err.code).toBe('NOT_FOUND');
    expect(err.name).toBe('NotFoundError');
    expect(err instanceof DomainError).toBe(true);
    expect(err instanceof Error).toBe(true);
  });

  it('ValidationError has correct code', () => {
    const err = new ValidationError('Title too long');
    expect(err.message).toBe('Title too long');
    expect(err.code).toBe('VALIDATION');
    expect(err instanceof DomainError).toBe(true);
  });

  it('ConflictError has correct code', () => {
    const err = new ConflictError('Already completed');
    expect(err.message).toBe('Already completed');
    expect(err.code).toBe('CONFLICT');
    expect(err instanceof DomainError).toBe(true);
  });
});
