import { describe, it, expect, beforeEach } from 'bun:test';
import {
  extractArchActions,
  extractValidationActions,
  buildActionItemReport,
  formatActionItems,
  resetActionCounter,
} from '../../src/core/domain/action-items.js';
import type { ArchAnalysisResult } from '../../src/core/domain/value-objects.js';
import type { ValidationVerdict } from '../../src/core/domain/validation-types.js';

beforeEach(() => resetActionCounter());

function makeArchResult(overrides: Partial<ArchAnalysisResult> = {}): ArchAnalysisResult {
  return {
    deadExports: [],
    orphanFiles: [],
    dependencyViolations: [],
    circularDeps: [],
    unusedPorts: [],
    unusedAdapters: [],
    summary: {
      totalFiles: 10, totalExports: 50,
      deadExportCount: 0, violationCount: 0, circularCount: 0, healthScore: 100,
    },
    ...overrides,
  };
}

function makeVerdict(overrides: Partial<ValidationVerdict> = {}): ValidationVerdict {
  return {
    passed: true,
    behavioralResults: [],
    propertyResults: [],
    smokeResults: [],
    signConventionAudit: { consistent: true, issues: [] },
    overallScore: 100,
    ...overrides,
  };
}

describe('extractArchActions', () => {
  it('returns empty for clean architecture', () => {
    expect(extractArchActions(makeArchResult())).toHaveLength(0);
  });

  it('creates critical action for cross-adapter violation', () => {
    const result = makeArchResult({
      dependencyViolations: [{
        from: 'src/adapters/primary/cli.ts',
        to: 'src/adapters/secondary/db.ts',
        fromLayer: 'adapters/primary',
        toLayer: 'adapters/secondary',
        rule: 'Adapters must not import other adapters',
      }],
    });
    const items = extractArchActions(result);
    expect(items).toHaveLength(1);
    expect(items[0].priority).toBe('critical');
    expect(items[0].category).toBe('violation');
    expect(items[0].suggestedFix).toContain('port interface');
  });

  it('creates critical action for circular dependencies', () => {
    const result = makeArchResult({
      circularDeps: [['a.ts', 'b.ts', 'c.ts']],
    });
    const items = extractArchActions(result);
    expect(items).toHaveLength(1);
    expect(items[0].priority).toBe('critical');
    expect(items[0].category).toBe('circular-dep');
  });

  it('groups dead exports by file', () => {
    const result = makeArchResult({
      deadExports: [
        { filePath: 'src/foo.ts', exportName: 'a', kind: 'function' },
        { filePath: 'src/foo.ts', exportName: 'b', kind: 'type' },
        { filePath: 'src/bar.ts', exportName: 'c', kind: 'const' },
      ],
    });
    const items = extractArchActions(result);
    expect(items).toHaveLength(2); // 2 files, not 3 exports
    expect(items[0].priority).toBe('low');
    expect(items[0].category).toBe('dead-code');
  });

  it('creates medium action for unused ports', () => {
    const result = makeArchResult({ unusedPorts: ['IUnusedPort'] });
    const items = extractArchActions(result);
    expect(items).toHaveLength(1);
    expect(items[0].priority).toBe('medium');
    expect(items[0].category).toBe('unused-port');
  });
});

describe('extractValidationActions', () => {
  it('returns empty for passing verdict', () => {
    expect(extractValidationActions(makeVerdict())).toHaveLength(0);
  });

  it('creates high-priority bug for behavioral spec failure', () => {
    const verdict = makeVerdict({
      behavioralResults: [{
        spec: { id: 'b1', description: 'Bird moves up on flap', category: 'physics', assertions: [] },
        passed: false,
        failures: ['Expected velocity < 0 but got 5'],
      }],
    });
    const items = extractValidationActions(verdict);
    expect(items.some((i) => i.category === 'bug' && i.priority === 'high')).toBe(true);
  });

  it('creates critical bug for smoke test failure', () => {
    const verdict = makeVerdict({
      smokeResults: [{
        scenario: { id: 's1', description: 'App starts', steps: [], expectedOutcome: 'No crash' },
        passed: false,
        failedAtStep: 1,
        error: 'Cannot read property of undefined',
      }],
    });
    const items = extractValidationActions(verdict);
    expect(items.some((i) => i.priority === 'critical')).toBe(true);
  });

  it('creates test-gap item when no property tests exist but behavioral specs do', () => {
    const verdict = makeVerdict({
      propertyResults: [],
      behavioralResults: [{
        spec: { id: 'b1', description: 'Something works', category: 'state', assertions: [] },
        passed: true,
        failures: [],
      }],
    });
    const items = extractValidationActions(verdict);
    expect(items.some((i) => i.category === 'test-gap')).toBe(true);
  });

  it('creates high-priority bug for sign convention issues', () => {
    const verdict = makeVerdict({
      signConventionAudit: {
        consistent: false,
        issues: ['gravity sign is positive but flapStrength is also positive'],
      },
    });
    const items = extractValidationActions(verdict);
    expect(items.some((i) => i.title.includes('Sign convention'))).toBe(true);
  });

  it('extracts fix hint from hardcoded year failure', () => {
    const verdict = makeVerdict({
      behavioralResults: [{
        spec: { id: 'b2', description: 'Leaderboard shows current season', category: 'state', assertions: [] },
        passed: false,
        failures: ['handleLeaderboard hardcodes Season(2025) instead of time.Now().Year()'],
      }],
    });
    const items = extractValidationActions(verdict);
    const bug = items.find((i) => i.category === 'bug');
    expect(bug?.suggestedFix).toContain('time.Now().Year()');
  });
});

describe('buildActionItemReport', () => {
  it('combines arch and validation results', () => {
    const arch = makeArchResult({
      dependencyViolations: [{
        from: 'a.ts', to: 'b.ts',
        fromLayer: 'adapters/primary', toLayer: 'adapters/secondary',
        rule: 'No cross-adapter',
      }],
    });
    const verdict = makeVerdict({
      smokeResults: [{
        scenario: { id: 's1', description: 'Startup', steps: [], expectedOutcome: 'OK' },
        passed: false, error: 'crash',
      }],
    });
    const report = buildActionItemReport(arch, verdict);
    expect(report.source).toBe('combined');
    expect(report.totalItems).toBeGreaterThanOrEqual(2);
    // Critical items should be first (sorted)
    expect(report.items[0].priority).toBe('critical');
  });

  it('works with arch-only', () => {
    const report = buildActionItemReport(makeArchResult());
    expect(report.source).toBe('arch-analysis');
  });

  it('works with verdict-only', () => {
    const report = buildActionItemReport(undefined, makeVerdict());
    expect(report.source).toBe('validation-verdict');
  });
});

describe('formatActionItems', () => {
  it('produces readable report text', () => {
    const report = buildActionItemReport(makeArchResult({
      dependencyViolations: [{
        from: 'src/adapters/primary/cli.ts',
        to: 'src/adapters/secondary/db.ts',
        fromLayer: 'adapters/primary',
        toLayer: 'adapters/secondary',
        rule: 'No cross-adapter imports',
      }],
      deadExports: [
        { filePath: 'src/old.ts', exportName: 'unused', kind: 'function' },
      ],
    }));
    const text = formatActionItems(report);
    expect(text).toContain('ACTION ITEMS');
    expect(text).toContain('MUST FIX');
    expect(text).toContain('SHOULD FIX');
    expect(text).toContain('[CRITICAL]');
    expect(text).toContain('[LOW]');
  });

  it('shows all-clear for empty report', () => {
    const report = buildActionItemReport(makeArchResult());
    const text = formatActionItems(report);
    expect(text).toContain('No action items');
  });
});
