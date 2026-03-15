// ─── Value Objects ──────────────────────────

export interface Vec2 {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface GameConfig {
  canvasWidth: number;
  canvasHeight: number;
  gravity: number;
  flapStrength: number;
  pipeSpeed: number;
  pipeGap: number;
  pipeWidth: number;
  pipeSpawnInterval: number;
}

// ─── Game State (immutable snapshots) ──────

export interface BirdState {
  position: Vec2;
  velocity: number;
  rotation: number;
  alive: boolean;
}

export interface PipeState {
  x: number;
  gapY: number;
  scored: boolean;
}

export interface GameState {
  bird: BirdState;
  pipes: PipeState[];
  score: number;
  highScore: number;
  phase: 'ready' | 'playing' | 'gameover';
  tick: number;
}

// ─── Input Port (Primary / Driving) ────────

export interface IGamePort {
  /** Start or restart the game */
  start(): void;
  /** Process one game tick (called every frame) */
  tick(deltaMs: number): GameState;
  /** Bird flaps (player input) */
  flap(): void;
  /** Get current state without advancing */
  getState(): GameState;
}

// ─── Output Ports (Secondary / Driven) ─────

export interface IRenderPort {
  /** Initialize the rendering surface */
  init(config: GameConfig): Promise<void>;
  /** Render a frame from game state */
  render(state: GameState, config: GameConfig): void;
  /** Clean up resources */
  destroy(): void;
}

export interface IAudioPort {
  playFlap(): void;
  playScore(): void;
  playHit(): void;
}

export interface IStoragePort {
  loadHighScore(): Promise<number>;
  saveHighScore(score: number): Promise<void>;
}

export interface IInputPort {
  /** Register a callback for player input (tap/click/spacebar) */
  onFlap(callback: () => void): void;
  /** Start listening for input */
  start(): void;
  /** Stop listening */
  stop(): void;
}
