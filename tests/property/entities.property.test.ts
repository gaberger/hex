import { describe, it, expect } from 'bun:test';
import fc from 'fast-check';
import { QualityScore, FeedbackLoop, TaskGraph } from '../../src/core/domain/entities.js';
import type { WorkplanStep } from '../../src/core/domain/value-objects.js';

describe('QualityScore (property tests)', () => {
  it('score is always between 0 and 100', () => {
    fc.assert(
      fc.property(
        fc.boolean(),
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.double({ min: 0, max: 1, noNaN: true }),
        (compileSuccess, lintErrors, lintWarnings, testsPassed, testsFailed, tokenEfficiency) => {
          const qs = new QualityScore(
            compileSuccess, lintErrors, lintWarnings,
            testsPassed, testsFailed, tokenEfficiency,
          );
          expect(qs.score).toBeGreaterThanOrEqual(0);
          expect(qs.score).toBeLessThanOrEqual(100);
        },
      ),
    );
  });

  it('score is 0 when compilation fails', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.double({ min: 0, max: 1, noNaN: true }),
        (lintErrors, lintWarnings, testsPassed, testsFailed, tokenEfficiency) => {
          const qs = new QualityScore(
            false, lintErrors, lintWarnings,
            testsPassed, testsFailed, tokenEfficiency,
          );
          expect(qs.score).toBe(0);
        },
      ),
    );
  });

  it('passing requires compile success, zero lint errors, and zero test failures', () => {
    fc.assert(
      fc.property(
        fc.boolean(),
        fc.nat({ max: 50 }),
        fc.nat({ max: 50 }),
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.double({ min: 0, max: 1, noNaN: true }),
        (compileSuccess, lintErrors, lintWarnings, testsPassed, testsFailed, tokenEfficiency) => {
          const qs = new QualityScore(
            compileSuccess, lintErrors, lintWarnings,
            testsPassed, testsFailed, tokenEfficiency,
          );
          if (qs.passing) {
            expect(compileSuccess).toBe(true);
            expect(lintErrors).toBe(0);
            expect(testsFailed).toBe(0);
          }
        },
      ),
    );
  });

  it('more lint errors never increase the score', () => {
    fc.assert(
      fc.property(
        fc.nat({ max: 50 }),
        fc.nat({ max: 50 }),
        fc.nat({ max: 100 }),
        fc.nat({ max: 100 }),
        fc.double({ min: 0, max: 1, noNaN: true }),
        fc.integer({ min: 1, max: 50 }),
        (lintErrors, lintWarnings, testsPassed, testsFailed, tokenEfficiency, extraErrors) => {
          const qs1 = new QualityScore(true, lintErrors, lintWarnings, testsPassed, testsFailed, tokenEfficiency);
          const qs2 = new QualityScore(true, lintErrors + extraErrors, lintWarnings, testsPassed, testsFailed, tokenEfficiency);
          expect(qs2.score).toBeLessThanOrEqual(qs1.score);
        },
      ),
    );
  });
});

describe('FeedbackLoop (property tests)', () => {
  it('canRetry is true when iterations < maxIterations', () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 20 }),
        (maxIterations) => {
          const loop = new FeedbackLoop(maxIterations);
          expect(loop.canRetry).toBe(true);
          expect(loop.iterationCount).toBe(0);
        },
      ),
    );
  });

  it('iterationCount matches the number of recorded iterations', () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 10 }),
        fc.integer({ min: 0, max: 10 }),
        (maxIterations, recordCount) => {
          const loop = new FeedbackLoop(maxIterations);
          const actualRecords = Math.min(recordCount, 10);
          for (let i = 0; i < actualRecords; i++) {
            loop.record({
              code: { filePath: 'test.ts', language: 'typescript', content: '', astSummary: { filePath: 'test.ts', language: 'typescript', level: 'L0', exports: [], imports: [], dependencies: [], lineCount: 0, tokenEstimate: 0 } },
              build: { success: true, errors: [], duration: 0 },
              lintErrors: [],
              testResult: { success: true, passed: 1, failed: 0, skipped: 0, duration: 0, failures: [] },
              quality: new QualityScore(true, 0, 0, 1, 0, 0.2),
              events: [],
            });
          }
          expect(loop.iterationCount).toBe(actualRecords);
        },
      ),
    );
  });
});

describe('TaskGraph (property tests)', () => {
  const stepIdArb = fc.string({ minLength: 1, maxLength: 20 }).map(s => `step-${s}`);

  it('size matches number of added steps', () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.record({
            id: stepIdArb,
            description: fc.string(),
            adapter: fc.string(),
            dependencies: fc.constant([] as string[]),
          }),
          { maxLength: 20 },
        ),
        (steps) => {
          const graph = new TaskGraph();
          const uniqueSteps = new Map<string, WorkplanStep>();
          for (const step of steps) {
            uniqueSteps.set(step.id, step);
            graph.addStep(step);
          }
          // TaskGraph uses Map, so duplicate IDs overwrite
          expect(graph.size).toBe(uniqueSteps.size);
        },
      ),
    );
  });

  it('getStep returns added steps', () => {
    fc.assert(
      fc.property(
        stepIdArb,
        fc.string(),
        fc.string(),
        (id, description, adapter) => {
          const graph = new TaskGraph();
          const step: WorkplanStep = { id, description, adapter, dependencies: [] };
          graph.addStep(step);
          expect(graph.getStep(id)).toEqual(step);
        },
      ),
    );
  });

  it('topological sort includes all steps', () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.record({
            id: stepIdArb,
            description: fc.string(),
            adapter: fc.string(),
            dependencies: fc.constant([] as string[]),
          }),
          { maxLength: 15 },
        ),
        (steps) => {
          const graph = new TaskGraph();
          const uniqueIds = new Set<string>();
          for (const step of steps) {
            uniqueIds.add(step.id);
            graph.addStep(step);
          }
          const sorted = graph.topologicalSort();
          expect(sorted).toHaveLength(uniqueIds.size);
        },
      ),
    );
  });

  it('topological sort respects dependency ordering', () => {
    // Build a known DAG: A -> B -> C (C depends on B, B depends on A)
    const graph = new TaskGraph();
    graph.addStep({ id: 'A', description: 'first', adapter: 'domain', dependencies: [] });
    graph.addStep({ id: 'B', description: 'second', adapter: 'ports', dependencies: ['A'] });
    graph.addStep({ id: 'C', description: 'third', adapter: 'adapter', dependencies: ['B'] });

    const sorted = graph.topologicalSort();
    const idxA = sorted.findIndex(s => s.id === 'A');
    const idxB = sorted.findIndex(s => s.id === 'B');
    const idxC = sorted.findIndex(s => s.id === 'C');

    expect(idxA).toBeLessThan(idxB);
    expect(idxB).toBeLessThan(idxC);
  });

  it('all ready steps have their dependencies present in the graph', () => {
    fc.assert(
      fc.property(
        fc.array(
          fc.record({
            id: stepIdArb,
            description: fc.string(),
            adapter: fc.string(),
            dependencies: fc.constant([] as string[]),
          }),
          { maxLength: 10 },
        ),
        (steps) => {
          const graph = new TaskGraph();
          for (const step of steps) {
            graph.addStep(step);
          }
          const ready = graph.getReady();
          // All steps with no deps should be ready
          for (const step of ready) {
            for (const depId of step.dependencies) {
              expect(graph.getStep(depId)).toBeDefined();
            }
          }
        },
      ),
    );
  });
});
