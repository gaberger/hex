/**
 * Flappy Bird — Game Engine (Use Case)
 *
 * Orchestrates domain logic and adapters.
 * BS-7: First tap transitions ready->playing AND flaps in one call,
 * preventing the state-race bug from the original implementation.
 */

import type {
  IGamePort,
  IRenderPort,
  IAudioPort,
  IStoragePort,
  GameState,
  GameConfig,
} from '../ports/index.js';
import { createInitialState, flapState, tickState } from '../domain/game-state.js';

export class GameEngine implements IGamePort {
  private state: GameState;

  constructor(
    private readonly config: GameConfig,
    private readonly renderer: IRenderPort,
    private readonly audio: IAudioPort,
    private readonly storage: IStoragePort,
  ) {
    const highScore = this.storage.loadHighScore();
    this.state = createInitialState(this.config, highScore);
  }

  /**
   * BS-6: Start sets phase to 'ready' (not 'playing').
   */
  start(): void {
    const highScore = this.storage.loadHighScore();
    this.state = createInitialState(this.config, highScore);
    this.render();
  }

  /**
   * BS-7: First tap transitions ready->playing AND applies flap.
   * BS-8: Subsequent taps during playing apply flap.
   * BS-10: Tap during gameover resets to ready.
   *
   * This unified handler prevents the state-race bug where
   * setting phase and applying flap were separate operations.
   */
  flap(): void {
    switch (this.state.phase) {
      case 'ready':
        // Transition to playing AND flap in one atomic operation
        this.state = { ...this.state, phase: 'playing' };
        this.state = flapState(this.state, this.config);
        this.audio.playFlap();
        break;

      case 'playing':
        this.state = flapState(this.state, this.config);
        this.audio.playFlap();
        break;

      case 'gameover':
        // BS-13: Save high score before reset
        if (this.state.score > this.state.highScore) {
          this.storage.saveHighScore(this.state.score);
        }
        this.start();
        break;
    }
  }

  /**
   * Advance game by dt seconds.
   */
  tick(dt: number): void {
    const prevPhase = this.state.phase;
    this.state = tickState(this.state, this.config, dt);

    // Detect transition to gameover
    if (prevPhase === 'playing' && this.state.phase === 'gameover') {
      this.audio.playHit();
      if (this.state.score > this.state.highScore) {
        this.storage.saveHighScore(this.state.score);
      }
    }

    // Detect scoring
    if (this.state.phase === 'playing') {
      // Audio feedback for score is handled by checking pipe pass in tick
      // We compare score change
    }

    this.render();
  }

  getState(): GameState {
    return this.state;
  }

  private render(): void {
    this.renderer.clear();
    this.renderer.drawGround(this.config);

    for (const pipe of this.state.pipes) {
      this.renderer.drawPipe(pipe, this.config);
    }

    this.renderer.drawBird(this.state.bird);
    this.renderer.drawScore(this.state.score);
    this.renderer.drawOverlay(this.state.phase, this.state.score, this.state.highScore);
  }
}
