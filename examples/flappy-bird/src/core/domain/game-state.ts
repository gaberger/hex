/**
 * Flappy Bird — Game State Management
 *
 * Pure functions that transition game state.
 * All functions are immutable — they return new state objects.
 */

import type { GameState, GameConfig, Bird, Pipe } from './types.js';
import {
  applyGravity,
  applyFlap,
  applyVelocity,
  calculateRotation,
  checkBounds,
  checkPipeCollision,
  checkPipePass,
} from './physics.js';

// ---------------------------------------------------------------------------
// State creation
// ---------------------------------------------------------------------------

/**
 * BS-6: Game starts in 'ready' state.
 */
export function createInitialState(config: GameConfig, highScore: number): GameState {
  return {
    bird: {
      x: config.birdX,
      y: config.canvasHeight / 2,
      velocity: 0,
      rotation: 0,
    },
    pipes: [],
    phase: 'ready',
    score: 0,
    highScore,
    elapsed: 0,
  };
}

// ---------------------------------------------------------------------------
// State transitions
// ---------------------------------------------------------------------------

/**
 * BS-7 / BS-8: Apply flap during 'playing' phase.
 * Sets bird velocity to flapStrength (already negative per sign convention).
 */
export function flapState(state: GameState, config: GameConfig): GameState {
  if (state.phase !== 'playing') {
    return state;
  }
  const newVelocity = applyFlap(config.flapStrength);
  return {
    ...state,
    bird: {
      ...state.bird,
      velocity: newVelocity,
      rotation: calculateRotation(newVelocity),
    },
  };
}

/**
 * Advance the game by one tick (dt seconds).
 * Handles physics, pipe spawning, collision, and scoring.
 */
export function tickState(state: GameState, config: GameConfig, dt: number): GameState {
  if (state.phase !== 'playing') {
    return state;
  }

  // Physics: gravity + velocity
  const newVelocity = applyGravity(state.bird.velocity, config.gravity, dt);
  let newY = applyVelocity(state.bird.y, newVelocity, dt);

  // Clamp to ceiling (y >= 0) but do NOT kill — BS-4
  newY = Math.max(0, newY);

  const newRotation = calculateRotation(newVelocity);

  const newBird: Bird = {
    ...state.bird,
    y: newY,
    velocity: newVelocity,
    rotation: newRotation,
  };

  // Ground collision — BS-4
  if (checkBounds(newBird, config)) {
    return {
      ...state,
      bird: newBird,
      phase: 'gameover',
      highScore: Math.max(state.score, state.highScore),
    };
  }

  // Move pipes and check collisions
  const newElapsed = state.elapsed + dt;
  let newPipes = movePipes(state.pipes, config, dt);

  // Spawn pipes at interval
  if (shouldSpawnPipe(state.elapsed, newElapsed, config.pipeSpawnInterval)) {
    newPipes = [...newPipes, createPipe(config)];
  }

  // Pipe collision — BS-5
  for (const pipe of newPipes) {
    if (checkPipeCollision(newBird, pipe, config)) {
      return {
        ...state,
        bird: newBird,
        pipes: newPipes,
        phase: 'gameover',
        elapsed: newElapsed,
        highScore: Math.max(state.score, state.highScore),
      };
    }
  }

  // Scoring — BS-11
  let newScore = state.score;
  newPipes = newPipes.map((pipe) => {
    if (checkPipePass(newBird, pipe)) {
      newScore += 1;
      return { ...pipe, scored: true };
    }
    return pipe;
  });

  return {
    bird: newBird,
    pipes: newPipes,
    phase: 'playing',
    score: newScore,
    highScore: Math.max(newScore, state.highScore),
    elapsed: newElapsed,
  };
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function movePipes(pipes: Pipe[], config: GameConfig, dt: number): Pipe[] {
  return pipes
    .map((p) => ({ ...p, x: p.x - config.pipeSpeed * dt }))
    .filter((p) => p.x + config.pipeWidth > 0);
}

function shouldSpawnPipe(prevElapsed: number, newElapsed: number, interval: number): boolean {
  return Math.floor(newElapsed / interval) > Math.floor(prevElapsed / interval);
}

function createPipe(config: GameConfig): Pipe {
  const minGapY = 60;
  const maxGapY = config.canvasHeight - config.groundHeight - config.pipeGap - 60;
  const gapY = minGapY + Math.random() * (maxGapY - minGapY);
  return {
    x: config.canvasWidth,
    gapY,
    scored: false,
  };
}
