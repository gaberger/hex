import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/**
 * Test 20 of the plan.
 *
 * The whole correctness story rests on one claim: no function in this program
 * can stop half way through. The two keywords that would break that claim are
 * built here from pieces, on purpose. If they were written out, this file
 * would find itself and the check would always fail.
 */
const FORBIDDEN: readonly string[] = ['as' + 'ync', 'aw' + 'ait'];
const PATTERN = new RegExp(`\\b(${FORBIDDEN.join('|')})\\b`);

const SRC_DIR = fileURLToPath(new URL('../src', import.meta.url));

function typescriptFilesUnder(dir: string): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) found.push(...typescriptFilesUnder(full));
    else if (entry.name.endsWith('.ts')) found.push(full);
  }
  return found;
}

test('the source tree is found, and it is not empty', () => {
  const files = typescriptFilesUnder(SRC_DIR);
  assert.equal(files.length > 10, true, `only ${files.length} source files were found`);
});

test('no function anywhere under src can stop half way through', () => {
  const offenders: string[] = [];

  for (const file of typescriptFilesUnder(SRC_DIR)) {
    const lines = readFileSync(file, 'utf8').split('\n');
    lines.forEach((line, index) => {
      if (PATTERN.test(line)) {
        offenders.push(`${file}:${index + 1}: ${line.trim()}`);
      }
    });
  }

  assert.deepEqual(offenders, [], `suspended execution found:\n${offenders.join('\n')}`);
});

test('no function anywhere under src returns a Promise', () => {
  const offenders: string[] = [];
  const promisePattern = /\bPromise\s*</;

  for (const file of typescriptFilesUnder(SRC_DIR)) {
    const lines = readFileSync(file, 'utf8').split('\n');
    lines.forEach((line, index) => {
      if (promisePattern.test(line)) {
        offenders.push(`${file}:${index + 1}: ${line.trim()}`);
      }
    });
  }

  assert.deepEqual(offenders, [], `a promise was found:\n${offenders.join('\n')}`);
});

test('every test file sits flat in src, so the runner glob finds it', () => {
  const nested = typescriptFilesUnder(SRC_DIR)
    .filter((file) => file.endsWith('.test.ts'))
    .filter((file) => join(SRC_DIR, file.slice(SRC_DIR.length + 1)) !== file || file.slice(SRC_DIR.length + 1).includes('/'));

  assert.deepEqual(nested, [], `these test files would never run:\n${nested.join('\n')}`);
});
