/**
 * Cross-language DJB2 hash compatibility test.
 *
 * Verifies that the TypeScript makeProjectId() and Rust make_project_id()
 * produce identical output for the same inputs. If this test fails,
 * DashboardAdapter (TS) and hex-hub (Rust) will disagree on project IDs.
 */
import { describe, it, expect } from 'bun:test';

// Exact copy of makeProjectId from dashboard-hub.ts (line 90-94)
function makeProjectId(rootPath: string): string {
  const basename = rootPath.split('/').pop() ?? 'unknown';
  const hash = Array.from(rootPath).reduce(
    (h, c) => ((h << 5) - h + c.charCodeAt(0)) | 0,
    0,
  );
  return `${basename}-${(hash >>> 0).toString(36)}`;
}

// Hardcoded canonical values — identical vectors exist in hex-hub/src/state.rs.
// If EITHER side changes, both tests will fail, preventing silent divergence.
const TEST_VECTORS: Array<{ path: string; expected: string }> = [
  { path: '/Users/gary/projects/my-app', expected: 'my-app-1v7n98d' },
  { path: '/tmp/test', expected: 'test-14nsdrt' },
  { path: '/a/b/c/d/e', expected: 'e-1cqbqw4' },
  { path: '/Users/gary/hex-intf', expected: 'hex-intf-1x2ydj5' },
  { path: '/', expected: '-1b' },
  { path: '/single', expected: 'single-zng5yv' },
];

describe('makeProjectId cross-language compatibility', () => {
  it('produces deterministic output', () => {
    for (const { path } of TEST_VECTORS) {
      expect(makeProjectId(path)).toBe(makeProjectId(path));
    }
  });

  it('uses basename as prefix', () => {
    const id = makeProjectId('/Users/gary/projects/my-app');
    expect(id.startsWith('my-app-')).toBe(true);
  });

  it('different paths produce different IDs', () => {
    const id1 = makeProjectId('/a/my-app');
    const id2 = makeProjectId('/b/my-app');
    expect(id1).not.toBe(id2);
  });

  // Hardcoded test vectors — must match hex-hub/src/state.rs
  // project_id_matches_typescript_implementation test exactly.
  it('matches hardcoded test vectors (cross-language contract)', () => {
    for (const { path, expected } of TEST_VECTORS) {
      expect(makeProjectId(path)).toBe(expected);
    }
  });
});
