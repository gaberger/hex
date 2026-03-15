/**
 * Unit Tests — Physics Domain
 *
 * CRITICAL: All test configs use NEGATIVE flapStrength matching production.
 * This prevents the test-config-parity bug (Bug #1 from root cause analysis).
 */

import { describe, it, expect } from 'bun:test';
import {
  applyGravity,
  applyFlap,
  applyVelocity,
  calculateRotation,
  checkBounds,
  checkPipeCollision,
  checkPipePass,
} from '../../src/core/domain/physics.js';
import type { Bird, Pipe, GameConfig } from '../../src/core/ports/index.js';

// Production-matching config — flapStrength is NEGATIVE
const config: GameConfig = {
  canvasWidth: 400,
  canvasHeight: 600,
  gravity: 980,
  flapStrength: -280,     // NEGATIVE — matches production
  pipeWidth: 52,
  pipeGap: 140,
  pipeSpeed: 160,
  pipeSpawnInterval: 1.8,
  groundHeight: 80,
  birdSize: 24,
  birdX: 80,
};

function makeBird(overrides: Partial<Bird> = {}): Bird {
  return { x: 80, y: 300, velocity: 0, rotation: 0, ...overrides };
}

function makePipe(overrides: Partial<Pipe> = {}): Pipe {
  return { x: 200, gapY: 200, scored: false, ...overrides };
}

// ---------------------------------------------------------------------------
// applyGravity
// ---------------------------------------------------------------------------

describe('applyGravity', () => {
  it('increases velocity (bird accelerates downward)', () => {
    const result = applyGravity(0, config.gravity, 1 / 60);
    expect(result).toBeGreaterThan(0);
  });

  it('adds gravity * dt to current velocity', () => {
    const dt = 0.016;
    const result = applyGravity(100, 980, dt);
    expect(result).toBeCloseTo(100 + 980 * dt, 5);
  });
});

// ---------------------------------------------------------------------------
// applyFlap — BS-2: flap moves bird UPWARD (negative velocity)
// ---------------------------------------------------------------------------

describe('applyFlap', () => {
  it('returns flapStrength directly (negative = upward)', () => {
    const result = applyFlap(config.flapStrength);
    expect(result).toBe(-280);
  });

  it('result is negative (bird rises)', () => {
    const result = applyFlap(config.flapStrength);
    expect(result).toBeLessThan(0);
  });

  it('does NOT negate the input — passes through as-is', () => {
    const result = applyFlap(-280);
    // If there were a double-negation bug, this would be +280
    expect(result).toBe(-280);
  });
});

// ---------------------------------------------------------------------------
// applyVelocity
// ---------------------------------------------------------------------------

describe('applyVelocity', () => {
  it('moves bird downward when velocity is positive', () => {
    const result = applyVelocity(300, 100, 0.016);
    expect(result).toBeGreaterThan(300);
  });

  it('moves bird upward when velocity is negative', () => {
    const result = applyVelocity(300, -280, 0.016);
    expect(result).toBeLessThan(300);
  });
});

// ---------------------------------------------------------------------------
// calculateRotation — BS-3
// ---------------------------------------------------------------------------

describe('calculateRotation', () => {
  it('returns negative rotation when rising (velocity < 0)', () => {
    expect(calculateRotation(-280)).toBeLessThan(0);
  });

  it('returns positive rotation when falling (velocity > 0)', () => {
    expect(calculateRotation(200)).toBeGreaterThan(0);
  });

  it('returns zero rotation at zero velocity', () => {
    expect(calculateRotation(0)).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// checkBounds — BS-4: dies on GROUND only, NOT ceiling
// ---------------------------------------------------------------------------

describe('checkBounds', () => {
  it('returns true when bird hits the ground', () => {
    const groundY = config.canvasHeight - config.groundHeight; // 520
    const bird = makeBird({ y: groundY - config.birdSize + 1 }); // touching ground
    expect(checkBounds(bird, config)).toBe(true);
  });

  it('returns false when bird is at ceiling (y=0) — ceiling does NOT kill', () => {
    const bird = makeBird({ y: 0 });
    expect(checkBounds(bird, config)).toBe(false);
  });

  it('returns false when bird is above screen (y < 0) — no ceiling death', () => {
    const bird = makeBird({ y: -10 });
    expect(checkBounds(bird, config)).toBe(false);
  });

  it('returns false when bird is in normal play area', () => {
    const bird = makeBird({ y: 300 });
    expect(checkBounds(bird, config)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// checkPipeCollision — BS-5
// ---------------------------------------------------------------------------

describe('checkPipeCollision', () => {
  it('returns false when bird is in the gap', () => {
    const pipe = makePipe({ x: 80, gapY: 200 }); // gap: 200..340
    const bird = makeBird({ y: 250 }); // inside gap
    expect(checkPipeCollision(bird, pipe, config)).toBe(false);
  });

  it('returns true when bird hits top pipe', () => {
    const pipe = makePipe({ x: 80, gapY: 200 });
    const bird = makeBird({ y: 150 }); // above gap
    expect(checkPipeCollision(bird, pipe, config)).toBe(true);
  });

  it('returns true when bird hits bottom pipe', () => {
    const pipe = makePipe({ x: 80, gapY: 200 });
    const bird = makeBird({ y: 350 }); // below gap (200 + 140 = 340)
    expect(checkPipeCollision(bird, pipe, config)).toBe(true);
  });

  it('returns false when bird is past the pipe horizontally', () => {
    const pipe = makePipe({ x: 10 }); // pipe at x=10, width=52 -> ends at 62
    const bird = makeBird({ x: 80, y: 100 }); // bird starts at 80, past pipe
    expect(checkPipeCollision(bird, pipe, config)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// checkPipePass — BS-11
// ---------------------------------------------------------------------------

describe('checkPipePass', () => {
  it('returns true when bird passes unscored pipe', () => {
    const bird = makeBird({ x: 250 });
    const pipe = makePipe({ x: 200, scored: false });
    expect(checkPipePass(bird, pipe)).toBe(true);
  });

  it('returns false for already-scored pipe', () => {
    const bird = makeBird({ x: 250 });
    const pipe = makePipe({ x: 200, scored: true });
    expect(checkPipePass(bird, pipe)).toBe(false);
  });

  it('returns false when bird has not passed pipe yet', () => {
    const bird = makeBird({ x: 80 });
    const pipe = makePipe({ x: 200, scored: false });
    expect(checkPipePass(bird, pipe)).toBe(false);
  });
});
