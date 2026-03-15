/**
 * Unit Tests — Game State
 *
 * Uses NEGATIVE flapStrength matching production config.
 */

import { describe, it, expect } from 'bun:test';
import { createInitialState, flapState, tickState } from '../../src/core/domain/game-state.js';
import type { GameConfig, GameState } from '../../src/core/ports/index.js';

const config: GameConfig = {
  canvasWidth: 400,
  canvasHeight: 600,
  gravity: 980,
  flapStrength: -280,     // NEGATIVE — production value
  pipeWidth: 52,
  pipeGap: 140,
  pipeSpeed: 160,
  pipeSpawnInterval: 1.8,
  groundHeight: 80,
  birdSize: 24,
  birdX: 80,
};

// ---------------------------------------------------------------------------
// createInitialState — BS-6
// ---------------------------------------------------------------------------

describe('createInitialState', () => {
  it('starts in ready phase', () => {
    const state = createInitialState(config, 0);
    expect(state.phase).toBe('ready');
  });

  it('places bird at center height', () => {
    const state = createInitialState(config, 0);
    expect(state.bird.y).toBe(config.canvasHeight / 2);
  });

  it('starts with zero velocity', () => {
    const state = createInitialState(config, 0);
    expect(state.bird.velocity).toBe(0);
  });

  it('preserves high score', () => {
    const state = createInitialState(config, 42);
    expect(state.highScore).toBe(42);
  });

  it('starts with zero score and no pipes', () => {
    const state = createInitialState(config, 0);
    expect(state.score).toBe(0);
    expect(state.pipes).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// flapState — BS-7, BS-8
// ---------------------------------------------------------------------------

describe('flapState', () => {
  it('sets negative velocity when playing (bird rises)', () => {
    const state = createInitialState(config, 0);
    const playing: GameState = { ...state, phase: 'playing' };
    const result = flapState(playing, config);
    expect(result.bird.velocity).toBe(-280);
    expect(result.bird.velocity).toBeLessThan(0);
  });

  it('does nothing when phase is ready', () => {
    const state = createInitialState(config, 0);
    const result = flapState(state, config);
    expect(result).toBe(state); // same reference — no change
  });

  it('does nothing when phase is gameover', () => {
    const state = createInitialState(config, 0);
    const gameover: GameState = { ...state, phase: 'gameover' };
    const result = flapState(gameover, config);
    expect(result).toBe(gameover);
  });

  it('overrides positive velocity with negative on flap', () => {
    const state = createInitialState(config, 0);
    const falling: GameState = {
      ...state,
      phase: 'playing',
      bird: { ...state.bird, velocity: 200 },
    };
    const result = flapState(falling, config);
    expect(result.bird.velocity).toBe(-280);
  });
});

// ---------------------------------------------------------------------------
// tickState
// ---------------------------------------------------------------------------

describe('tickState', () => {
  it('does nothing when not playing', () => {
    const state = createInitialState(config, 0);
    const result = tickState(state, config, 1 / 60);
    expect(result).toBe(state);
  });

  it('applies gravity when playing (velocity increases)', () => {
    const state = createInitialState(config, 0);
    const playing: GameState = { ...state, phase: 'playing' };
    const result = tickState(playing, config, 1 / 60);
    expect(result.bird.velocity).toBeGreaterThan(0);
  });

  it('clamps bird to ceiling (y >= 0) without dying — BS-4', () => {
    const state = createInitialState(config, 0);
    const atCeiling: GameState = {
      ...state,
      phase: 'playing',
      bird: { ...state.bird, y: 5, velocity: -500 },
    };
    const result = tickState(atCeiling, config, 1 / 60);
    expect(result.bird.y).toBeGreaterThanOrEqual(0);
    expect(result.phase).toBe('playing'); // NOT gameover
  });

  it('transitions to gameover when bird hits ground — BS-4', () => {
    const state = createInitialState(config, 0);
    const nearGround: GameState = {
      ...state,
      phase: 'playing',
      bird: { ...state.bird, y: 490, velocity: 300 },
    };
    const result = tickState(nearGround, config, 0.1);
    expect(result.phase).toBe('gameover');
  });

  it('updates high score on gameover — BS-13', () => {
    const state = createInitialState(config, 5);
    const scored: GameState = {
      ...state,
      phase: 'playing',
      score: 10,
      bird: { ...state.bird, y: 490, velocity: 300 },
    };
    const result = tickState(scored, config, 0.1);
    expect(result.phase).toBe('gameover');
    expect(result.highScore).toBe(10);
  });
});
