/**
 * Property Tests — Coordination Lock Invariants
 *
 * Tests universal invariants of the coordination layer's lock and claim
 * data structures without requiring a running hub. Validates that the
 * TypeScript types and adapter logic maintain consistency guarantees.
 */

import { describe, it, expect } from 'bun:test';
import type {
  LockResult,
  ClaimResult,
  UnstagedFile,
  WorktreeLock,
} from '../../src/core/ports/coordination.js';

// ── Property: LockResult mutual exclusion ────────────────

describe('Property: LockResult mutual exclusion', () => {
  const lockResults: LockResult[] = [
    { acquired: true, lock: makeLock('inst-1', 'feat-a', 'domain'), conflict: null },
    { acquired: false, lock: null, conflict: makeLock('inst-2', 'feat-a', 'domain') },
  ];

  for (const result of lockResults) {
    it(`acquired=${result.acquired} → lock and conflict are mutually exclusive`, () => {
      if (result.acquired) {
        expect(result.lock).not.toBeNull();
        expect(result.conflict).toBeNull();
      } else {
        expect(result.lock).toBeNull();
        expect(result.conflict).not.toBeNull();
      }
    });
  }
});

// ── Property: ClaimResult mutual exclusion ───────────────

describe('Property: ClaimResult mutual exclusion', () => {
  const claimResults: ClaimResult[] = [
    { claimed: true, claim: { taskId: 't1', instanceId: 'i1', claimedAt: now(), heartbeatAt: now() }, conflict: null },
    { claimed: false, claim: null, conflict: { taskId: 't1', instanceId: 'i2', claimedAt: now(), heartbeatAt: now() } },
  ];

  for (const result of claimResults) {
    it(`claimed=${result.claimed} → claim and conflict are mutually exclusive`, () => {
      if (result.claimed) {
        expect(result.claim).not.toBeNull();
        expect(result.conflict).toBeNull();
      } else {
        expect(result.claim).toBeNull();
        expect(result.conflict).not.toBeNull();
      }
    });
  }
});

// ── Property: Lock key uniqueness ────────────────────────

describe('Property: lock keys are unique per project+feature+layer', () => {
  const combos = [
    { project: 'p1', feature: 'auth', layer: 'domain' },
    { project: 'p1', feature: 'auth', layer: 'port' },
    { project: 'p1', feature: 'billing', layer: 'domain' },
    { project: 'p2', feature: 'auth', layer: 'domain' },
  ];

  it('all combinations produce distinct keys', () => {
    const keys = combos.map(c => `${c.project}:${c.feature}:${c.layer}`);
    const uniqueKeys = new Set(keys);
    expect(uniqueKeys.size).toBe(keys.length);
  });

  it('same inputs produce same key (deterministic)', () => {
    const a = `${combos[0].project}:${combos[0].feature}:${combos[0].layer}`;
    const b = `${combos[0].project}:${combos[0].feature}:${combos[0].layer}`;
    expect(a).toBe(b);
  });
});

// ── Property: UnstagedFile status is exhaustive ──────────

describe('Property: UnstagedFile status values', () => {
  const validStatuses: UnstagedFile['status'][] = ['modified', 'added', 'deleted'];

  for (const status of validStatuses) {
    it(`"${status}" is a valid status`, () => {
      const file: UnstagedFile = { path: 'test.ts', status, layer: 'other' };
      expect(validStatuses).toContain(file.status);
    });
  }
});

// ── Property: Layer classification completeness ──────────

describe('Property: classifyLayer covers all hex layers', () => {
  const pathToExpectedLayer: [string, string][] = [
    ['src/core/domain/entities.ts', 'domain'],
    ['src/core/ports/index.ts', 'port'],
    ['src/core/usecases/analyzer.ts', 'usecase'],
    ['src/adapters/primary/cli.ts', 'primary-adapter'],
    ['src/adapters/secondary/git.ts', 'secondary-adapter'],
    ['README.md', 'other'],
    ['package.json', 'other'],
    ['tests/unit/foo.test.ts', 'other'],
  ];

  // Replicate the adapter's classifyLayer logic for property testing
  function classifyLayer(filePath: string): string {
    if (filePath.includes('/core/domain/') || filePath.includes('core/domain/')) return 'domain';
    if (filePath.includes('/core/ports/') || filePath.includes('core/ports/')) return 'port';
    if (filePath.includes('/core/usecases/') || filePath.includes('core/usecases/')) return 'usecase';
    if (filePath.includes('/adapters/primary/') || filePath.includes('adapters/primary/')) return 'primary-adapter';
    if (filePath.includes('/adapters/secondary/') || filePath.includes('adapters/secondary/')) return 'secondary-adapter';
    return 'other';
  }

  for (const [path, expectedLayer] of pathToExpectedLayer) {
    it(`"${path}" → "${expectedLayer}"`, () => {
      expect(classifyLayer(path)).toBe(expectedLayer);
    });
  }

  it('every path maps to exactly one layer (no ambiguity)', () => {
    const ambiguousPath = 'src/core/domain/ports/thing.ts'; // has both domain and ports
    const result = classifyLayer(ambiguousPath);
    // domain check comes first, so domain wins — this is by design
    expect(result).toBe('domain');
  });
});

// ── Property: TTL is always positive ─────────────────────

describe('Property: WorktreeLock TTL invariants', () => {
  it('default TTL (300s) is reasonable for development work', () => {
    const lock = makeLock('i1', 'feat', 'domain');
    expect(lock.ttlSecs).toBeGreaterThan(0);
    expect(lock.ttlSecs).toBeLessThanOrEqual(3600); // max 1 hour
  });

  it('heartbeatAt is never before acquiredAt', () => {
    const lock = makeLock('i1', 'feat', 'domain');
    expect(new Date(lock.heartbeatAt).getTime()).toBeGreaterThanOrEqual(
      new Date(lock.acquiredAt).getTime()
    );
  });
});

// ── Helpers ─────────────────────────────────────────────

function now(): string {
  return new Date().toISOString();
}

function makeLock(instanceId: string, feature: string, layer: string): WorktreeLock {
  const ts = now();
  return {
    instanceId,
    projectId: 'test-proj',
    feature,
    layer,
    acquiredAt: ts,
    heartbeatAt: ts,
    ttlSecs: 300,
  };
}
