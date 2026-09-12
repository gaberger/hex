import test from 'node:test';
import assert from 'node:assert/strict';

import { nextSeed, randomBelow } from './domain/rng.ts';

// Test 17: a bound under 1 must throw. A silent 0 overwrites a real tile.
test('randomBelow throws when the bound is zero', () => {
  assert.throws(() => randomBelow(1234, 0), RangeError);
});

test('randomBelow throws when the bound is negative', () => {
  assert.throws(() => randomBelow(1234, -1), RangeError);
});

test('randomBelow throws when the bound is not a whole number', () => {
  assert.throws(() => randomBelow(1234, 2.5), RangeError);
});

test('randomBelow stays inside the bound', () => {
  let seed = 7;
  for (let i = 0; i < 2000; i += 1) {
    const draw = randomBelow(seed, 16);
    assert.equal(draw.value >= 0, true);
    assert.equal(draw.value < 16, true);
    seed = draw.seed;
  }
});

test('the mixer is pure: the same seed gives the same next seed', () => {
  assert.equal(nextSeed(42), nextSeed(42));
  assert.equal(nextSeed(0), nextSeed(0));
});

test('the seed stays an unsigned 32-bit whole number', () => {
  let seed = 0;
  for (let i = 0; i < 1000; i += 1) {
    seed = nextSeed(seed);
    assert.equal(Number.isInteger(seed), true);
    assert.equal(seed >= 0, true);
    assert.equal(seed <= 0xffffffff, true);
    assert.equal(seed >>> 0, seed);
  }
});

test('the mixer does not stick on one value', () => {
  const seen = new Set<number>();
  let seed = 1;
  for (let i = 0; i < 500; i += 1) {
    seed = nextSeed(seed);
    seen.add(seed);
  }
  assert.equal(seen.size > 400, true, `the mixer repeated too often: ${seen.size} of 500`);
});

test('a draw carries a seed forward, so the next draw differs', () => {
  const first = randomBelow(99, 10);
  const second = randomBelow(first.seed, 10);
  assert.notEqual(first.seed, second.seed);
});
