/**
 * Test fixtures for domain entity unit tests.
 * Provides factory functions with sensible defaults.
 */

import type {
  CodeUnit,
  BuildResult,
  LintError,
  TestResult,
  WorkplanStep,
  ASTSummary,
} from '../../src/core/ports/index.js';

import { QualityScore, type FeedbackIteration, type DomainEvent } from '../../src/core/domain/entities.js';

// ─── Quality Score Factory ──────────────────────────────

interface QualityScoreOverrides {
  compileSuccess?: boolean;
  lintErrorCount?: number;
  lintWarningCount?: number;
  testsPassed?: number;
  testsFailed?: number;
  tokenEfficiency?: number;
}

export function makeQualityScore(overrides: QualityScoreOverrides = {}): QualityScore {
  return new QualityScore(
    overrides.compileSuccess ?? true,
    overrides.lintErrorCount ?? 0,
    overrides.lintWarningCount ?? 0,
    overrides.testsPassed ?? 10,
    overrides.testsFailed ?? 0,
    overrides.tokenEfficiency ?? 0.2,
  );
}

// ─── Feedback Iteration Factory ─────────────────────────

interface FeedbackIterationOverrides {
  quality?: QualityScore;
  events?: DomainEvent[];
  buildSuccess?: boolean;
  testsPassed?: number;
  testsFailed?: number;
}

export function makeFeedbackIteration(overrides: FeedbackIterationOverrides = {}): FeedbackIteration {
  const quality = overrides.quality ?? makeQualityScore();
  const summary: ASTSummary = {
    filePath: 'test.ts',
    language: 'typescript',
    level: 'L0',
    exports: [],
    imports: [],
    dependencies: [],
    lineCount: 10,
    tokenEstimate: 50,
  };
  return {
    code: { filePath: 'test.ts', language: 'typescript', content: '// test', astSummary: summary },
    build: { success: overrides.buildSuccess ?? true, errors: [], duration: 100 },
    lintErrors: [],
    testResult: {
      success: (overrides.testsFailed ?? 0) === 0,
      passed: overrides.testsPassed ?? 10,
      failed: overrides.testsFailed ?? 0,
      skipped: 0,
      duration: 200,
      failures: [],
    },
    quality,
    events: overrides.events ?? [],
  };
}

// ─── Workplan Step Factory ──────────────────────────────

interface WorkplanStepOverrides {
  id?: string;
  description?: string;
  adapter?: string;
  dependencies?: string[];
  assignee?: string;
}

export function makeWorkplanStep(overrides: WorkplanStepOverrides = {}): WorkplanStep {
  return {
    id: overrides.id ?? 'step-1',
    description: overrides.description ?? 'Default step',
    adapter: overrides.adapter ?? 'typescript',
    dependencies: overrides.dependencies ?? [],
    assignee: overrides.assignee,
  };
}
