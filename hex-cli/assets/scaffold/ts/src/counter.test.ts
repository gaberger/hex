/**
 * The gate. `npm test` must exit 0 on a freshly scaffolded project after one
 * `npm install`.
 *
 * Gate-first development (ADR-2026-09-11-1900): this file is the spec. If you
 * change what the project should do, change this first.
 */
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Count } from './core/domain/count.js';
import { increment } from './core/usecases/increment.js';
import { counter, incrementOnce } from './composition-root.js';

test('a new count starts at zero', () => {
  assert.equal(Count.zero().value(), 0);
});

test('incrementing twice gives two', () => {
  const store = counter();
  increment(store);
  assert.equal(increment(store).value(), 2);
});

test('the wired application increments', () => {
  assert.equal(incrementOnce().value(), 1);
});

test('a count saturates rather than wrapping', () => {
  // A counter that silently restarts at zero is worse than one that stops.
  assert.equal(counter().load().value(), 0);
});
