import test from 'node:test';
import assert from 'node:assert/strict';

import { createGame2048 } from './composition-root.ts';
import type { Direction } from './domain/types.ts';
import type { RandomSource } from './ports/random-source.ts';

function fixedRandom(seed: number): RandomSource {
  return { nextSeed: () => seed };
}

const TEN_MOVES: readonly Direction[] = [
  'left', 'up', 'right', 'down', 'left', 'up', 'right', 'down', 'left', 'up',
];

// Test 21 of the plan: the real wiring starts and plays.
test('the wired game plays ten moves and keeps an honest score', () => {
  const api = createGame2048({ random: fixedRandom(0xc0ffee) });
  const start = api.startGame('g');
  assert.equal(start.score, 0);

  let expectedScore = 0;
  let moved = 0;
  let last = start;

  for (const direction of TEN_MOVES) {
    const result = api.playMove('g', direction);
    expectedScore += result.gained;
    if (result.outcome === 'moved') moved += 1;
    last = result.state;
  }

  assert.equal(moved > 0, true, 'ten moves on a fresh board moved nothing');
  assert.equal(last.score, expectedScore);
  assert.equal(last.moveCount, moved);
  assert.equal(api.getGame('g')?.score, expectedScore);
});

test('the wired game is repeatable when the seed is fixed', () => {
  function play(): readonly number[] {
    const api = createGame2048({ random: fixedRandom(20260911) });
    api.startGame('g');
    for (const direction of TEN_MOVES) api.playMove('g', direction);
    return api.getGame('g')?.grid ?? [];
  }
  assert.deepEqual(play(), play());
});

test('the default wiring starts a game with no overrides at all', () => {
  const api = createGame2048();
  const state = api.startGame('real');
  assert.equal(state.grid.length, 16);
  assert.equal(state.grid.filter((cell) => cell !== 0).length, 2);
  assert.equal(state.status, 'playing');
});

test('the default wiring uses real randomness, so two games differ', () => {
  const grids = new Set<string>();
  for (let i = 0; i < 20; i += 1) {
    const api = createGame2048();
    grids.add(api.startGame('g').grid.join(','));
  }
  assert.equal(grids.size > 1, true, 'twenty fresh games were all identical');
});

test('only the store override is replaced, and the rest still works', () => {
  const saved = new Map<string, unknown>();
  const api = createGame2048({
    store: {
      load: (gameId) => saved.get(gameId) as never,
      save: (gameId, state) => {
        saved.set(gameId, state);
      },
      remove: (gameId) => {
        saved.delete(gameId);
      },
    },
  });
  api.startGame('g');
  assert.equal(saved.has('g'), true);
});

test('each wired game keeps its own store', () => {
  const first = createGame2048({ random: fixedRandom(1) });
  const second = createGame2048({ random: fixedRandom(1) });
  first.startGame('g');
  assert.equal(second.getGame('g'), undefined);
});

test('the wired game refuses a duplicate id', () => {
  const api = createGame2048({ random: fixedRandom(5) });
  api.startGame('g');
  assert.throws(() => api.startGame('g'), /already exists/);
});
