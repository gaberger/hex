/**
 * Property-Based Tests — Physics
 *
 * These tests verify domain invariants that must hold for ALL inputs,
 * catching sign-convention and logic bugs that specific-value tests miss.
 */

import { describe, it, expect } from 'bun:test';
import {
  applyFlap,
  applyGravity,
  applyVelocity,
  checkBounds,
} from '../../src/core/domain/physics.js';
import type { Bird, GameConfig } from '../../src/core/ports/index.js';

const config: GameConfig = {
  canvasWidth: 400,
  canvasHeight: 600,
  gravity: 980,
  flapStrength: -280,
  pipeWidth: 52,
  pipeGap: 140,
  pipeSpeed: 160,
  pipeSpawnInterval: 1.8,
  groundHeight: 80,
  birdSize: 24,
  birdX: 80,
};

/**
 * Simple pseudo-random number generator for reproducible property tests.
 * Not cryptographically secure — just for test variety.
 */
function* randomValues(seed: number, count: number, min: number, max: number): Generator<number> {
  let s = seed;
  for (let i = 0; i < count; i++) {
    s = (s * 1664525 + 1013904223) & 0x7fffffff;
    yield min + (s / 0x7fffffff) * (max - min);
  }
}

const SAMPLE_COUNT = 200;

// ---------------------------------------------------------------------------
// Property: applyFlap always returns negative velocity
// ---------------------------------------------------------------------------

describe('Property: applyFlap always produces negative velocity', () => {
  it('for any positive starting velocity, flap result is negative', () => {
    for (const vel of randomValues(42, SAMPLE_COUNT, 0, 1000)) {
      // applyFlap ignores current velocity; it returns flapStrength directly
      const result = applyFlap(config.flapStrength);
      expect(result).toBeLessThan(0);
      // Also verify we didn't accidentally use the input velocity
      expect(result).toBe(config.flapStrength);
      // Suppress unused variable warning
      void vel;
    }
  });
});

// ---------------------------------------------------------------------------
// Property: applyGravity always increases velocity (for dt > 0)
// ---------------------------------------------------------------------------

describe('Property: applyGravity always increases velocity', () => {
  it('for any velocity and positive dt, result > input', () => {
    for (const vel of randomValues(99, SAMPLE_COUNT, -500, 500)) {
      const dt = 1 / 60;
      const result = applyGravity(vel, config.gravity, dt);
      expect(result).toBeGreaterThan(vel);
    }
  });
});

// ---------------------------------------------------------------------------
// Property: after flap then N gravity ticks, bird eventually falls
// ---------------------------------------------------------------------------

describe('Property: gravity eventually overcomes flap', () => {
  it('after a flap, sufficient gravity ticks make velocity positive (falling)', () => {
    let velocity = applyFlap(config.flapStrength); // starts negative
    expect(velocity).toBeLessThan(0);

    const dt = 1 / 60;
    let ticks = 0;
    const maxTicks = 600; // 10 seconds at 60fps

    while (velocity <= 0 && ticks < maxTicks) {
      velocity = applyGravity(velocity, config.gravity, dt);
      ticks++;
    }

    expect(velocity).toBeGreaterThan(0);
    expect(ticks).toBeLessThan(maxTicks);
  });
});

// ---------------------------------------------------------------------------
// Property: checkBounds never triggers for y in [0, groundY - birdSize)
// ---------------------------------------------------------------------------

describe('Property: checkBounds is safe in valid play area', () => {
  it('never triggers for any y in [0, groundY - birdSize)', () => {
    const groundY = config.canvasHeight - config.groundHeight;
    const safeMax = groundY - config.birdSize;

    for (const y of randomValues(77, SAMPLE_COUNT, 0, safeMax - 0.01)) {
      const bird: Bird = { x: 80, y, velocity: 0, rotation: 0 };
      expect(checkBounds(bird, config)).toBe(false);
    }
  });

  it('never triggers for negative y (ceiling) — BS-4', () => {
    for (const y of randomValues(55, SAMPLE_COUNT, -200, -0.01)) {
      const bird: Bird = { x: 80, y, velocity: 0, rotation: 0 };
      expect(checkBounds(bird, config)).toBe(false);
    }
  });
});

// ---------------------------------------------------------------------------
// Property: velocity-position consistency
// ---------------------------------------------------------------------------

describe('Property: position changes match velocity sign', () => {
  it('positive velocity moves bird downward (y increases)', () => {
    for (const vel of randomValues(33, SAMPLE_COUNT, 1, 1000)) {
      const y0 = 300;
      const y1 = applyVelocity(y0, vel, 1 / 60);
      expect(y1).toBeGreaterThan(y0);
    }
  });

  it('negative velocity moves bird upward (y decreases)', () => {
    for (const vel of randomValues(44, SAMPLE_COUNT, -1000, -1)) {
      const y0 = 300;
      const y1 = applyVelocity(y0, vel, 1 / 60);
      expect(y1).toBeLessThan(y0);
    }
  });
});
