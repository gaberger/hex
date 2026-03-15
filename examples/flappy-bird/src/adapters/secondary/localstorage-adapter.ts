/**
 * LocalStorage Adapter — Secondary Adapter
 * Implements IStoragePort using browser localStorage.
 */

import type { IStoragePort } from '../../core/ports/index.js';

const HIGH_SCORE_KEY = 'flappy-bird-high-score';

export class LocalStorageAdapter implements IStoragePort {
  loadHighScore(): number {
    try {
      const raw = localStorage.getItem(HIGH_SCORE_KEY);
      if (raw === null) return 0;
      const parsed = parseInt(raw, 10);
      return Number.isNaN(parsed) ? 0 : parsed;
    } catch {
      return 0;
    }
  }

  saveHighScore(score: number): void {
    try {
      localStorage.setItem(HIGH_SCORE_KEY, String(score));
    } catch {
      // Silently ignore storage errors (e.g., private browsing)
    }
  }
}
