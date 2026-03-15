/**
 * Flappy Bird — Composition Root
 *
 * Wires ports to adapters and runs the game loop.
 */

import type { GameConfig } from './core/ports/index.js';
import { GameEngine } from './core/usecases/game-engine.js';
import { CanvasRenderer } from './adapters/secondary/canvas-renderer.js';
import { BrowserAudio } from './adapters/secondary/browser-audio.js';
import { LocalStorageAdapter } from './adapters/secondary/localstorage-adapter.js';
import { BrowserInput } from './adapters/primary/browser-input.js';

// ---------------------------------------------------------------------------
// Configuration — sign convention enforced here
// ---------------------------------------------------------------------------

const config: GameConfig = {
  canvasWidth: 400,
  canvasHeight: 600,
  gravity: 980,             // POSITIVE: accelerates bird downward
  flapStrength: -280,       // NEGATIVE: sets velocity upward
  pipeWidth: 52,
  pipeGap: 140,
  pipeSpeed: 160,
  pipeSpawnInterval: 1.8,
  groundHeight: 80,
  birdSize: 24,
  birdX: 80,
};

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

function main(): void {
  const canvas = document.getElementById('game-canvas') as HTMLCanvasElement;
  if (!canvas) throw new Error('Canvas element #game-canvas not found');

  canvas.width = config.canvasWidth;
  canvas.height = config.canvasHeight;

  const renderer = new CanvasRenderer(canvas);
  const audio = new BrowserAudio();
  const storage = new LocalStorageAdapter();
  const engine = new GameEngine(config, renderer, audio, storage);
  const input = new BrowserInput(canvas);

  engine.start();
  input.onAction(() => engine.flap());

  // Game loop
  let lastTime = 0;
  const MAX_DT = 1 / 30; // Cap delta to prevent physics tunneling

  function loop(timestamp: number): void {
    const dt = lastTime === 0 ? 1 / 60 : Math.min((timestamp - lastTime) / 1000, MAX_DT);
    lastTime = timestamp;
    engine.tick(dt);
    requestAnimationFrame(loop);
  }

  requestAnimationFrame(loop);
}

// Start when DOM is ready
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', main);
} else {
  main();
}
