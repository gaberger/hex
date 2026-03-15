import { createInitialState, tickState, flapState, resetState } from './core/domain/game-state.js';
import { CanvasRenderer } from './adapters/secondary/canvas-renderer.js';
import { BrowserAudio } from './adapters/secondary/browser-audio.js';
import { LocalStorageAdapter } from './adapters/secondary/localstorage-adapter.js';
import { BrowserInput } from './adapters/primary/browser-input.js';
import type { GameConfig, GameState } from './core/ports/index.js';

const config: GameConfig = {
  canvasWidth: 400,
  canvasHeight: 600,
  gravity: 980,
  flapStrength: -280,
  pipeSpeed: 120,
  pipeGap: 150,
  pipeWidth: 60,
  pipeSpawnInterval: 90,
};

const audio = new BrowserAudio();
const storage = new LocalStorageAdapter();
const renderer = new CanvasRenderer();
const input = new BrowserInput();

let state: GameState = createInitialState(config);
let prevScore = 0;

storage.loadHighScore().then(hs => { state = { ...state, highScore: hs }; });

input.onFlap(() => {
  if (state.phase === 'ready') {
    state = { ...state, phase: 'playing' };
    audio.playFlap();
    state = flapState(state, config);
  } else if (state.phase === 'playing') {
    audio.playFlap();
    state = flapState(state, config);
  } else {
    state = resetState(state, config);
  }
});

await renderer.init(config);
input.start();

let lastTime = performance.now();
prevScore = 0;

function loop(now: number): void {
  const delta = now - lastTime;
  lastTime = now;

  state = tickState(state, config, delta);

  if (state.score > prevScore) {
    audio.playScore();
    prevScore = state.score;
  }

  if (state.phase === 'gameover' && !state.bird.alive) {
    audio.playHit();
    storage.saveHighScore(state.highScore);
    // Mark alive to avoid repeated hit sounds
    state = { ...state, bird: { ...state.bird, alive: true } };
  }

  renderer.render(state, config);
  requestAnimationFrame(loop);
}

requestAnimationFrame(loop);
