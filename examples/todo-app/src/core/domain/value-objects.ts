import { randomUUID } from 'node:crypto';
import { ValidationError } from './errors.js';

export type TodoId = string;
export type TodoStatus = 'pending' | 'in-progress' | 'completed';
export type Priority = 'low' | 'medium' | 'high';

export interface TodoTitle {
  readonly value: string;
}

export function createTodoId(): TodoId {
  return randomUUID();
}

export function createTodoTitle(raw: string): TodoTitle {
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    throw new ValidationError('Todo title cannot be empty');
  }
  if (trimmed.length > 200) {
    throw new ValidationError('Todo title cannot exceed 200 characters');
  }
  return { value: trimmed };
}

export function isValidStatus(s: string): s is TodoStatus {
  return s === 'pending' || s === 'in-progress' || s === 'completed';
}

export function isValidPriority(p: string): p is Priority {
  return p === 'low' || p === 'medium' || p === 'high';
}

export function shortId(id: TodoId): string {
  return id.slice(0, 8);
}
