/**
 * Flappy Bird — Physics Domain
 *
 * SIGN CONVENTION (screen coordinates):
 *   +Y = downward on screen
 *   gravity = positive number (e.g. 980) -> increases velocity -> bird falls
 *   flapStrength = NEGATIVE number (e.g. -280) -> sets velocity negative -> bird rises
 *   velocity > 0 means falling, velocity < 0 means rising
 *
 * IMPORTANT: applyFlap returns flapStrength DIRECTLY. It does NOT negate it.
 * The caller provides flapStrength as a negative value per the sign convention.
 */

import type { Bird, Pipe, GameConfig } from '../ports/index.js';

// ---------------------------------------------------------------------------
// Pure physics functions
// ---------------------------------------------------------------------------

/**
 * Apply gravity to the bird's velocity over a time step.
 * Gravity is positive, so velocity increases (bird accelerates downward).
 */
export function applyGravity(velocity: number, gravity: number, dt: number): number {
  return velocity + gravity * dt;
}

/**
 * Apply a flap: returns flapStrength directly.
 * flapStrength is ALREADY NEGATIVE per the sign convention contract.
 * Do NOT negate it — that was Bug #1 in the original implementation.
 */
export function applyFlap(flapStrength: number): number {
  return flapStrength;
}

/**
 * Update bird position from velocity over a time step.
 */
export function applyVelocity(y: number, velocity: number, dt: number): number {
  return y + velocity * dt;
}

/**
 * Calculate bird rotation from velocity.
 * Negative velocity (rising) -> negative rotation (nose up).
 * Positive velocity (falling) -> positive rotation (nose down).
 */
export function calculateRotation(velocity: number): number {
  const maxAngle = Math.PI / 4;
  const clampedVel = Math.max(-400, Math.min(400, velocity));
  return (clampedVel / 400) * maxAngle;
}

/**
 * Check if the bird has hit the ground.
 * BS-4: Bird dies on GROUND contact only, NOT ceiling.
 * Ceiling is clamped (bird stops at y=0) but does not kill.
 */
export function checkBounds(bird: Bird, config: GameConfig): boolean {
  const groundY = config.canvasHeight - config.groundHeight;
  return bird.y + config.birdSize >= groundY;
}

/**
 * Check if the bird collides with a pipe.
 * BS-5: Bird dies when colliding with any part of a pipe.
 */
export function checkPipeCollision(bird: Bird, pipe: Pipe, config: GameConfig): boolean {
  const birdRight = bird.x + config.birdSize;
  const birdBottom = bird.y + config.birdSize;
  const pipeLeft = pipe.x;
  const pipeRight = pipe.x + config.pipeWidth;

  // Horizontal overlap check
  if (birdRight <= pipeLeft || bird.x >= pipeRight) {
    return false;
  }

  // Vertical gap check — bird must be fully inside the gap to survive
  const gapTop = pipe.gapY;
  const gapBottom = pipe.gapY + config.pipeGap;

  return bird.y < gapTop || birdBottom > gapBottom;
}

/**
 * Check if the bird has passed a pipe (for scoring).
 * BS-11: Score increments when bird fully passes a pipe.
 */
export function checkPipePass(bird: Bird, pipe: Pipe): boolean {
  return !pipe.scored && bird.x > pipe.x;
}
