import test from 'node:test';
import assert from 'node:assert/strict';

import { slideGrid } from './domain/game.ts';
import { EMPTY_GRID, LANES, emptyCells, gridEquals, hasTile } from './domain/grid.ts';
import type { Direction, Grid } from './domain/types.ts';

// One asymmetric board. A symmetric one cannot tell a correct lane table from
// a backwards one.
const BOARD: Grid = [
  2, 2, 0, 4,
  0, 4, 4, 0,
  2, 0, 0, 2,
  0, 0, 8, 8,
];

// Test 9: four hand-written expected results, one per direction.
const EXPECTED: Readonly<Record<Direction, { grid: Grid; gained: number }>> = {
  left: {
    grid: [
      4, 4, 0, 0,
      8, 0, 0, 0,
      4, 0, 0, 0,
      16, 0, 0, 0,
    ],
    gained: 32,
  },
  right: {
    grid: [
      0, 0, 4, 4,
      0, 0, 0, 8,
      0, 0, 0, 4,
      0, 0, 0, 16,
    ],
    gained: 32,
  },
  up: {
    grid: [
      4, 2, 4, 4,
      0, 4, 8, 2,
      0, 0, 0, 8,
      0, 0, 0, 0,
    ],
    gained: 4,
  },
  down: {
    grid: [
      0, 0, 0, 0,
      0, 0, 0, 4,
      0, 2, 4, 2,
      4, 4, 8, 8,
    ],
    gained: 4,
  },
};

for (const direction of ['left', 'right', 'up', 'down'] as const) {
  test(`the board slides ${direction} to a hand-written result`, () => {
    const result = slideGrid(BOARD, direction);
    assert.deepEqual(result.grid, EXPECTED[direction].grid);
    assert.equal(result.gained, EXPECTED[direction].gained);
    assert.equal(result.changed, true);
  });
}

// Test 6: merge direction on a right slide.
test('a right slide merges the RIGHT pair', () => {
  const grid: Grid = [
    2, 2, 2, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
  ];
  const result = slideGrid(grid, 'right');
  assert.deepEqual(result.grid.slice(0, 4), [0, 0, 2, 4]);
  assert.notDeepEqual(result.grid.slice(0, 4), [0, 0, 4, 2]);
});

test('an up slide merges the TOP pair', () => {
  const grid: Grid = [
    2, 0, 0, 0,
    2, 0, 0, 0,
    2, 0, 0, 0,
    0, 0, 0, 0,
  ];
  const result = slideGrid(grid, 'up');
  assert.deepEqual([result.grid[0], result.grid[4], result.grid[8], result.grid[12]], [4, 2, 0, 0]);
});

test('a down slide merges the BOTTOM pair', () => {
  const grid: Grid = [
    2, 0, 0, 0,
    2, 0, 0, 0,
    2, 0, 0, 0,
    0, 0, 0, 0,
  ];
  const result = slideGrid(grid, 'down');
  assert.deepEqual([result.grid[0], result.grid[4], result.grid[8], result.grid[12]], [0, 0, 2, 4]);
});

// Test 10: each lane table covers every cell exactly once.
test('each lane table visits all sixteen cells exactly once', () => {
  for (const direction of ['left', 'right', 'up', 'down'] as const) {
    const flat = LANES[direction].flatMap((lane) => [...lane]);
    assert.equal(flat.length, 16, `${direction} must list sixteen positions`);
    const sorted = [...flat].sort((a, b) => a - b);
    const all = Array.from({ length: 16 }, (_unused, i) => i);
    assert.deepEqual(sorted, all, `${direction} must cover every cell once`);
  }
});

test('right and down read their lanes backwards from left and up', () => {
  for (let lane = 0; lane < 4; lane += 1) {
    assert.deepEqual([...LANES.right[lane]], [...LANES.left[lane]].reverse());
    assert.deepEqual([...LANES.down[lane]], [...LANES.up[lane]].reverse());
  }
});

test('a slide that changes nothing reports changed false', () => {
  const grid: Grid = [
    2, 4, 8, 16,
    4, 2, 16, 8,
    2, 4, 8, 16,
    4, 2, 16, 8,
  ];
  const result = slideGrid(grid, 'left');
  assert.equal(result.changed, false);
  assert.equal(result.gained, 0);
  assert.deepEqual(result.grid, grid);
});

test('the empty grid holds sixteen zeros and is frozen', () => {
  assert.equal(EMPTY_GRID.length, 16);
  assert.equal(EMPTY_GRID.every((cell) => cell === 0), true);
  assert.equal(Object.isFrozen(EMPTY_GRID), true);
});

test('gridEquals compares cell by cell', () => {
  const a: Grid = [...EMPTY_GRID];
  const b: Grid = [...EMPTY_GRID];
  assert.equal(gridEquals(a, b), true);
  const c = [...EMPTY_GRID];
  c[7] = 2;
  assert.equal(gridEquals(a, c), false);
});

test('emptyCells lists the free positions in rising order', () => {
  const grid: Grid = [
    0, 2, 0, 4,
    2, 2, 2, 2,
    2, 2, 2, 2,
    2, 2, 2, 0,
  ];
  assert.deepEqual(emptyCells(grid), [0, 2, 15]);
  assert.deepEqual(emptyCells(EMPTY_GRID), Array.from({ length: 16 }, (_unused, i) => i));
});

test('hasTile finds a value, and reports a missing one', () => {
  const grid = [...EMPTY_GRID];
  grid[5] = 2048;
  assert.equal(hasTile(grid, 2048), true);
  assert.equal(hasTile(grid, 1024), false);
});
