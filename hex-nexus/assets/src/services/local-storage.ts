/**
 * local-storage.ts — localStorage secondary adapter (ADR-056).
 *
 * Singleton service wrapping browser localStorage with typed JSON
 * serialization. Implements IStorageAdapter port interface.
 *
 * Stores import this instead of calling localStorage directly,
 * making them testable (swap with in-memory mock).
 */
import type { IStorageAdapter } from '../types/services';

class LocalStorageAdapter implements IStorageAdapter {
  get<T = string>(key: string): T | null {
    try {
      const raw = localStorage.getItem(key);
      if (raw === null) return null;
      // Try JSON parse for complex types, fall back to raw string
      try {
        return JSON.parse(raw) as T;
      } catch {
        return raw as unknown as T;
      }
    } catch {
      return null;
    }
  }

  set<T = string>(key: string, value: T): void {
    try {
      const serialized = typeof value === 'string' ? value : JSON.stringify(value);
      localStorage.setItem(key, serialized);
    } catch (e) {
      console.warn('[storage] Failed to write:', key, e);
    }
  }

  remove(key: string): void {
    try {
      localStorage.removeItem(key);
    } catch {
      // Ignore — storage may be unavailable
    }
  }

  has(key: string): boolean {
    try {
      return localStorage.getItem(key) !== null;
    } catch {
      return false;
    }
  }
}

/** Singleton localStorage adapter. */
export const storage: IStorageAdapter = new LocalStorageAdapter();
