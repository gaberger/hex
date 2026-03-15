import type { GameConfig, GameState, IGamePort, IAudioPort, IStoragePort } from '../ports/index.js';
import { createInitialState, tickState, flapState, resetState, gameOverState } from '../domain/game-state.js';
import { checkCollision, checkBounds, getBirdRect, isPastPipe } from '../domain/physics.js';

export class GameEngine implements IGamePort {
  private state: GameState;

  constructor(
    private readonly config: GameConfig,
    private readonly audio: IAudioPort,
    private readonly storage: IStoragePort,
  ) {
    this.state = createInitialState(config);
  }

  start(): void {
    const highScore = this.state.highScore;
    this.state = resetState(this.state, this.config);
    this.state = { ...this.state, phase: 'playing', highScore };

    this.storage.loadHighScore().then(saved => {
      if (this.state.phase === 'playing') {
        this.state = { ...this.state, highScore: Math.max(this.state.highScore, saved) };
      }
    });
  }

  tick(deltaMs: number): GameState {
    if (this.state.phase !== 'playing') {
      return this.state;
    }

    const prevScore = this.state.score;
    const prevPhase = this.state.phase;

    this.state = tickState(this.state, this.config, deltaMs);

    if (this.state.score > prevScore) {
      this.audio.playScore();
    }

    if (this.state.phase === 'gameover' && prevPhase === 'playing') {
      this.audio.playHit();
      if (this.state.score > this.state.highScore) {
        this.state = { ...this.state, highScore: this.state.score };
      }
      this.storage.saveHighScore(this.state.highScore);
    }

    return this.state;
  }

  flap(): void {
    if (this.state.phase === 'ready') {
      this.start();
    }

    if (this.state.phase === 'playing') {
      this.state = flapState(this.state, this.config);
      this.audio.playFlap();
    }
  }

  getState(): GameState {
    return this.state;
  }
}
