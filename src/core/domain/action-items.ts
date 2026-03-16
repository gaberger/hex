/**
 * Action Item Extractor
 *
 * Pure functions that transform validation results into structured,
 * actionable tasks. These feed into the dev-tracker's task queue so
 * that findings from hex analyze and validation verdicts automatically
 * become work items — closing the validate → fix loop.
 */

import type { ArchAnalysisResult, DependencyViolation, DeadExport, DependencyDirection, RepoHygieneResult } from './value-objects.js';
import type { ValidationVerdict } from './validation-types.js';

// ─── Action Item Types ─────────────────────────────────

export type ActionPriority = 'critical' | 'high' | 'medium' | 'low';
export type ActionCategory = 'bug' | 'violation' | 'dead-code' | 'test-gap' | 'circular-dep' | 'unused-port' | 'hygiene';

export interface ActionItem {
  id: string;
  category: ActionCategory;
  priority: ActionPriority;
  title: string;
  description: string;
  file?: string;
  line?: number;
  layer?: DependencyDirection;
  suggestedFix?: string;
  autoFixable: boolean;
}

export interface ActionItemReport {
  timestamp: string;
  source: 'arch-analysis' | 'validation-verdict' | 'combined';
  totalItems: number;
  byCriticality: { critical: number; high: number; medium: number; low: number };
  byCategory: Partial<Record<ActionCategory, number>>;
  items: ActionItem[];
  summary: string;
}

// ─── ID Generator ──────────────────────────────────────

let actionCounter = 0;

function nextId(prefix: string): string {
  actionCounter++;
  return `${prefix}-${actionCounter.toString().padStart(3, '0')}`;
}

function resetActionCounter(): void {
  actionCounter = 0;
}

// ─── Extract from Architecture Analysis ────────────────

export function extractArchActions(result: ArchAnalysisResult): ActionItem[] {
  const items: ActionItem[] = [];

  // Boundary violations → critical or high priority
  for (const v of result.dependencyViolations) {
    const isCrossAdapter =
      v.fromLayer.startsWith('adapters/') && v.toLayer.startsWith('adapters/');
    const isDomainLeak =
      v.fromLayer === 'domain' && v.toLayer !== 'domain';

    items.push({
      id: nextId('VIO'),
      category: 'violation',
      priority: isCrossAdapter || isDomainLeak ? 'critical' : 'high',
      title: `Hex boundary violation: ${v.fromLayer} → ${v.toLayer}`,
      description: v.rule,
      file: v.from,
      layer: v.fromLayer,
      suggestedFix: suggestViolationFix(v),
      autoFixable: false,
    });
  }

  // Circular dependencies → critical
  for (const cycle of result.circularDeps) {
    items.push({
      id: nextId('CYC'),
      category: 'circular-dep',
      priority: 'critical',
      title: `Circular dependency: ${cycle.length} files in cycle`,
      description: cycle.join(' → ') + ' → [cycle]',
      file: cycle[0],
      autoFixable: false,
    });
  }

  // Unused ports → medium (design smell)
  for (const port of result.unusedPorts) {
    items.push({
      id: nextId('UPT'),
      category: 'unused-port',
      priority: 'medium',
      title: `Unused port interface: ${port}`,
      description: `Port interface ${port} has no adapter implementation. Either implement it or remove it.`,
      suggestedFix: `Implement ${port} in an adapter, or remove it if no longer needed`,
      autoFixable: false,
    });
  }

  // Dead exports → low (cleanup)
  // Group by file to avoid flooding
  const deadByFile = new Map<string, DeadExport[]>();
  for (const d of result.deadExports) {
    if (!deadByFile.has(d.filePath)) deadByFile.set(d.filePath, []);
    deadByFile.get(d.filePath)!.push(d);
  }

  for (const [file, exports] of deadByFile) {
    const names = exports.map((e) => e.exportName).join(', ');
    items.push({
      id: nextId('DEA'),
      category: 'dead-code',
      priority: 'low',
      title: `${exports.length} dead export(s) in ${shortPath(file)}`,
      description: `Unused exports: ${names}`,
      file,
      suggestedFix: `Remove unused exports or add consumers`,
      autoFixable: true,
    });
  }

  return items;
}

// ─── Extract from Validation Verdict ───────────────────

export function extractValidationActions(verdict: ValidationVerdict): ActionItem[] {
  const items: ActionItem[] = [];

  // Failed behavioral specs → high priority bugs
  for (const br of verdict.behavioralResults) {
    if (!br.passed) {
      for (const failure of br.failures) {
        items.push({
          id: nextId('BUG'),
          category: 'bug',
          priority: 'high',
          title: `Behavioral spec failure: ${br.spec.description}`,
          description: failure,
          suggestedFix: extractFixFromFailure(failure),
          autoFixable: false,
        });
      }
    }
  }

  // Failed property tests → high priority (invariant broken)
  for (const pr of verdict.propertyResults) {
    if (!pr.passed) {
      items.push({
        id: nextId('BUG'),
        category: 'bug',
        priority: 'high',
        title: `Property invariant broken: ${pr.spec.description}`,
        description: pr.counterexample
          ? `Counterexample found: ${pr.counterexample}`
          : `Property "${pr.spec.property}" does not hold`,
        autoFixable: false,
      });
    }
  }

  // Failed smoke tests → critical (app doesn't work end-to-end)
  for (const sr of verdict.smokeResults) {
    if (!sr.passed) {
      items.push({
        id: nextId('BUG'),
        category: 'bug',
        priority: 'critical',
        title: `Smoke test failure: ${sr.scenario.description}`,
        description: sr.error
          ? `Failed at step ${sr.failedAtStep ?? '?'}: ${sr.error}`
          : `Scenario did not produce expected outcome: ${sr.scenario.expectedOutcome}`,
        autoFixable: false,
      });
    }
  }

  // Sign convention issues → high (subtle runtime bugs)
  if (!verdict.signConventionAudit.consistent) {
    for (const issue of verdict.signConventionAudit.issues) {
      items.push({
        id: nextId('BUG'),
        category: 'bug',
        priority: 'high',
        title: 'Sign convention inconsistency',
        description: issue,
        suggestedFix: 'Review coordinate system and force sign conventions in domain code',
        autoFixable: false,
      });
    }
  }

  // Low scores in categories → test gap items
  const behavioralScore = verdict.behavioralResults.length > 0
    ? (verdict.behavioralResults.filter((r) => r.passed).length / verdict.behavioralResults.length) * 100
    : 100;
  const propertyScore = verdict.propertyResults.length > 0
    ? (verdict.propertyResults.filter((r) => r.passed).length / verdict.propertyResults.length) * 100
    : 0; // 0 if no property tests exist at all

  if (verdict.propertyResults.length === 0 && verdict.behavioralResults.length > 0) {
    // Only flag missing property tests when the project has behavioral specs
    // (empty verdict = no specs at all, not a gap)
    items.push({
      id: nextId('GAP'),
      category: 'test-gap',
      priority: 'medium',
      title: 'No property tests defined',
      description: 'Property tests provide invariant checking with random inputs. Add property specs for domain functions.',
      suggestedFix: 'Add PropertySpec entries for core domain functions',
      autoFixable: false,
    });
  } else if (verdict.propertyResults.length > 0 && propertyScore < 80) {
    items.push({
      id: nextId('GAP'),
      category: 'test-gap',
      priority: 'medium',
      title: `Property test coverage low: ${propertyScore.toFixed(0)}%`,
      description: `${verdict.propertyResults.filter((r) => !r.passed).length} of ${verdict.propertyResults.length} property tests failing`,
      autoFixable: false,
    });
  }

  if (behavioralScore < 90 && verdict.behavioralResults.length > 0) {
    items.push({
      id: nextId('GAP'),
      category: 'test-gap',
      priority: 'medium',
      title: `Behavioral spec pass rate: ${behavioralScore.toFixed(0)}%`,
      description: `${verdict.behavioralResults.filter((r) => !r.passed).length} behavioral specs failing`,
      autoFixable: false,
    });
  }

  return items;
}

// ─── Extract from Repo Hygiene ─────────────────────────

export function extractHygieneActions(hygiene: RepoHygieneResult): ActionItem[] {
  const items: ActionItem[] = [];
  for (const f of hygiene.findings) {
    const priority: ActionPriority =
      f.severity === 'critical' ? 'high' :
      f.severity === 'warning' ? 'medium' : 'low';
    items.push({
      id: nextId('HYG'),
      category: 'hygiene',
      priority,
      title: `${f.category}: ${f.path}`,
      description: f.description,
      file: f.path,
      suggestedFix: f.suggestedFix,
      autoFixable: f.category === 'build-artifact' || f.category === 'runtime-state',
    });
  }
  return items;
}

// ─── Combined Report ───────────────────────────────────

export function buildActionItemReport(
  archResult?: ArchAnalysisResult,
  verdict?: ValidationVerdict,
): ActionItemReport {
  resetActionCounter();

  const items: ActionItem[] = [];
  let source: ActionItemReport['source'] = 'combined';

  if (archResult) {
    items.push(...extractArchActions(archResult));
    if (archResult.repoHygiene) {
      items.push(...extractHygieneActions(archResult.repoHygiene));
    }
    source = verdict ? 'combined' : 'arch-analysis';
  }
  if (verdict) {
    items.push(...extractValidationActions(verdict));
    if (!archResult) source = 'validation-verdict';
  }

  // Sort: critical first, then high, medium, low
  const priorityOrder: Record<ActionPriority, number> = { critical: 0, high: 1, medium: 2, low: 3 };
  items.sort((a, b) => priorityOrder[a.priority] - priorityOrder[b.priority]);

  const byCriticality = { critical: 0, high: 0, medium: 0, low: 0 };
  const byCategory: Partial<Record<ActionCategory, number>> = {};

  for (const item of items) {
    byCriticality[item.priority]++;
    byCategory[item.category] = (byCategory[item.category] ?? 0) + 1;
  }

  const summary = buildSummaryText(items, byCriticality);

  return {
    timestamp: new Date().toISOString(),
    source,
    totalItems: items.length,
    byCriticality,
    byCategory,
    items,
    summary,
  };
}

// ─── Format Action Items as Text Report ────────────────

export function formatActionItems(report: ActionItemReport): string {
  const lines: string[] = [];
  const ln = (text = '') => lines.push(text);

  ln('='.repeat(60));
  ln('  ACTION ITEMS');
  ln('='.repeat(60));
  ln();
  ln(`  Source:    ${report.source}`);
  ln(`  Items:    ${report.totalItems}`);
  ln(`  Critical: ${report.byCriticality.critical}  High: ${report.byCriticality.high}  Medium: ${report.byCriticality.medium}  Low: ${report.byCriticality.low}`);
  ln();

  if (report.items.length === 0) {
    ln('  No action items. All validations passed.');
    ln();
    ln('='.repeat(60));
    return lines.join('\n');
  }

  // Critical + High items get full detail
  const urgent = report.items.filter((i) => i.priority === 'critical' || i.priority === 'high');
  const rest = report.items.filter((i) => i.priority === 'medium' || i.priority === 'low');

  if (urgent.length > 0) {
    ln('-'.repeat(60));
    ln('  MUST FIX');
    ln('-'.repeat(60));
    ln();
    for (const item of urgent) {
      const tag = item.priority === 'critical' ? '[CRITICAL]' : '[HIGH]';
      ln(`  ${item.id} ${tag} ${item.title}`);
      ln(`    ${item.description}`);
      if (item.file) ln(`    File: ${item.file}${item.line ? `:${item.line}` : ''}`);
      if (item.suggestedFix) ln(`    Fix:  ${item.suggestedFix}`);
      ln();
    }
  }

  if (rest.length > 0) {
    ln('-'.repeat(60));
    ln('  SHOULD FIX');
    ln('-'.repeat(60));
    ln();
    for (const item of rest) {
      const tag = item.priority === 'medium' ? '[MEDIUM]' : '[LOW]';
      ln(`  ${item.id} ${tag} ${item.title}`);
      ln(`    ${item.description}`);
      if (item.suggestedFix) ln(`    Fix:  ${item.suggestedFix}`);
      ln();
    }
  }

  ln('='.repeat(60));
  ln(`  ${report.summary}`);
  ln('='.repeat(60));

  return lines.join('\n');
}

// ─── Helpers ───────────────────────────────────────────

function shortPath(fullPath: string): string {
  const parts = fullPath.split('/');
  return parts.length <= 3 ? fullPath : parts.slice(-3).join('/');
}

function suggestViolationFix(v: DependencyViolation): string {
  if (v.fromLayer.startsWith('adapters/') && v.toLayer.startsWith('adapters/')) {
    return `Move shared logic to a port interface or domain service, then have both adapters depend on the port`;
  }
  if (v.fromLayer === 'domain') {
    return `Domain must be pure — extract the dependency into a port interface that gets injected`;
  }
  if (v.toLayer === 'usecases') {
    return `Adapters should depend on ports, not usecases. Inject the usecase through a port interface`;
  }
  return `Restructure import to go through the ports layer`;
}

function extractFixFromFailure(failure: string): string | undefined {
  // Try to extract actionable fix hints from failure descriptions
  const patterns: Array<[RegExp, string]> = [
    // Specific patterns first, generic fallbacks last
    [/Season\(\d{4}\)/i, 'Replace hardcoded year with time.Now().Year()'],
    [/nil pointer|null reference/i, 'Add nil/null check before accessing the value'],
    [/hardcod(e|ed|es|ing)\s+/i, 'Replace hardcoded value with dynamic computation'],
    [/expected .+ but got/i, 'Check the computation logic against the spec'],
  ];

  for (const [pattern, fix] of patterns) {
    if (pattern.test(failure)) return fix;
  }
  return undefined;
}

function buildSummaryText(
  items: ActionItem[],
  counts: Record<ActionPriority, number>,
): string {
  if (items.length === 0) return 'All clear — no action items.';
  if (counts.critical > 0) {
    return `${counts.critical} critical issue(s) require immediate attention.`;
  }
  if (counts.high > 0) {
    return `${counts.high} high-priority issue(s) should be fixed before shipping.`;
  }
  return `${items.length} item(s) to address — all medium or low priority.`;
}
