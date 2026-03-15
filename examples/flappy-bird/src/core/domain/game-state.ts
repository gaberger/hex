import type { GameConfig, GameState } from '../ports/index.js';
import {
  applyGravity, applyFlap, updatePosition, calculateRotation,
  checkCollision, checkBounds, generatePipeGapY, shouldSpawnPipe,
  isPastPipe, getBirdRect,
} from './physics.js';

const BIRD_START_X = 80;

export function createInitialState(config: GameConfig): GameState {
  return {
    bird: {
      position: { x: BIRD_START_X, y: config.canvasHeight / 2 },
      velocity: 0,
      rotation: 0,
      alive: true,
    },
    pipes: [],
    score: 0,
    highScore: 0,
    phase: 'ready',
    tick: 0,
  };
}

export function tickState(state: GameState, config: GameConfig, deltaMs: number): GameState {
  if (state.phase !== 'playing') return state;

  const newTick = state.tick + 1;
  const vel = applyGravity(state.bird.velocity, config.gravity, deltaMs);
  const pos = updatePosition(state.bird.position, vel, deltaMs);
  const rot = calculateRotation(vel);

  if (checkBounds(pos, config.canvasHeight)) {
    return gameOverState({ ...state, bird: { ...state.bird, position: pos, velocity: vel, rotation: rot } });
  }

  const birdRect = getBirdRect(pos);
  let pipes = state.pipes
    .map(p => ({ ...p, x: p.x - config.pipeSpeed * (deltaMs / 1000) }))
    .filter(p => p.x + config.pipeWidth > -10);

  for (const pipe of pipes) {
    if (checkCollision(birdRect, pipe, config)) {
      return gameOverState({ ...state, bird: { ...state.bird, position: pos, velocity: vel, rotation: rot }, pipes });
    }
  }

  let score = state.score;
  pipes = pipes.map(p => {
    if (!p.scored && isPastPipe(pos.x, p.x, config.pipeWidth)) {
      score += 1;
      return { ...p, scored: true };
    }
    return p;
  });

  if (shouldSpawnPipe(newTick, config.pipeSpawnInterval)) {
    pipes = [...pipes, { x: config.canvasWidth, gapY: generatePipeGapY(config.canvasHeight, config.pipeGap), scored: false }];
  }

  const highScore = Math.max(score, state.highScore);
  return { bird: { position: pos, velocity: vel, rotation: rot, alive: true }, pipes, score, highScore, phase: 'playing', tick: newTick };
}

export function flapState(state: GameState, config: GameConfig): GameState {
  if (state.phase !== 'playing') return state;
  return { ...state, bird: { ...state.bird, velocity: applyFlap(state.bird.velocity, config.flapStrength) } };
}

export function gameOverState(state: GameState): GameState {
  return { ...state, bird: { ...state.bird, alive: false }, phase: 'gameover' };
}

export function resetState(state: GameState, config: GameConfig): GameState {
  return { ...createInitialState(config), highScore: state.highScore };
}
