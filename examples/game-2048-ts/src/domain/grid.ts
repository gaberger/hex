import type { Cell, Direction, Grid } from './types.ts';

/** Four cell positions, read in slide order: first position is the far side. */
export type Lane = readonly [number, number, number, number];

export const EMPTY_GRID: Grid = Object.freeze([
  0, 0, 0, 0,
  0, 0, 0, 0,
  0, 0, 0, 0,
  0, 0, 0, 0,
]);

/**
 * You write the slide once and turn the tray. Each lane lists the four
 * positions in the order the cells travel, so every direction is a left slide.
 * The tables are written out by hand; a generated table can share a bug with
 * the code that generates it.
 */
export const LANES: Readonly<Record<Direction, readonly Lane[]>> = Object.freeze({
  left: Object.freeze([
    [0, 1, 2, 3],
    [4, 5, 6, 7],
    [8, 9, 10, 11],
    [12, 13, 14, 15],
  ] as const),
  right: Object.freeze([
    [3, 2, 1, 0],
    [7, 6, 5, 4],
    [11, 10, 9, 8],
    [15, 14, 13, 12],
  ] as const),
  up: Object.freeze([
    [0, 4, 8, 12],
    [1, 5, 9, 13],
    [2, 6, 10, 14],
    [3, 7, 11, 15],
  ] as const),
  down: Object.freeze([
    [12, 8, 4, 0],
    [13, 9, 5, 1],
    [14, 10, 6, 2],
    [15, 11, 7, 3],
  ] as const),
});

export function gridEquals(a: Grid, b: Grid): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

/** Positions of the empty cells, in rising order. */
export function emptyCells(grid: Grid): number[] {
  const empties: number[] = [];
  for (let i = 0; i < grid.length; i += 1) {
    if (grid[i] === 0) empties.push(i);
  }
  return empties;
}

export function hasTile(grid: Grid, value: Cell): boolean {
  for (let i = 0; i < grid.length; i += 1) {
    if (grid[i] === value) return true;
  }
  return false;
}
