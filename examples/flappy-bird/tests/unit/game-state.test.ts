import { describe, it, expect } from 'bun:test';
import {
  createInitialState, tickState, flapState, gameOverState, resetState,
} from '../../src/core/domain/game-state.js';
import type { GameConfig } from '../../src/core/ports/index.js';

const cfg: GameConfig = {
  canvasWidth: 400, canvasHeight: 600, gravity: 980,
  flapStrength: -300, pipeSpeed: 150, pipeGap: 120,
  pipeWidth: 50, pipeSpawnInterval: 90,
};

describe('createInitialState', () => {
  it('bird at starting position, score 0, phase ready', () => {
    const s = createInitialState(cfg);
    expect(s.bird.position).toEqual({ x: 80, y: 300 });
    expect(s.score).toBe(0);
    expect(s.phase).toBe('ready');
    expect(s.pipes).toEqual([]);
  });
});

describe('tickState', () => {
  const playing = { ...createInitialState(cfg), phase: 'playing' as const };

  it('bird falls due to gravity when playing', () => {
    const next = tickState(playing, cfg, 16);
    expect(next.bird.velocity).toBeGreaterThan(0);
  });

  it('pipes move left', () => {
    const withPipe = { ...playing, pipes: [{ x: 300, gapY: 300, scored: false }] };
    const next = tickState(withPipe, cfg, 16);
    expect(next.pipes[0].x).toBeLessThan(300);
  });

  it('score increments when bird passes pipe', () => {
    const withPipe = {
      ...playing,
      bird: { ...playing.bird, position: { x: 200, y: 300 } },
      pipes: [{ x: 100, gapY: 310, scored: false }],
    };
    const next = tickState(withPipe, cfg, 16);
    expect(next.score).toBe(1);
  });

  it('transitions to gameover on collision', () => {
    const withPipe = {
      ...playing,
      pipes: [{ x: 75, gapY: 200, scored: false }],
    };
    const next = tickState(withPipe, cfg, 16);
    expect(next.phase).toBe('gameover');
  });

  it('returns same state if not playing', () => {
    const ready = createInitialState(cfg);
    expect(tickState(ready, cfg, 16)).toBe(ready);
  });
});

describe('flapState', () => {
  const playing = { ...createInitialState(cfg), phase: 'playing' as const };

  it('bird velocity becomes negative (upward)', () => {
    const next = flapState(playing, cfg);
    expect(next.bird.velocity).toBe(-300);
  });

  it('only works in playing phase', () => {
    const ready = createInitialState(cfg);
    expect(flapState(ready, cfg)).toBe(ready);
  });
});

describe('gameOverState', () => {
  it('preserves score, sets phase', () => {
    const s = { ...createInitialState(cfg), score: 5, phase: 'playing' as const };
    const over = gameOverState(s);
    expect(over.phase).toBe('gameover');
    expect(over.score).toBe(5);
    expect(over.bird.alive).toBe(false);
  });
});

describe('resetState', () => {
  it('returns to initial with high score preserved', () => {
    const s = { ...createInitialState(cfg), highScore: 10, phase: 'gameover' as const };
    const next = resetState(s, cfg);
    expect(next.phase).toBe('ready');
    expect(next.score).toBe(0);
    expect(next.highScore).toBe(10);
  });
});
