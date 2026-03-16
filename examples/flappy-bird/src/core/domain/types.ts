/**
 * Flappy Bird — Domain Value Types
 *
 * These types live in the domain layer (innermost ring) so that
 * both domain logic and ports can reference them without violating
 * hexagonal dependency rules.
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

export interface LeaderboardEntry {
  id: number;
  name: string;
  score: number;
  timestamp: string;
}
