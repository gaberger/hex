/**
 * Architecture Report Formatter
 *
 * Pure function that transforms an ArchAnalysisResult into a
 * human-readable report with tables, error rates, and severity grades.
 * No I/O — usable by both CLI and MCP adapters.
 */

import type { ArchAnalysisResult, DependencyViolation, DeadExport, DependencyDirection, RepoHygieneResult, HygieneSeverity } from './value-objects.js';

// ─── Report Configuration ──────────────────────────────

interface ReportOptions {
  /** Show full file paths (false = basename only) */
  fullPaths?: boolean;
  /** Maximum items to show per section before truncating */
  maxItems?: number;
  /** Include the rules reference table */
  showRulesReference?: boolean;
}

const DEFAULTS: Required<ReportOptions> = {
  fullPaths: false,
  maxItems: 20,
  showRulesReference: true,
};

// ─── Severity & Grading ────────────────────────────────

type Grade = 'A' | 'B' | 'C' | 'D' | 'F';

function scoreToGrade(score: number): Grade {
  if (score >= 90) return 'A';
  if (score >= 75) return 'B';
  if (score >= 60) return 'C';
  if (score >= 40) return 'D';
  return 'F';
}

function gradeLabel(grade: Grade): string {
  const labels: Record<Grade, string> = {
    A: 'Excellent',
    B: 'Good',
    C: 'Needs Attention',
    D: 'Poor',
    F: 'Critical',
  };
  return labels[grade];
}

type Severity = 'critical' | 'warning' | 'info';

function violationSeverity(v: DependencyViolation): Severity {
  // Cross-adapter coupling and domain violations are critical
  if (v.fromLayer.startsWith('adapters/') && v.toLayer.startsWith('adapters/')) return 'critical';
  if (v.fromLayer === 'domain' && v.toLayer !== 'domain') return 'critical';
  // Adapter importing usecases or infrastructure is a warning
  if (v.fromLayer.startsWith('adapters/') && v.toLayer === 'usecases') return 'warning';
  if (v.fromLayer.startsWith('adapters/') && v.toLayer === 'infrastructure') return 'warning';
  return 'warning';
}

// ─── Table Helpers ─────────────────────────────────────

function shortPath(fullPath: string): string {
  // Show last 2-3 path segments for readability
  const parts = fullPath.split('/');
  return parts.length <= 3 ? fullPath : parts.slice(-3).join('/');
}

function padRight(str: string, len: number): string {
  return str.length >= len ? str.slice(0, len) : str + ' '.repeat(len - str.length);
}

function padLeft(str: string, len: number): string {
  return str.length >= len ? str.slice(0, len) : ' '.repeat(len - str.length) + str;
}

function table(headers: string[], rows: string[][], colWidths?: number[]): string {
  if (rows.length === 0) return '';

  // Auto-calculate column widths if not provided
  const widths = colWidths ?? headers.map((h, i) => {
    const maxRow = Math.max(...rows.map((r) => (r[i] ?? '').length));
    return Math.max(h.length, maxRow);
  });

  const headerLine = headers.map((h, i) => padRight(h, widths[i])).join(' | ');
  const separator = widths.map((w) => '-'.repeat(w)).join('-+-');
  const dataLines = rows.map((row) =>
    row.map((cell, i) => padRight(cell, widths[i])).join(' | '),
  );

  return [headerLine, separator, ...dataLines].join('\n');
}

// ─── Layer Statistics ──────────────────────────────────

interface LayerStats {
  layer: DependencyDirection;
  files: number;
  exports: number;
  deadExports: number;
  violations: number;
  deadRate: string;      // percentage
  violationRate: string; // percentage
}

function computeLayerStats(
  result: ArchAnalysisResult,
  allFiles: Map<string, DependencyDirection>,
): LayerStats[] {
  const layers: DependencyDirection[] = [
    'domain', 'ports', 'usecases',
    'adapters/primary', 'adapters/secondary', 'infrastructure',
  ];

  return layers.map((layer) => {
    const files = [...allFiles.entries()].filter(([, l]) => l === layer).length;
    const dead = result.deadExports.filter((d) => classifyFromPath(d.filePath) === layer).length;
    const viols = result.dependencyViolations.filter((v) => v.fromLayer === layer).length;

    // Count exports in this layer
    const exports = result.deadExports.filter((d) => classifyFromPath(d.filePath) === layer).length
      + files; // approximate — dead exports + at least 1 live export per file

    const deadRate = files > 0 ? ((dead / Math.max(exports, 1)) * 100).toFixed(1) : '0.0';
    const violationRate = files > 0 ? ((viols / files) * 100).toFixed(1) : '0.0';

    return { layer, files, exports, deadExports: dead, violations: viols, deadRate, violationRate };
  });
}

/** Lightweight layer classification from file path (mirrors layer-classifier but avoids import) */
function classifyFromPath(filePath: string): DependencyDirection | 'unknown' {
  if (filePath.includes('/domain/') || filePath.includes('/internal/domain/')) return 'domain';
  if (filePath.includes('/ports/') || filePath.includes('/internal/ports/')) return 'ports';
  if (filePath.includes('/usecases/') || filePath.includes('/internal/usecases/')) return 'usecases';
  if (filePath.includes('/adapters/primary/') || filePath.includes('/cmd/')) return 'adapters/primary';
  if (filePath.includes('/adapters/secondary/') || filePath.includes('/pkg/')) return 'adapters/secondary';
  if (filePath.includes('/infrastructure/')) return 'infrastructure';
  return 'unknown';
}

// ─── Main Report Formatter ─────────────────────────────

export function formatArchReport(
  result: ArchAnalysisResult,
  projectPath: string,
  opts: ReportOptions = {},
): string {
  const o = { ...DEFAULTS, ...opts };
  const s = result.summary;
  const grade = scoreToGrade(s.healthScore);
  const pathFn = o.fullPaths ? (p: string) => p : shortPath;
  const lines: string[] = [];
  const ln = (text = '') => lines.push(text);

  // ── Header ──────────────────────────────────────────
  ln('='.repeat(60));
  ln('  HEXAGONAL ARCHITECTURE HEALTH REPORT');
  ln('='.repeat(60));
  ln();
  ln(`  Project:  ${projectPath}`);
  ln(`  Date:     ${new Date().toISOString().split('T')[0]}`);
  ln(`  Grade:    ${grade} (${gradeLabel(grade)})`);
  ln(`  Score:    ${s.healthScore}/100`);
  ln();

  // ── Summary Dashboard ───────────────────────────────
  ln('-'.repeat(60));
  ln('  SUMMARY');
  ln('-'.repeat(60));
  ln();

  ln(table(
    ['Metric', 'Count', 'Status'],
    [
      ['Files scanned', String(s.totalFiles), s.totalFiles > 0 ? 'OK' : 'WARN'],
      ['Total exports', String(s.totalExports), 'INFO'],
      ['Boundary violations', String(s.violationCount), s.violationCount === 0 ? 'PASS' : 'FAIL'],
      ['Circular dependencies', String(s.circularCount), s.circularCount === 0 ? 'PASS' : 'FAIL'],
      ['Dead exports', String(s.deadExportCount), s.deadExportCount === 0 ? 'PASS' : 'WARN'],
      ['Unused ports', String(result.unusedPorts.length), result.unusedPorts.length === 0 ? 'PASS' : 'WARN'],
      ['Unused adapters', String(result.unusedAdapters.length), result.unusedAdapters.length === 0 ? 'PASS' : 'INFO'],
      ['Orphan files', String(result.orphanFiles.length), result.orphanFiles.length === 0 ? 'PASS' : 'INFO'],
      ['Repo hygiene', result.repoHygiene ? String(result.repoHygiene.findings.length) : 'N/A',
        !result.repoHygiene ? 'SKIP' : result.repoHygiene.clean ? 'PASS' : 'WARN'],
    ],
    [22, 7, 6],
  ));
  ln();

  // ── Error Rates ─────────────────────────────────────
  const violationRate = s.totalFiles > 0
    ? ((s.violationCount / s.totalFiles) * 100).toFixed(1)
    : '0.0';
  const deadExportRate = s.totalExports > 0
    ? ((s.deadExportCount / s.totalExports) * 100).toFixed(1)
    : '0.0';

  ln('-'.repeat(60));
  ln('  ERROR RATES');
  ln('-'.repeat(60));
  ln();
  ln(table(
    ['Rate', 'Value', 'Threshold', 'Status'],
    [
      ['Violation rate', `${violationRate}%`, '0.0%', s.violationCount === 0 ? 'PASS' : 'FAIL'],
      ['Dead export rate', `${deadExportRate}%`, '<10.0%', parseFloat(deadExportRate) < 10 ? 'PASS' : 'WARN'],
      ['Circular dep rate', s.circularCount > 0 ? `${s.circularCount} cycles` : '0 cycles', '0 cycles', s.circularCount === 0 ? 'PASS' : 'FAIL'],
    ],
    [18, 12, 12, 6],
  ));
  ln();

  // ── Layer Breakdown ─────────────────────────────────
  const allFiles = new Map<string, DependencyDirection>();
  // Approximate file classification from violations and dead exports
  for (const v of result.dependencyViolations) {
    allFiles.set(v.from, v.fromLayer);
    allFiles.set(v.to, v.toLayer);
  }
  for (const d of result.deadExports) {
    const layer = classifyFromPath(d.filePath);
    if (layer !== 'unknown') allFiles.set(d.filePath, layer);
  }

  if (allFiles.size > 0) {
    const layerStats = computeLayerStats(result, allFiles);
    const nonEmpty = layerStats.filter((l) => l.files > 0 || l.violations > 0 || l.deadExports > 0);

    if (nonEmpty.length > 0) {
      ln('-'.repeat(60));
      ln('  LAYER BREAKDOWN');
      ln('-'.repeat(60));
      ln();
      ln(table(
        ['Layer', 'Violations', 'Dead Exports', 'Status'],
        nonEmpty.map((l) => [
          l.layer,
          String(l.violations),
          String(l.deadExports),
          l.violations > 0 ? 'FAIL' : l.deadExports > 0 ? 'WARN' : 'PASS',
        ]),
        [20, 11, 13, 6],
      ));
      ln();
    }
  }

  // ── Boundary Violations (Detail) ────────────────────
  if (result.dependencyViolations.length > 0) {
    ln('-'.repeat(60));
    ln('  BOUNDARY VIOLATIONS');
    ln('-'.repeat(60));
    ln();

    // Group by severity
    const critical = result.dependencyViolations.filter((v) => violationSeverity(v) === 'critical');
    const warnings = result.dependencyViolations.filter((v) => violationSeverity(v) === 'warning');

    if (critical.length > 0) {
      ln(`  [CRITICAL] ${critical.length} violation(s)`);
      ln();
      const critRows = critical.slice(0, o.maxItems).map((v) => [
        pathFn(v.from),
        pathFn(v.to),
        `${v.fromLayer} -> ${v.toLayer}`,
        v.rule,
      ]);
      ln(table(['From', 'To', 'Direction', 'Rule'], critRows));
      if (critical.length > o.maxItems) {
        ln(`  ... and ${critical.length - o.maxItems} more critical violations`);
      }
      ln();
    }

    if (warnings.length > 0) {
      ln(`  [WARNING] ${warnings.length} violation(s)`);
      ln();
      const warnRows = warnings.slice(0, o.maxItems).map((v) => [
        pathFn(v.from),
        pathFn(v.to),
        `${v.fromLayer} -> ${v.toLayer}`,
        v.rule,
      ]);
      ln(table(['From', 'To', 'Direction', 'Rule'], warnRows));
      if (warnings.length > o.maxItems) {
        ln(`  ... and ${warnings.length - o.maxItems} more warnings`);
      }
      ln();
    }
  }

  // ── Circular Dependencies ───────────────────────────
  if (result.circularDeps.length > 0) {
    ln('-'.repeat(60));
    ln('  CIRCULAR DEPENDENCIES');
    ln('-'.repeat(60));
    ln();
    for (const cycle of result.circularDeps.slice(0, o.maxItems)) {
      const short = cycle.map(pathFn);
      ln(`  ${short.join(' -> ')} -> [cycle]`);
    }
    if (result.circularDeps.length > o.maxItems) {
      ln(`  ... and ${result.circularDeps.length - o.maxItems} more cycles`);
    }
    ln();
  }

  // ── Dead Exports ────────────────────────────────────
  if (result.deadExports.length > 0) {
    ln('-'.repeat(60));
    ln('  DEAD EXPORTS');
    ln('-'.repeat(60));
    ln();

    // Group by file
    const byFile = new Map<string, DeadExport[]>();
    for (const d of result.deadExports) {
      const key = pathFn(d.filePath);
      if (!byFile.has(key)) byFile.set(key, []);
      byFile.get(key)!.push(d);
    }

    const deadRows: string[][] = [];
    let shown = 0;
    for (const [file, exports] of byFile) {
      if (shown >= o.maxItems) break;
      for (const exp of exports) {
        if (shown >= o.maxItems) break;
        deadRows.push([file, exp.exportName, exp.kind]);
        shown++;
      }
    }

    ln(table(['File', 'Export', 'Kind'], deadRows));
    if (result.deadExports.length > o.maxItems) {
      ln(`  ... and ${result.deadExports.length - o.maxItems} more dead exports`);
    }
    ln();
  }

  // ── Unused Ports & Adapters ─────────────────────────
  if (result.unusedPorts.length > 0 || result.unusedAdapters.length > 0) {
    ln('-'.repeat(60));
    ln('  UNUSED PORTS & ADAPTERS');
    ln('-'.repeat(60));
    ln();

    if (result.unusedPorts.length > 0) {
      ln('  Unused port interfaces (no adapter implements them):');
      for (const p of result.unusedPorts) {
        ln(`    - ${p}`);
      }
      ln();
    }

    if (result.unusedAdapters.length > 0) {
      ln('  Adapter files not implementing any port:');
      for (const a of result.unusedAdapters) {
        ln(`    - ${pathFn(a)}`);
      }
      ln();
    }
  }

  // ── Orphan Files ────────────────────────────────────
  if (result.orphanFiles.length > 0) {
    ln('-'.repeat(60));
    ln('  ORPHAN FILES');
    ln('-'.repeat(60));
    ln();
    ln('  Files with no incoming or outgoing import edges:');
    for (const f of result.orphanFiles.slice(0, o.maxItems)) {
      ln(`    - ${pathFn(f)}`);
    }
    if (result.orphanFiles.length > o.maxItems) {
      ln(`    ... and ${result.orphanFiles.length - o.maxItems} more`);
    }
    ln();
  }

  // ── Repo Hygiene (Anti-Slop) ────────────────────────
  if (result.repoHygiene && result.repoHygiene.findings.length > 0) {
    ln('-'.repeat(60));
    ln('  REPO HYGIENE');
    ln('-'.repeat(60));
    ln();
    formatHygieneFindings(result.repoHygiene, ln, o.maxItems);
  }

  // ── Rules Reference ─────────────────────────────────
  if (o.showRulesReference) {
    ln('-'.repeat(60));
    ln('  HEXAGONAL RULES REFERENCE');
    ln('-'.repeat(60));
    ln();
    ln(table(
      ['#', 'Rule', 'Severity'],
      [
        ['1', 'domain/ must only import from domain/', 'CRITICAL'],
        ['2', 'ports/ may import from domain/ only', 'CRITICAL'],
        ['3', 'usecases/ may import domain/ + ports/ only', 'WARNING'],
        ['4', 'adapters/primary/ may import ports/ only', 'WARNING'],
        ['5', 'adapters/secondary/ may import ports/ only', 'WARNING'],
        ['6', 'adapters must NEVER import other adapters', 'CRITICAL'],
        ['7', 'composition-root is the ONLY adapter importer', 'WARNING'],
      ],
      [2, 48, 8],
    ));
    ln();
  }

  // ── Footer ──────────────────────────────────────────
  ln('='.repeat(60));
  ln(`  Score: ${s.healthScore}/100 | Grade: ${grade} | ${gradeLabel(grade)}`);
  if (s.violationCount > 0 || s.circularCount > 0) {
    ln('  Action required: Fix boundary violations and circular deps');
  } else if (s.deadExportCount > 0) {
    ln('  Suggestion: Remove dead exports to improve score');
  } else {
    ln('  All hexagonal architecture rules are satisfied');
  }
  ln('='.repeat(60));

  return lines.join('\n');
}

// ─── Hygiene Section Formatter ─────────────────────────

function formatHygieneFindings(
  hygiene: RepoHygieneResult,
  ln: (text?: string) => void,
  maxItems: number,
): void {
  const severityOrder: Record<HygieneSeverity, number> = { critical: 0, warning: 1, info: 2 };
  const sorted = [...hygiene.findings].sort((a, b) => severityOrder[a.severity] - severityOrder[b.severity]);

  const bySeverity = new Map<HygieneSeverity, typeof sorted>();
  for (const f of sorted) {
    if (!bySeverity.has(f.severity)) bySeverity.set(f.severity, []);
    bySeverity.get(f.severity)!.push(f);
  }

  for (const [severity, findings] of bySeverity) {
    const label = severity.toUpperCase();
    ln(`  [${label}] ${findings.length} finding(s)`);
    ln();
    const rows = findings.slice(0, maxItems).map((f) => [
      f.category,
      shortPath(f.path),
      f.suggestedFix,
    ]);
    ln(table(['Category', 'Path', 'Suggested Fix'], rows));
    if (findings.length > maxItems) {
      ln(`  ... and ${findings.length - maxItems} more ${severity} findings`);
    }
    ln();
  }

  // Summary line
  ln(`  Totals: ${hygiene.uncommittedCount} uncommitted, ${hygiene.stagedCount} staged, ` +
    `${hygiene.orphanWorktreeCount} orphan worktrees, ${hygiene.embeddedRepoCount} embedded repos`);
  ln();
}

/**
 * Compact single-line summary for status bars and quick checks.
 */
export function formatCompactSummary(result: ArchAnalysisResult): string {
  const s = result.summary;
  const grade = scoreToGrade(s.healthScore);
  const parts = [
    `Score: ${s.healthScore}/100 (${grade})`,
    `Files: ${s.totalFiles}`,
    `Violations: ${s.violationCount}`,
    `Circular: ${s.circularCount}`,
    `Dead: ${s.deadExportCount}`,
  ];
  return parts.join(' | ');
}
