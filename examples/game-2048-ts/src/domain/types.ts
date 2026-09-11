/** A cell holds 0 for empty, or a power of two >= 2. */
export type Cell = number;

/** Sixteen cells, flat. Index `r * 4 + c` is row `r`, column `c`. */
export type Grid = readonly Cell[];

/** Four cells of one lane, before or after a slide. */
export type Row = readonly [Cell, Cell, Cell, Cell];

export type Direction = 'left' | 'right' | 'up' | 'down';

export type GameStatus = 'playing' | 'over';

export interface GameState {
  readonly grid: Grid;
  readonly score: number;
  /** Unsigned 32-bit. The luck lives in the state, so a replay is exact. */
  readonly seed: number;
  readonly moveCount: number;
  readonly status: GameStatus;
  /** Sticky: 2048 was reached at some point. Play continues. */
  readonly won: boolean;
}
