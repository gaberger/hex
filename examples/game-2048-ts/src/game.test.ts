import test from 'node:test';
import assert from 'node:assert/strict';

import { applyMove, isGameOver, newGame, spawnTile } from './domain/game.ts';
import { freezeState } from './domain/state.ts';
import type { Direction, GameState, Grid } from './domain/types.ts';

const DIRECTIONS: readonly Direction[] = ['left', 'right', 'up', 'down'];

/** A full board where no two neighbours are equal. Nothing can move. */
const DEAD_BOARD: Grid = [
  2, 4, 8, 16,
  4, 2, 16, 8,
  2, 4, 8, 16,
  4, 2, 16, 8,
];

function stateOf(grid: Grid, extra: Partial<GameState> = {}): GameState {
  return freezeState({
    grid: [...grid],
    score: 0,
    seed: 123456,
    moveCount: 0,
    status: isGameOver(grid) ? 'over' : 'playing',
    won: false,
    ...extra,
  });
}

function sumOf(grid: Grid): number {
  return grid.reduce((total, cell) => total + cell, 0);
}

// ---------------------------------------------------------------------------
// Test 4 and 11: game-over detection, on boards built full on purpose.
// ---------------------------------------------------------------------------

test('a full board with no equal neighbours is over', () => {
  assert.equal(isGameOver(DEAD_BOARD), true);
  assert.equal(stateOf(DEAD_BOARD).status, 'over');
});

test('lowering one cell to make a pair brings the board back to life', () => {
  const alive = [...DEAD_BOARD];
  alive[1] = 2; // row 0 becomes 2 2 8 16, so a left merge exists.
  assert.equal(isGameOver(alive), false);
  assert.equal(stateOf(alive).status, 'playing');
});

test('one empty cell is enough to keep the game alive', () => {
  const alive = [...DEAD_BOARD];
  alive[15] = 0;
  assert.equal(isGameOver(alive), false);
});

// Test 12: a hand-written second opinion. The verdicts are written by hand and
// not produced by applyMove, which is the code under test.
const VERDICTS: readonly { readonly name: string; readonly grid: Grid; readonly over: boolean }[] = [
  { name: 'full, no equal neighbour anywhere', grid: DEAD_BOARD, over: true },
  {
    name: 'full, one pair in the top row',
    grid: [2, 2, 8, 16, 4, 2, 16, 8, 2, 4, 8, 16, 4, 2, 16, 8],
    over: false,
  },
  {
    name: 'full, one pair in the bottom-right corner',
    grid: [2, 4, 8, 16, 4, 2, 16, 8, 2, 4, 8, 16, 4, 2, 8, 8],
    over: false,
  },
  {
    name: 'full, the only pair stands upright in the last column',
    grid: [2, 4, 8, 16, 4, 2, 16, 8, 2, 4, 8, 16, 4, 2, 16, 16],
    over: false,
  },
  {
    name: 'full, the only pair stands upright in the first column',
    grid: [2, 4, 8, 16, 4, 2, 16, 8, 4, 4, 8, 16, 2, 2, 16, 8],
    over: false,
  },
  { name: 'a board with one empty cell', grid: [...DEAD_BOARD.slice(0, 15), 0], over: false },
  {
    name: 'an empty board',
    grid: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    over: false,
  },
  {
    name: 'full, every tile the same',
    grid: [2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2],
    over: false,
  },
];

for (const verdict of VERDICTS) {
  test(`game over table: ${verdict.name} -> ${verdict.over ? 'over' : 'playing'}`, () => {
    assert.equal(isGameOver(verdict.grid), verdict.over);
  });
}

test('the game over table and the move rule agree', () => {
  // A second opinion built the other way round: if every direction is blocked,
  // the board is over.
  //
  // This holds only once at least one tile is on the board. An empty board has
  // free cells, so it is not over, but no slide can change it either. A real
  // game never reaches that state, because newGame always places two tiles.
  for (const verdict of VERDICTS) {
    if (verdict.grid.every((cell) => cell === 0)) continue;
    const state = freezeState({
      grid: [...verdict.grid],
      score: 0,
      seed: 5,
      moveCount: 0,
      status: 'playing',
      won: false,
    });
    const anyMoveWorks = DIRECTIONS.some(
      (direction) => applyMove(state, direction).outcome === 'moved',
    );
    assert.equal(anyMoveWorks, !verdict.over, `disagreement on: ${verdict.name}`);
  }
});

// ---------------------------------------------------------------------------
// Test 3 (domain half) and 18: blocked and rejected moves.
// ---------------------------------------------------------------------------

// Full, no horizontal pair, so a left move is blocked. Column 0 holds 2 above
// 2, so the game is still alive.
const LEFT_BLOCKED: Grid = [
  2, 4, 8, 16,
  2, 16, 4, 8,
  4, 2, 16, 2,
  8, 4, 2, 16,
];

test('a move that changes nothing is blocked, and changes no field', () => {
  const before = stateOf(LEFT_BLOCKED, { score: 100, seed: 999, moveCount: 7 });
  assert.equal(before.status, 'playing');

  const result = applyMove(before, 'left');

  assert.equal(result.outcome, 'blocked');
  assert.equal(result.gained, 0);
  assert.equal(result.spawnedAt, null);
  assert.equal(result.spawnedValue, null);
  assert.deepEqual(result.state.grid, before.grid);
  assert.equal(result.state.score, 100);
  assert.equal(result.state.seed, 999);
  assert.equal(result.state.moveCount, 7);
});

// Test 18.
test('a game that is over rejects every move and stays unchanged', () => {
  const dead = stateOf(DEAD_BOARD, { score: 4242, seed: 777, moveCount: 31 });
  assert.equal(dead.status, 'over');

  for (const direction of DIRECTIONS) {
    const result = applyMove(dead, direction);
    assert.equal(result.outcome, 'rejected');
    assert.equal(result.gained, 0);
    assert.equal(result.spawnedAt, null);
    assert.equal(result.spawnedValue, null);
    assert.deepEqual(result.state.grid, dead.grid);
    assert.equal(result.state.score, 4242);
    assert.equal(result.state.seed, 777);
    assert.equal(result.state.moveCount, 31);
  }
});

// ---------------------------------------------------------------------------
// Test 16: spawn behaviour.
// ---------------------------------------------------------------------------

test('spawn on a full board returns null, and does not hang', () => {
  assert.equal(spawnTile(DEAD_BOARD, 1), null);
});

test('spawn puts exactly one tile on an empty cell', () => {
  const grid: Grid = [
    2, 4, 8, 16,
    4, 2, 16, 8,
    2, 4, 8, 16,
    4, 2, 16, 0,
  ];
  const spawn = spawnTile(grid, 55);
  assert.notEqual(spawn, null);
  assert.equal(spawn?.index, 15);
  assert.equal(spawn?.value === 2 || spawn?.value === 4, true);
  assert.equal(sumOf(spawn?.grid ?? []), sumOf(grid) + (spawn?.value ?? 0));
});

test('spawn only ever places a 2 or a 4', () => {
  let seed = 3;
  const values = new Set<number>();
  for (let i = 0; i < 500; i += 1) {
    const spawn = spawnTile([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], seed);
    assert.notEqual(spawn, null);
    if (spawn !== null) {
      values.add(spawn.value);
      seed = spawn.seed;
    }
  }
  assert.deepEqual([...values].sort((a, b) => a - b), [2, 4]);
});

// ---------------------------------------------------------------------------
// Test 23: a new game.
// ---------------------------------------------------------------------------

test('a new game starts with exactly two tiles, each a 2 or a 4', () => {
  for (let seed = 0; seed < 300; seed += 1) {
    const state = newGame(seed);
    const tiles = state.grid.filter((cell) => cell !== 0);
    assert.equal(tiles.length, 2, `seed ${seed} did not place two tiles`);
    for (const tile of tiles) {
      assert.equal(tile === 2 || tile === 4, true, `seed ${seed} placed a ${tile}`);
    }
    assert.equal(state.score, 0);
    assert.equal(state.moveCount, 0);
    assert.equal(state.status, 'playing');
    assert.equal(state.won, false);
  }
});

// ---------------------------------------------------------------------------
// Test 13: the sum rule. It cannot tell left from right, and it is kept anyway.
// ---------------------------------------------------------------------------

test('the board total grows by exactly the spawned value on every move', () => {
  let state = newGame(20260911);
  for (let i = 0; i < 400 && state.status === 'playing'; i += 1) {
    const direction = DIRECTIONS[i % 4];
    const before = sumOf(state.grid);
    const result = applyMove(state, direction);

    if (result.outcome === 'moved') {
      assert.equal(result.spawnedValue !== null, true);
      assert.equal(sumOf(result.state.grid), before + (result.spawnedValue ?? 0));
    } else {
      assert.equal(sumOf(result.state.grid), before);
    }
    state = result.state;
  }
});

test('the score only ever grows, by the gain the move reported', () => {
  let state = newGame(31337);
  let expected = 0;
  for (let i = 0; i < 400 && state.status === 'playing'; i += 1) {
    const result = applyMove(state, DIRECTIONS[i % 4]);
    expected += result.gained;
    assert.equal(result.state.score, expected);
    state = result.state;
  }
});

test('moveCount counts the moves that actually moved', () => {
  let state = newGame(8675309);
  let moved = 0;
  for (let i = 0; i < 200 && state.status === 'playing'; i += 1) {
    const result = applyMove(state, DIRECTIONS[i % 4]);
    if (result.outcome === 'moved') moved += 1;
    assert.equal(result.state.moveCount, moved);
    state = result.state;
  }
});

// ---------------------------------------------------------------------------
// Test 14: determinism.
// ---------------------------------------------------------------------------

test('the same seed and the same moves give the same board, ten times', () => {
  const moves: readonly Direction[] = [
    'left', 'up', 'right', 'down', 'left', 'left', 'up', 'right', 'down', 'up',
    'right', 'right', 'down', 'left', 'up',
  ];

  function play(): GameState {
    let state = newGame(0xdecafbad);
    for (const direction of moves) {
      state = applyMove(state, direction).state;
    }
    return state;
  }

  const first = play();
  for (let run = 0; run < 10; run += 1) {
    const again = play();
    assert.deepEqual(again.grid, first.grid, `run ${run} drifted`);
    assert.equal(again.score, first.score);
    assert.equal(again.seed, first.seed);
    assert.equal(again.moveCount, first.moveCount);
    assert.equal(again.status, first.status);
  }
});

test('different seeds do not all give the same board', () => {
  const boards = new Set<string>();
  for (let seed = 1; seed <= 50; seed += 1) {
    boards.add(newGame(seed).grid.join(','));
  }
  assert.equal(boards.size > 1, true);
});

// ---------------------------------------------------------------------------
// Test 15: the freeze is deep, or it is a lie.
// ---------------------------------------------------------------------------

test('writing into the grid of a state throws a TypeError', () => {
  const state = newGame(11);
  assert.throws(() => {
    (state.grid as number[])[0] = 99;
  }, TypeError);
});

test('writing a field of a state throws a TypeError', () => {
  const state = newGame(11);
  assert.throws(() => {
    (state as { score: number }).score = 99999;
  }, TypeError);
});

test('every state that leaves the domain is frozen, grid and all', () => {
  const fresh = newGame(2048);
  assert.equal(Object.isFrozen(fresh), true);
  assert.equal(Object.isFrozen(fresh.grid), true);

  const after = applyMove(fresh, 'left');
  assert.equal(Object.isFrozen(after.state), true);
  assert.equal(Object.isFrozen(after.state.grid), true);
});

test('a move never edits the state it was given', () => {
  const before = newGame(4096);
  const snapshot = [...before.grid];
  applyMove(before, 'left');
  assert.deepEqual(before.grid, snapshot);
});

// ---------------------------------------------------------------------------
// The status is read after the spawn, and won is sticky.
// ---------------------------------------------------------------------------

test('a spawn that fills the last cell can end the game', () => {
  // One empty cell, and a left move that merges the top row and frees nothing
  // else. The spawn refills the board.
  const grid: Grid = [
    2, 2, 8, 16,
    4, 2, 16, 8,
    2, 4, 8, 16,
    4, 2, 16, 8,
  ];
  const state = stateOf(grid, { seed: 5 });
  assert.equal(state.status, 'playing');

  const result = applyMove(state, 'left');
  assert.equal(result.outcome, 'moved');
  // The board is full again after the spawn, so the status must be honest.
  assert.equal(result.state.grid.includes(0), false);
  assert.equal(result.state.status, isGameOver(result.state.grid) ? 'over' : 'playing');
});

test('won is a sticky flag, and play continues after 2048', () => {
  const grid: Grid = [
    1024, 1024, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
    0, 0, 0, 0,
  ];
  const state = stateOf(grid, { seed: 77 });
  const first = applyMove(state, 'left');

  assert.equal(first.outcome, 'moved');
  assert.equal(first.state.won, true);
  assert.equal(first.state.status, 'playing');
  assert.equal(first.gained, 2048);

  // The 2048 tile merges away, and won stays true.
  const second = applyMove(first.state, 'right');
  assert.equal(second.state.won, true);
});

test('a moved result reports where the new tile landed', () => {
  const state = newGame(60606);
  const result = applyMove(state, 'left');
  if (result.outcome === 'moved') {
    assert.equal(typeof result.spawnedAt, 'number');
    assert.equal(state.grid[result.spawnedAt ?? 0], 0, 'the tile landed on an occupied cell');
    assert.equal(result.state.grid[result.spawnedAt ?? 0], result.spawnedValue);
  }
});

test('an empty board is not over, and yet no slide can change it', () => {
  const empty: Grid = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
  const state = freezeState({
    grid: empty,
    score: 0,
    seed: 1,
    moveCount: 0,
    status: 'playing',
    won: false,
  });

  assert.equal(isGameOver(empty), false);
  for (const direction of DIRECTIONS) {
    assert.equal(applyMove(state, direction).outcome, 'blocked');
  }
  // newGame never hands this board out, which is why the rule above is safe.
  assert.equal(newGame(1).grid.filter((cell) => cell !== 0).length, 2);
});
