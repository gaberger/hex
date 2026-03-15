import { describe, it, expect } from 'bun:test';
import {
  applyGravity, applyFlap, updatePosition, calculateRotation,
  checkCollision, checkBounds, generatePipeGapY, shouldSpawnPipe,
  isPastPipe,
} from '../../src/core/domain/physics.js';
import type { GameConfig, PipeState, Rect } from '../../src/core/ports/index.js';

const cfg: GameConfig = {
  canvasWidth: 400, canvasHeight: 600, gravity: 980,
  flapStrength: 300, pipeSpeed: 150, pipeGap: 120,
  pipeWidth: 50, pipeSpawnInterval: 90,
};

describe('applyGravity', () => {
  it('increases velocity (falls faster)', () => {
    expect(applyGravity(0, 980, 16)).toBeGreaterThan(0);
    expect(applyGravity(100, 980, 16)).toBeGreaterThan(100);
  });
});

describe('applyFlap', () => {
  it('sets negative velocity (goes up)', () => {
    expect(applyFlap(200, -280)).toBeLessThan(0);
    expect(applyFlap(200, -280)).toBe(-280);
  });
});

describe('updatePosition', () => {
  it('moves bird based on velocity', () => {
    const pos = updatePosition({ x: 80, y: 300 }, 100, 16);
    expect(pos.y).toBeGreaterThan(300);
    expect(pos.x).toBe(80);
  });
});

describe('calculateRotation', () => {
  it('positive for falling', () => {
    expect(calculateRotation(200)).toBeGreaterThan(0);
  });
  it('negative for rising', () => {
    expect(calculateRotation(-200)).toBeLessThan(0);
  });
});

describe('checkCollision', () => {
  const bird: Rect = { x: 80, y: 300, width: 20, height: 20 };

  it('true when bird overlaps pipe (hits top)', () => {
    const pipe: PipeState = { x: 75, gapY: 200, scored: false };
    expect(checkCollision(bird, pipe, cfg)).toBe(true);
  });

  it('true when bird hits bottom pipe', () => {
    const pipe: PipeState = { x: 75, gapY: 250, scored: false };
    expect(checkCollision(bird, pipe, cfg)).toBe(true);
  });

  it('false when bird in gap', () => {
    const pipe: PipeState = { x: 75, gapY: 310, scored: false };
    expect(checkCollision(bird, pipe, cfg)).toBe(false);
  });
});

describe('checkBounds', () => {
  it('true when bird hits ground (600h - 20 ground)', () => {
    expect(checkBounds({ x: 80, y: 565 }, 600)).toBe(true); // 565 + 20(BIRD_SIZE) >= 580(ground)
  });
  it('false when bird above ceiling (allowed)', () => {
    expect(checkBounds({ x: 80, y: -1 }, 600)).toBe(false); // can fly above
  });
  it('false when bird in bounds', () => {
    expect(checkBounds({ x: 80, y: 300 }, 600)).toBe(false);
  });
});

describe('generatePipeGapY', () => {
  it('result is within valid range', () => {
    for (let i = 0; i < 50; i++) {
      const y = generatePipeGapY(600, 120);
      expect(y).toBeGreaterThanOrEqual(100);
      expect(y).toBeLessThanOrEqual(500);
    }
  });
});

describe('shouldSpawnPipe', () => {
  it('true at correct intervals', () => {
    expect(shouldSpawnPipe(90, 90)).toBe(true);
    expect(shouldSpawnPipe(180, 90)).toBe(true);
    expect(shouldSpawnPipe(45, 90)).toBe(false);
    expect(shouldSpawnPipe(0, 90)).toBe(false);
  });
});

describe('isPastPipe', () => {
  it('true when bird has passed pipe', () => {
    expect(isPastPipe(150, 50, 50)).toBe(true);
    expect(isPastPipe(80, 100, 50)).toBe(false);
  });
});
