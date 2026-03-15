import { describe, it, expect } from 'bun:test';
import { GameEngine } from '../../src/core/usecases/game-engine.js';
import type { GameConfig, IAudioPort, IStoragePort } from '../../src/core/ports/index.js';

const cfg: GameConfig = {
  canvasWidth: 400, canvasHeight: 600, gravity: 980,
  flapStrength: -300, pipeSpeed: 150, pipeGap: 120,
  pipeWidth: 50, pipeSpawnInterval: 90,
};

function mockAudio(): IAudioPort & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    playFlap() { calls.push('flap'); },
    playScore() { calls.push('score'); },
    playHit() { calls.push('hit'); },
  };
}

function mockStorage(highScore = 0): IStoragePort & { saved: number[] } {
  const saved: number[] = [];
  return {
    saved,
    loadHighScore: async () => highScore,
    saveHighScore: async (s: number) => { saved.push(s); },
  };
}

describe('GameEngine', () => {
  it('start() sets phase to playing', () => {
    const engine = new GameEngine(cfg, mockAudio(), mockStorage());
    engine.start();
    expect(engine.getState().phase).toBe('playing');
  });

  it('tick() advances game state', () => {
    const engine = new GameEngine(cfg, mockAudio(), mockStorage());
    engine.start();
    const before = engine.getState().tick;
    engine.tick(16);
    expect(engine.getState().tick).toBeGreaterThan(before);
  });

  it('flap() changes bird velocity', () => {
    const engine = new GameEngine(cfg, mockAudio(), mockStorage());
    engine.flap(); // starts game from ready
    expect(engine.getState().bird.velocity).toBe(-300);
  });

  it('getState() returns current state without mutation', () => {
    const engine = new GameEngine(cfg, mockAudio(), mockStorage());
    const a = engine.getState();
    const b = engine.getState();
    expect(a).toEqual(b);
  });

  it('score event plays score audio', () => {
    const audio = mockAudio();
    const engine = new GameEngine(cfg, audio, mockStorage());
    engine.start();
    // Place a pipe the bird has already passed
    const state = engine.getState();
    (engine as any).state = {
      ...state,
      bird: { ...state.bird, position: { x: 200, y: 300 } },
      pipes: [{ x: 100, gapY: 310, scored: false }],
    };
    engine.tick(16);
    expect(audio.calls).toContain('score');
  });

  it('collision plays hit audio and saves high score', () => {
    const audio = mockAudio();
    const storage = mockStorage(0);
    const engine = new GameEngine(cfg, audio, storage);
    engine.start();
    // Place bird right on a pipe to cause collision
    const state = engine.getState();
    (engine as any).state = {
      ...state, score: 5,
      pipes: [{ x: 75, gapY: 200, scored: false }],
    };
    engine.tick(16);
    expect(audio.calls).toContain('hit');
    expect(storage.saved.length).toBeGreaterThan(0);
  });
});
