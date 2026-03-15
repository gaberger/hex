/**
 * Flappy Bird — Ports (Hexagonal Architecture)
 *
 * Sign Convention Contract (screen coordinates):
 *   +Y = downward
 *   Gravity = positive (adds to velocity, bird falls)
 *   Flap strength = NEGATIVE (sets velocity negative, bird rises)
 *   Velocity: positive = falling, negative = rising
 */

// ---------------------------------------------------------------------------
// Value types
// ---------------------------------------------------------------------------

export type Phase = 'ready' | 'playing' | 'gameover';

export interface Bird {
  x: number;
  y: number;
  velocity: number;
  rotation: number;
}

export interface Pipe {
  x: number;
  gapY: number;
  scored: boolean;
}

export interface GameState {
  bird: Bird;
  pipes: Pipe[];
  phase: Phase;
  score: number;
  highScore: number;
  elapsed: number;
}

export interface GameConfig {
  canvasWidth: number;
  canvasHeight: number;
  /** Downward acceleration. MUST BE POSITIVE. */
  gravity: number;
  /** Upward force applied on flap. MUST BE NEGATIVE (screen Y-axis points down). */
  flapStrength: number;
  pipeWidth: number;
  pipeGap: number;
  pipeSpeed: number;
  pipeSpawnInterval: number;
  groundHeight: number;
  birdSize: number;
  birdX: number;
}

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
