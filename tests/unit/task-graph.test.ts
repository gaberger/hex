import { describe, it, expect } from 'bun:test';
import { TaskGraph } from '../../src/core/domain/entities.js';
import { makeWorkplanStep } from './fixtures.js';

describe('TaskGraph', () => {
  it('addStep adds steps', () => {
    const graph = new TaskGraph();
    graph.addStep(makeWorkplanStep({ id: 'a' }));
    expect(graph.size).toBe(1);
  });

  it('size tracks step count', () => {
    const graph = new TaskGraph();
    expect(graph.size).toBe(0);
    graph.addStep(makeWorkplanStep({ id: 'a' }));
    graph.addStep(makeWorkplanStep({ id: 'b' }));
    expect(graph.size).toBe(2);
  });

  it('getStep retrieves by ID', () => {
    const graph = new TaskGraph();
    const step = makeWorkplanStep({ id: 'x', description: 'Find me' });
    graph.addStep(step);
    expect(graph.getStep('x')).toBe(step);
  });

  it('getStep returns undefined for missing ID', () => {
    const graph = new TaskGraph();
    expect(graph.getStep('nonexistent')).toBeUndefined();
  });

  it('getReady returns steps with satisfied dependencies', () => {
    const graph = new TaskGraph();
    graph.addStep(makeWorkplanStep({ id: 'a', dependencies: [] }));
    graph.addStep(makeWorkplanStep({ id: 'b', dependencies: ['a'] }));
    graph.addStep(makeWorkplanStep({ id: 'c', dependencies: ['missing'] }));

    const ready = graph.getReady();
    const readyIds = ready.map((s) => s.id);
    expect(readyIds).toContain('a');
    expect(readyIds).toContain('b');
    expect(readyIds).not.toContain('c');
  });

  it('topologicalSort returns correct order for chain dependencies', () => {
    const graph = new TaskGraph();
    // Add in reverse order to prove sorting works
    graph.addStep(makeWorkplanStep({ id: 'c', dependencies: ['b'] }));
    graph.addStep(makeWorkplanStep({ id: 'b', dependencies: ['a'] }));
    graph.addStep(makeWorkplanStep({ id: 'a', dependencies: [] }));

    const sorted = graph.topologicalSort();
    const ids = sorted.map((s) => s.id);
    expect(ids.indexOf('a')).toBeLessThan(ids.indexOf('b'));
    expect(ids.indexOf('b')).toBeLessThan(ids.indexOf('c'));
  });

  it('topologicalSort handles parallel independent steps', () => {
    const graph = new TaskGraph();
    graph.addStep(makeWorkplanStep({ id: 'a', dependencies: [] }));
    graph.addStep(makeWorkplanStep({ id: 'b', dependencies: [] }));
    graph.addStep(makeWorkplanStep({ id: 'c', dependencies: [] }));

    const sorted = graph.topologicalSort();
    expect(sorted).toHaveLength(3);
    const ids = sorted.map((s) => s.id);
    expect(ids).toContain('a');
    expect(ids).toContain('b');
    expect(ids).toContain('c');
  });

  it('topologicalSort handles diamond dependencies (A->B, A->C, B->D, C->D)', () => {
    const graph = new TaskGraph();
    graph.addStep(makeWorkplanStep({ id: 'A', dependencies: [] }));
    graph.addStep(makeWorkplanStep({ id: 'B', dependencies: ['A'] }));
    graph.addStep(makeWorkplanStep({ id: 'C', dependencies: ['A'] }));
    graph.addStep(makeWorkplanStep({ id: 'D', dependencies: ['B', 'C'] }));

    const sorted = graph.topologicalSort();
    const ids = sorted.map((s) => s.id);
    expect(ids.indexOf('A')).toBeLessThan(ids.indexOf('B'));
    expect(ids.indexOf('A')).toBeLessThan(ids.indexOf('C'));
    expect(ids.indexOf('B')).toBeLessThan(ids.indexOf('D'));
    expect(ids.indexOf('C')).toBeLessThan(ids.indexOf('D'));
  });
});
