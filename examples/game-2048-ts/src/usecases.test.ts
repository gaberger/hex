import test from 'node:test';
import assert from 'node:assert/strict';

import { isGameOver } from './domain/game.ts';
import { freezeState } from './domain/state.ts';
import type { GameState, Grid } from './domain/types.ts';
import type { BoardStore } from './ports/board-store.ts';
import type { RandomSource } from './ports/random-source.ts';
import { getGame } from './usecases/get-game.ts';
import { playMove } from './usecases/play-move.ts';
import { startGame } from './usecases/start-game.ts';

/** A fake store that counts its own writes. It lives here, in the test file. */
interface CountingStore extends BoardStore {
  readonly counts: { writes: number; reads: number; removes: number };
}

function createCountingStore(): CountingStore {
  const games = new Map<string, GameState>();
  const counts = { writes: 0, reads: 0, removes: 0 };
  return {
    counts,
    load(gameId: string): GameState | undefined {
      counts.reads += 1;
      return games.get(gameId);
    },
    save(gameId: string, state: GameState): void {
      counts.writes += 1;
      games.set(gameId, state);
    },
    remove(gameId: string): void {
      counts.removes += 1;
      games.delete(gameId);
    },
  };
}

function fixedRandom(seed: number): RandomSource {
  return { nextSeed: () => seed };
}

// Full, no horizontal pair, so a left move is blocked. Column 0 holds 2 above
// 2, so the game is still alive and the move is not rejected.
const LEFT_BLOCKED: Grid = [
  2, 4, 8, 16,
  2, 16, 4, 8,
  4, 2, 16, 2,
  8, 4, 2, 16,
];

test('startGame saves a fresh two-tile game under the id you gave', () => {
  const store = createCountingStore();
  const state = startGame(store, fixedRandom(4242), 'game-1');

  assert.equal(store.counts.writes, 1);
  assert.equal(store.load('game-1'), state);
  assert.equal(state.grid.filter((cell) => cell !== 0).length, 2);
});

test('startGame throws on a duplicate id, and does not overwrite', () => {
  const store = createCountingStore();
  const first = startGame(store, fixedRandom(4242), 'game-1');

  assert.throws(() => startGame(store, fixedRandom(9999), 'game-1'), /already exists/);
  assert.equal(store.load('game-1'), first);
  assert.equal(store.counts.writes, 1);
});

test('playMove throws for a game the store never saw', () => {
  const store = createCountingStore();
  assert.throws(() => playMove(store, 'ghost', 'left'), /no game/);
  assert.equal(store.counts.writes, 0);
});

// Test 3 of the plan.
test('a blocked move writes nothing, and changes nothing', () => {
  const store = createCountingStore();
  const before = freezeState({
    grid: [...LEFT_BLOCKED],
    score: 100,
    seed: 999,
    moveCount: 7,
    status: isGameOver(LEFT_BLOCKED) ? 'over' : 'playing',
    won: false,
  });
  assert.equal(before.status, 'playing');

  store.save('g', before);
  const writesAfterSetup = store.counts.writes;

  const result = playMove(store, 'g', 'left');

  assert.equal(result.outcome, 'blocked');
  assert.equal(store.counts.writes, writesAfterSetup, 'a blocked move wrote to the store');
  assert.deepEqual(result.state.grid, before.grid);
  assert.equal(result.state.score, before.score);
  assert.equal(result.state.seed, before.seed);
  assert.equal(result.state.moveCount, before.moveCount);
  assert.equal(store.load('g'), before);
});

test('a move that moved is saved once', () => {
  const store = createCountingStore();
  startGame(store, fixedRandom(555), 'g');
  const writesAfterStart = store.counts.writes;

  let moved = 0;
  for (const direction of ['left', 'up', 'right', 'down'] as const) {
    if (playMove(store, 'g', direction).outcome === 'moved') moved += 1;
  }

  assert.equal(moved > 0, true, 'no direction moved on a fresh board');
  assert.equal(store.counts.writes, writesAfterStart + moved);
});

test('getGame reads the store, and reports a missing game as undefined', () => {
  const store = createCountingStore();
  assert.equal(getGame(store, 'nope'), undefined);

  const state = startGame(store, fixedRandom(31), 'g');
  assert.equal(getGame(store, 'g'), state);
});

test('the seed comes from the random source, so a fixed seed repeats a game', () => {
  const a = startGame(createCountingStore(), fixedRandom(777), 'g');
  const b = startGame(createCountingStore(), fixedRandom(777), 'g');
  assert.deepEqual(a.grid, b.grid);
  assert.equal(a.seed, b.seed);
});
