import type { IStoragePort } from '../../core/ports/index.js';

export class LocalStorageAdapter implements IStoragePort {
  async loadHighScore(): Promise<number> {
    return Number(localStorage.getItem('flappy-highscore') ?? '0');
  }

  async saveHighScore(score: number): Promise<void> {
    localStorage.setItem('flappy-highscore', String(score));
  }
}
