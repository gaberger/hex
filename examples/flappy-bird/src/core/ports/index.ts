/**
 * Flappy Bird — Ports (Hexagonal Architecture)
 *
 * Value types are defined in domain/types.ts and re-exported here
 * so that adapters can import everything from a single module.
 */

// Re-export domain value types (ports → domain is the correct dependency direction)
export type { Phase, Bird, Pipe, GameState, GameConfig, LeaderboardEntry } from '../domain/types.js';
import type { Bird, Pipe, Phase, GameState, GameConfig, LeaderboardEntry } from '../domain/types.js';

// ---------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------

export interface IGamePort {
  start(): void;
  flap(): void;
  tick(dt: number): void;
  getState(): GameState;
}

export interface IRenderPort {
  clear(): void;
  drawBird(bird: Bird): void;
  drawPipe(pipe: Pipe, config: GameConfig): void;
  drawGround(config: GameConfig): void;
  drawScore(score: number): void;
  drawOverlay(phase: Phase, score: number, highScore: number): void;
}

export interface IAudioPort {
  playFlap(): void;
  playScore(): void;
  playHit(): void;
}

export interface IStoragePort {
  loadHighScore(): number;
  saveHighScore(score: number): void;
}

export interface IInputPort {
  onAction(callback: () => void): void;
  destroy(): void;
}

export interface ILeaderboardPort {
  saveScore(name: string, score: number): Promise<void>;
  getTopScores(limit: number): Promise<LeaderboardEntry[]>;
}
