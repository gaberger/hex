import { describe, it, expect } from 'bun:test';
import { FeedbackLoop } from '../../src/core/domain/entities.js';
import type { DomainEvent } from '../../src/core/domain/entities.js';
import { makeQualityScore, makeFeedbackIteration } from './fixtures.js';

describe('FeedbackLoop', () => {
  it('record adds iterations', () => {
    const loop = new FeedbackLoop();
    loop.record(makeFeedbackIteration());
    expect(loop.iterationCount).toBe(1);
  });

  it('current returns the latest iteration', () => {
    const loop = new FeedbackLoop();
    const first = makeFeedbackIteration();
    const second = makeFeedbackIteration();
    loop.record(first);
    loop.record(second);
    expect(loop.current).toBe(second);
  });

  it('current returns undefined when empty', () => {
    const loop = new FeedbackLoop();
    expect(loop.current).toBeUndefined();
  });

  it('iterationCount tracks correctly', () => {
    const loop = new FeedbackLoop();
    expect(loop.iterationCount).toBe(0);
    loop.record(makeFeedbackIteration());
    loop.record(makeFeedbackIteration());
    loop.record(makeFeedbackIteration());
    expect(loop.iterationCount).toBe(3);
  });

  it('canRetry returns true when under maxIterations', () => {
    const loop = new FeedbackLoop(3);
    loop.record(makeFeedbackIteration());
    loop.record(makeFeedbackIteration());
    expect(loop.canRetry).toBe(true);
  });

  it('canRetry returns false when at maxIterations', () => {
    const loop = new FeedbackLoop(2);
    loop.record(makeFeedbackIteration());
    loop.record(makeFeedbackIteration());
    expect(loop.canRetry).toBe(false);
  });

  it('isConverging returns true when score improves', () => {
    const loop = new FeedbackLoop();
    loop.record(makeFeedbackIteration({ quality: makeQualityScore({ testsPassed: 5, testsFailed: 5 }) }));
    loop.record(makeFeedbackIteration({ quality: makeQualityScore({ testsPassed: 10, testsFailed: 0 }) }));
    expect(loop.isConverging).toBe(true);
  });

  it('isConverging returns false when score drops', () => {
    const loop = new FeedbackLoop();
    loop.record(makeFeedbackIteration({ quality: makeQualityScore({ testsPassed: 10, testsFailed: 0 }) }));
    loop.record(makeFeedbackIteration({ quality: makeQualityScore({ testsPassed: 5, testsFailed: 5 }) }));
    expect(loop.isConverging).toBe(false);
  });

  it('isConverging returns true with fewer than 2 iterations', () => {
    const loop = new FeedbackLoop();
    expect(loop.isConverging).toBe(true);
    loop.record(makeFeedbackIteration());
    expect(loop.isConverging).toBe(true);
  });

  it('toEvents collects all events from all iterations', () => {
    const loop = new FeedbackLoop();
    const event1: DomainEvent = { type: 'BuildSucceeded', payload: { duration: 100 } };
    const event2: DomainEvent = { type: 'LintPassed', payload: { filePath: 'a.ts', warningCount: 0 } };
    loop.record(makeFeedbackIteration({ events: [event1] }));
    loop.record(makeFeedbackIteration({ events: [event2] }));
    const all = loop.toEvents();
    expect(all).toHaveLength(2);
    expect(all[0]).toBe(event1);
    expect(all[1]).toBe(event2);
  });

  it('custom maxIterations is respected', () => {
    const loop = new FeedbackLoop(10);
    expect(loop.maxIterations).toBe(10);
    for (let i = 0; i < 9; i++) loop.record(makeFeedbackIteration());
    expect(loop.canRetry).toBe(true);
    loop.record(makeFeedbackIteration());
    expect(loop.canRetry).toBe(false);
  });
});
