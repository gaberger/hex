import test from 'node:test';
import assert from 'node:assert/strict';

import { slideRowLeft } from './domain/row.ts';
import type { Row } from './domain/types.ts';

// Test 1 of the plan: a row slides left.
test('a row slides its tiles to the left', () => {
  const row: Row = [0, 2, 0, 4];
  assert.deepEqual(slideRowLeft(row).row, [2, 4, 0, 0]);
});

test('a row that is already packed left does not change', () => {
  const row: Row = [2, 4, 8, 16];
  const result = slideRowLeft(row);
  assert.deepEqual(result.row, [2, 4, 8, 16]);
  assert.equal(result.gained, 0);
});

test('an empty row stays empty', () => {
  const row: Row = [0, 0, 0, 0];
  const result = slideRowLeft(row);
  assert.deepEqual(result.row, [0, 0, 0, 0]);
  assert.equal(result.gained, 0);
});

// Test 2: two equal tiles merge once, and the result cannot merge again.
test('four equal tiles make two pairs, never one big tile', () => {
  const row: Row = [2, 2, 2, 2];
  const result = slideRowLeft(row);
  assert.deepEqual(result.row, [4, 4, 0, 0]);
  assert.notDeepEqual(result.row, [8, 0, 0, 0]);
  assert.equal(result.gained, 8);
});

test('a merged tile does not merge again with the tile behind it', () => {
  const row: Row = [4, 2, 2, 0];
  const result = slideRowLeft(row);
  assert.deepEqual(result.row, [4, 4, 0, 0]);
  assert.notDeepEqual(result.row, [8, 0, 0, 0]);
  assert.equal(result.gained, 4);
});

// Test 5: the single best merge test. [2,2,2,2] is symmetric and cannot tell
// a leftmost-first rule from a rightmost-first one. This row can.
test('three equal tiles merge the LEFT pair, not the right one', () => {
  const row: Row = [0, 2, 2, 2];
  const result = slideRowLeft(row);
  assert.deepEqual(result.row, [4, 2, 0, 0]);
  assert.notDeepEqual(result.row, [2, 4, 0, 0]);
  assert.equal(result.gained, 4);
});

// Test 7.
test('a leading tile blocks nothing behind it', () => {
  const row: Row = [4, 2, 2, 0];
  assert.deepEqual(slideRowLeft(row).row, [4, 4, 0, 0]);
});

// Test 8: two separate pairs merge separately.
test('two different pairs merge into two tiles', () => {
  const row: Row = [2, 2, 4, 4];
  const result = slideRowLeft(row);
  assert.deepEqual(result.row, [4, 8, 0, 0]);
  assert.notDeepEqual(result.row, [16, 0, 0, 0]);
  assert.equal(result.gained, 12);
});

// Test 22: the classic scoring bug. A merge scores the RESULT, not one tile.
test('a merge scores the doubled tile, not a single tile', () => {
  const row: Row = [2, 2, 0, 0];
  const result = slideRowLeft(row);
  assert.equal(result.gained, 4);
  assert.notEqual(result.gained, 2);
});

test('the returned row always holds four cells', () => {
  const rows: Row[] = [
    [0, 0, 0, 0],
    [2, 0, 0, 0],
    [2, 2, 2, 2],
    [2, 4, 2, 4],
  ];
  for (const row of rows) {
    assert.equal(slideRowLeft(row).row.length, 4);
  }
});

test('sliding never invents or loses a tile value', () => {
  const row: Row = [2, 0, 4, 0];
  const result = slideRowLeft(row);
  const before = row.reduce((sum, cell) => sum + cell, 0);
  const after = result.row.reduce((sum, cell) => sum + cell, 0);
  assert.equal(after, before);
});
