/**
 * Unit Tests — Game Engine (Use Case)
 *
 * Tests the unified flap handler that prevents the state-race bug (Bug #3).
 */

import { describe, it, expect } from 'bun:test';
import { GameEngine } from '../../src/core/usecases/game-engine.js';
import type { IRenderPort, IAudioPort, IStoragePort, GameConfig } from '../../src/core/ports/index.js';

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

// Mock adapters
function mockRenderer(): IRenderPort {
  return {
    clear: () => {},
    drawBird: () => {},
    drawPipe: () => {},
    drawGround: () => {},
    drawScore: () => {},
    drawOverlay: () => {},
  };
}

function mockAudio(): IAudioPort & { flapCount: number; hitCount: number } {
  return {
    flapCount: 0,
    hitCount: 0,
    playFlap() { this.flapCount++; },
    playScore() {},
    playHit() { this.hitCount++; },
  };
}

function mockStorage(initialHighScore = 0): IStoragePort & { saved: number | null } {
  return {
    saved: null,
    loadHighScore: () => initialHighScore,
    saveHighScore(score: number) { this.saved = score; },
  };
}

// ---------------------------------------------------------------------------
// start — BS-6
// ---------------------------------------------------------------------------

describe('GameEngine.start', () => {
  it('initializes in ready phase', () => {
    const engine = new GameEngine(config, mockRenderer(), mockAudio(), mockStorage());
    engine.start();
    expect(engine.getState().phase).toBe('ready');
  });
});

// ---------------------------------------------------------------------------
// flap — BS-7: first tap transitions ready->playing AND flaps
// ---------------------------------------------------------------------------

describe('GameEngine.flap', () => {
  it('transitions from ready to playing on first flap — BS-7', () => {
    const engine = new GameEngine(config, mockRenderer(), mockAudio(), mockStorage());
    engine.start();
    engine.flap();
    expect(engine.getState().phase).toBe('playing');
  });

  it('applies upward velocity on first flap (not just phase change) — BS-7', () => {
    const engine = new GameEngine(config, mockRenderer(), mockAudio(), mockStorage());
    engine.start();
    engine.flap();
    const state = engine.getState();
    expect(state.bird.velocity).toBe(-280);
    expect(state.bird.velocity).toBeLessThan(0);
  });

  it('plays flap audio on first tap', () => {
    const audio = mockAudio();
    const engine = new GameEngine(config, mockRenderer(), audio, mockStorage());
    engine.start();
    engine.flap();
    expect(audio.flapCount).toBe(1);
  });

  it('applies flap during playing — BS-8', () => {
    const engine = new GameEngine(config, mockRenderer(), mockAudio(), mockStorage());
    engine.start();
    engine.flap(); // ready -> playing
    // Tick to let gravity pull bird down
    engine.tick(0.1);
    const velocityBefore = engine.getState().bird.velocity;
    expect(velocityBefore).toBeGreaterThan(-280); // gravity pulled it up from -280
    engine.flap(); // playing -> still playing, velocity reset
    expect(engine.getState().bird.velocity).toBe(-280);
  });

  it('resets game on flap during gameover — BS-10', () => {
    const engine = new GameEngine(config, mockRenderer(), mockAudio(), mockStorage());
    engine.start();
    engine.flap(); // start playing

    // Force gameover by ticking until ground hit
    for (let i = 0; i < 200; i++) {
      engine.tick(1 / 60);
      if (engine.getState().phase === 'gameover') break;
    }
    expect(engine.getState().phase).toBe('gameover');

    engine.flap(); // gameover -> ready
    expect(engine.getState().phase).toBe('ready');
  });
});

// ---------------------------------------------------------------------------
// tick
// ---------------------------------------------------------------------------

describe('GameEngine.tick', () => {
  it('does not change state in ready phase', () => {
    const engine = new GameEngine(config, mockRenderer(), mockAudio(), mockStorage());
    engine.start();
    const before = engine.getState();
    engine.tick(1 / 60);
    const after = engine.getState();
    expect(after.bird.y).toBe(before.bird.y);
  });

  it('bird falls under gravity during play', () => {
    const engine = new GameEngine(config, mockRenderer(), mockAudio(), mockStorage());
    engine.start();
    engine.flap();
    const yAfterFlap = engine.getState().bird.y;
    // Tick multiple times to let gravity win
    for (let i = 0; i < 60; i++) {
      engine.tick(1 / 60);
      if (engine.getState().phase !== 'playing') break;
    }
    expect(engine.getState().bird.y).toBeGreaterThan(yAfterFlap);
  });
});
