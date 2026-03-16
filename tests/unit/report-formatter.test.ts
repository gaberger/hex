import { describe, it, expect } from 'bun:test';
import { formatArchReport, formatCompactSummary } from '../../src/core/domain/report-formatter.js';
import type { ArchAnalysisResult } from '../../src/core/domain/value-objects.js';

function makeResult(overrides: Partial<ArchAnalysisResult> = {}): ArchAnalysisResult {
  return {
    deadExports: [],
    orphanFiles: [],
    dependencyViolations: [],
    circularDeps: [],
    unusedPorts: [],
    unusedAdapters: [],
    summary: {
      totalFiles: 10,
      totalExports: 50,
      deadExportCount: 0,
      violationCount: 0,
      circularCount: 0,
      healthScore: 100,
    },
    ...overrides,
  };
}

describe('formatArchReport', () => {
  it('produces a clean report for a healthy project', () => {
    const report = formatArchReport(makeResult(), '.');
    expect(report).toContain('HEXAGONAL ARCHITECTURE HEALTH REPORT');
    expect(report).toContain('Score:    100/100');
    expect(report).toContain('Grade:    A (Excellent)');
    expect(report).toContain('All hexagonal architecture rules are satisfied');
    // Summary table
    expect(report).toContain('Boundary violations');
    expect(report).toContain('PASS');
  });

  it('shows FAIL status for boundary violations', () => {
    const result = makeResult({
      dependencyViolations: [
        {
          from: 'src/adapters/primary/cli.ts',
          to: 'src/adapters/secondary/db.ts',
          fromLayer: 'adapters/primary',
          toLayer: 'adapters/secondary',
          rule: 'Adapters must not import other adapters',
        },
      ],
      summary: {
        totalFiles: 10,
        totalExports: 50,
        deadExportCount: 0,
        violationCount: 1,
        circularCount: 0,
        healthScore: 90,
      },
    });
    const report = formatArchReport(result, '.');
    expect(report).toContain('BOUNDARY VIOLATIONS');
    expect(report).toContain('[CRITICAL]');
    expect(report).toContain('Adapters must not import other adapters');
    expect(report).toContain('Action required');
  });

  it('shows error rates section', () => {
    const result = makeResult({
      deadExports: [
        { filePath: 'src/core/domain/foo.ts', exportName: 'unused', kind: 'function' },
      ],
      summary: {
        totalFiles: 10,
        totalExports: 50,
        deadExportCount: 1,
        violationCount: 0,
        circularCount: 0,
        healthScore: 99,
      },
    });
    const report = formatArchReport(result, '.');
    expect(report).toContain('ERROR RATES');
    expect(report).toContain('Violation rate');
    expect(report).toContain('Dead export rate');
    expect(report).toContain('2.0%'); // 1/50 = 2%
  });

  it('includes rules reference by default', () => {
    const report = formatArchReport(makeResult(), '.');
    expect(report).toContain('HEXAGONAL RULES REFERENCE');
    expect(report).toContain('domain/ must only import from domain/');
  });

  it('omits rules reference when showRulesReference is false', () => {
    const report = formatArchReport(makeResult(), '.', { showRulesReference: false });
    expect(report).not.toContain('HEXAGONAL RULES REFERENCE');
  });

  it('shows circular dependencies', () => {
    const result = makeResult({
      circularDeps: [['src/a.ts', 'src/b.ts', 'src/c.ts']],
      summary: {
        totalFiles: 10,
        totalExports: 50,
        deadExportCount: 0,
        violationCount: 0,
        circularCount: 1,
        healthScore: 85,
      },
    });
    const report = formatArchReport(result, '.');
    expect(report).toContain('CIRCULAR DEPENDENCIES');
    expect(report).toContain('[cycle]');
  });

  it('shows unused ports and adapters', () => {
    const result = makeResult({
      unusedPorts: ['IUnusedPort'],
      unusedAdapters: ['src/adapters/secondary/unused.ts'],
    });
    const report = formatArchReport(result, '.');
    expect(report).toContain('UNUSED PORTS & ADAPTERS');
    expect(report).toContain('IUnusedPort');
  });

  it('groups dead exports by file', () => {
    const result = makeResult({
      deadExports: [
        { filePath: 'src/core/domain/foo.ts', exportName: 'bar', kind: 'function' },
        { filePath: 'src/core/domain/foo.ts', exportName: 'baz', kind: 'type' },
      ],
      summary: {
        totalFiles: 5,
        totalExports: 20,
        deadExportCount: 2,
        violationCount: 0,
        circularCount: 0,
        healthScore: 98,
      },
    });
    const report = formatArchReport(result, '.');
    expect(report).toContain('DEAD EXPORTS');
    expect(report).toContain('bar');
    expect(report).toContain('baz');
  });

  it('assigns correct grade for different scores', () => {
    expect(formatArchReport(makeResult({ summary: { totalFiles: 1, totalExports: 1, deadExportCount: 0, violationCount: 0, circularCount: 0, healthScore: 95 } }), '.')).toContain('Grade:    A');
    expect(formatArchReport(makeResult({ summary: { totalFiles: 1, totalExports: 1, deadExportCount: 0, violationCount: 0, circularCount: 0, healthScore: 80 } }), '.')).toContain('Grade:    B');
    expect(formatArchReport(makeResult({ summary: { totalFiles: 1, totalExports: 1, deadExportCount: 0, violationCount: 0, circularCount: 0, healthScore: 65 } }), '.')).toContain('Grade:    C');
    expect(formatArchReport(makeResult({ summary: { totalFiles: 1, totalExports: 1, deadExportCount: 0, violationCount: 0, circularCount: 0, healthScore: 45 } }), '.')).toContain('Grade:    D');
    expect(formatArchReport(makeResult({ summary: { totalFiles: 1, totalExports: 1, deadExportCount: 0, violationCount: 0, circularCount: 0, healthScore: 30 } }), '.')).toContain('Grade:    F');
  });
});

describe('formatCompactSummary', () => {
  it('produces a single-line summary', () => {
    const result = makeResult();
    const compact = formatCompactSummary(result);
    expect(compact).toContain('Score: 100/100 (A)');
    expect(compact).toContain('Violations: 0');
    expect(compact).not.toContain('\n');
  });
});
