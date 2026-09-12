import test from 'node:test';
import assert from 'node:assert/strict';

import { createInMemoryBoardStore } from './adapters/secondary/in-memory-board-store.ts';
import { newGame } from './domain/game.ts';

test('a store returns undefined for a game it never saw', () => {
  const store = createInMemoryBoardStore();
  assert.equal(store.load('missing'), undefined);
});

test('a saved game comes back exactly as it went in', () => {
  const store = createInMemoryBoardStore();
  const state = newGame(1);
  store.save('g1', state);
  assert.equal(store.load('g1'), state);
});

// Test 19 of the plan.
test('remove makes load return undefined again', () => {
  const store = createInMemoryBoardStore();
  store.save('g1', newGame(1));
  assert.notEqual(store.load('g1'), undefined);

  store.remove('g1');
  assert.equal(store.load('g1'), undefined);
});

test('removing a game that is not there is harmless', () => {
  const store = createInMemoryBoardStore();
  assert.doesNotThrow(() => store.remove('never-existed'));
});

test('two games in one store do not touch each other', () => {
  const store = createInMemoryBoardStore();
  const a = newGame(1);
  const b = newGame(2);
  store.save('a', a);
  store.save('b', b);

  store.remove('a');
  assert.equal(store.load('a'), undefined);
  assert.equal(store.load('b'), b);
});

test('two stores do not share their games', () => {
  const first = createInMemoryBoardStore();
  const second = createInMemoryBoardStore();
  first.save('g', newGame(1));
  assert.equal(second.load('g'), undefined);
});
