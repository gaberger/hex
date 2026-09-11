import { EMPTY_GRID, LANES, emptyCells, gridEquals, hasTile } from './grid.ts';
import { randomBelow } from './rng.ts';
import { slideRowLeft } from './row.ts';
import { freezeState } from './state.ts';
import type { Direction, GameState, Grid, Row } from './types.ts';

export type MoveOutcome = 'moved' | 'blocked' | 'rejected';

export interface MoveResult {
  readonly state: GameState;
  readonly outcome: MoveOutcome;
  readonly gained: number;
  readonly spawnedAt: number | null;
  readonly spawnedValue: number | null;
}

export interface SlideGridResult {
  readonly grid: Grid;
  readonly gained: number;
  readonly changed: boolean;
}

export interface SpawnResult {
  readonly grid: Grid;
  readonly seed: number;
  readonly index: number;
  readonly value: number;
}

/** Slide all four lanes of one direction. The lane table does the turning. */
export function slideGrid(grid: Grid, direction: Direction): SlideGridResult {
  const next = grid.slice();
  let gained = 0;

  for (const lane of LANES[direction]) {
    const row: Row = [grid[lane[0]], grid[lane[1]], grid[lane[2]], grid[lane[3]]];
    const slid = slideRowLeft(row);
    gained += slid.gained;
    for (let i = 0; i < 4; i += 1) {
      next[lane[i]] = slid.row[i];
    }
  }

  return { grid: next, gained, changed: !gridEquals(grid, next) };
}

/**
 * Put one new tile on the board. This never loops.
 *
 * It lists the empty cells, draws one cell, then draws the value. Guess and
 * retry would spin forever on a full board, so it is banned. The draw order is
 * fixed: cell first, value second.
 */
export function spawnTile(grid: Grid, seed: number): SpawnResult | null {
  const empties = emptyCells(grid);
  if (empties.length === 0) return null;

  const cellDraw = randomBelow(seed, empties.length);
  const index = empties[cellDraw.value];

  const valueDraw = randomBelow(cellDraw.seed, 10);
  const value = valueDraw.value === 0 ? 4 : 2;

  const next = grid.slice();
  next[index] = value;
  return { grid: next, seed: valueDraw.seed, index, value };
}

/**
 * The game is over when no move can change the board.
 *
 * It stops at the first "no": any empty cell, any equal right neighbour, any
 * equal lower neighbour. Left and up ask the same question twice.
 */
export function isGameOver(grid: Grid): boolean {
  for (let i = 0; i < grid.length; i += 1) {
    if (grid[i] === 0) return false;
  }
  for (let r = 0; r < 4; r += 1) {
    for (let c = 0; c < 3; c += 1) {
      if (grid[r * 4 + c] === grid[r * 4 + c + 1]) return false;
    }
  }
  for (let r = 0; r < 3; r += 1) {
    for (let c = 0; c < 4; c += 1) {
      if (grid[r * 4 + c] === grid[(r + 1) * 4 + c]) return false;
    }
  }
  return true;
}

/** A fresh board with two tiles on it. */
export function newGame(seed: number): GameState {
  let grid: Grid = EMPTY_GRID;
  let carried = seed >>> 0;

  for (let i = 0; i < 2; i += 1) {
    const spawn = spawnTile(grid, carried);
    if (spawn === null) throw new Error('cannot start a game: the board is full');
    grid = spawn.grid;
    carried = spawn.seed;
  }

  return freezeState({
    grid,
    score: 0,
    seed: carried,
    moveCount: 0,
    status: isGameOver(grid) ? 'over' : 'playing',
    won: hasTile(grid, 2048),
  });
}

/**
 * Play one move. The result says what happened, so a caller never has to
 * compare two boards to find out.
 */
export function applyMove(state: GameState, direction: Direction): MoveResult {
  // 1. A dead game accepts no moves.
  if (state.status === 'over') {
    return {
      state: freezeState(state),
      outcome: 'rejected',
      gained: 0,
      spawnedAt: null,
      spawnedValue: null,
    };
  }

  // 2. Slide all four lanes.
  const slid = slideGrid(state.grid, direction);

  // 3. Nothing moved, so nothing happens: no spawn, no score, no write.
  if (!slid.changed) {
    return {
      state: freezeState(state),
      outcome: 'blocked',
      gained: 0,
      spawnedAt: null,
      spawnedValue: null,
    };
  }

  // 4. A changed grid always has an empty cell, so null here is a bug.
  const spawn = spawnTile(slid.grid, state.seed);
  if (spawn === null) {
    throw new Error('a changed grid must have an empty cell');
  }
  const grid = spawn.grid;

  // 5 and 6. Score, count, sticky win, then the status, after the spawn.
  //    A spawn can fill the last cell and end the game.
  const next = freezeState({
    grid,
    score: state.score + slid.gained,
    seed: spawn.seed,
    moveCount: state.moveCount + 1,
    status: isGameOver(grid) ? 'over' : 'playing',
    won: state.won || hasTile(grid, 2048),
  });

  // 7. Frozen, and reported.
  return {
    state: next,
    outcome: 'moved',
    gained: slid.gained,
    spawnedAt: spawn.index,
    spawnedValue: spawn.value,
  };
}
