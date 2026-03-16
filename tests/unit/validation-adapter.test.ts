import { describe, it, expect } from 'bun:test';
import { ValidationAdapter } from '../../src/adapters/secondary/validation-adapter.js';

describe('ValidationAdapter', () => {
  it('generateBehavioralSpecs creates specs from sentences', async () => {
    const adapter = new ValidationAdapter();
    const specs = await adapter.generateBehavioralSpecs('When user clicks the button. The score increases.');
    expect(specs).toHaveLength(2);
    expect(specs[0].assertions.length).toBeGreaterThan(0);
    expect(specs[0].category).toBe('input'); // "clicks" keyword
    expect(specs[1].category).toBe('state'); // no special keyword
  });

  it('generateBehavioralSpecs returns fallback for empty-ish input', async () => {
    const adapter = new ValidationAdapter();
    const specs = await adapter.generateBehavioralSpecs('');
    expect(specs).toHaveLength(1);
    expect(specs[0].category).toBe('state');
  });

  it('generatePropertySpecs creates one spec per function', async () => {
    const adapter = new ValidationAdapter();
    const specs = await adapter.generatePropertySpecs(['update', 'render']);
    expect(specs).toHaveLength(2);
    expect(specs[0].description).toContain('update');
    expect(specs[0].numTrials).toBe(100);
  });

  it('generateSmokeScenarios converts specs to smoke tests', async () => {
    const adapter = new ValidationAdapter();
    const bSpecs = await adapter.generateBehavioralSpecs('Player taps the screen.');
    const scenarios = await adapter.generateSmokeScenarios(bSpecs);
    expect(scenarios).toHaveLength(bSpecs.length);
    expect(scenarios[0].steps.length).toBeGreaterThanOrEqual(3);
    const actions = scenarios[0].steps.map((s) => s.action);
    expect(actions).toContain('wait');
    expect(actions).toContain('input');
    expect(actions).toContain('assert');
  });

  it('auditSignConventions returns empty for missing file', async () => {
    const adapter = new ValidationAdapter();
    const conventions = await adapter.auditSignConventions('/nonexistent/path.ts');
    expect(conventions).toEqual([]);
  });

  it('validate returns skeleton verdict with passed=false', async () => {
    const adapter = new ValidationAdapter();
    const bSpecs = await adapter.generateBehavioralSpecs('Something happens.');
    const pSpecs = await adapter.generatePropertySpecs(['fn']);
    const scenarios = await adapter.generateSmokeScenarios(bSpecs);
    const verdict = await adapter.validate(bSpecs, pSpecs, scenarios, []);
    expect(verdict.passed).toBe(false);
    expect(verdict.overallScore).toBe(0);
    expect(verdict.behavioralResults).toHaveLength(bSpecs.length);
    expect(verdict.propertyResults).toHaveLength(1);
    expect(verdict.signConventionAudit.consistent).toBe(true);
  });

  it('validate reports inconsistent when conventions present', async () => {
    const adapter = new ValidationAdapter();
    const verdict = await adapter.validate([], [], [], [{
      coordinate: 'Y axis',
      positiveDirection: 'downward',
      forces: [{ name: 'gravity', sign: 'positive', description: 'test' }],
    }]);
    expect(verdict.signConventionAudit.consistent).toBe(false);
    expect(verdict.signConventionAudit.issues.length).toBeGreaterThan(0);
  });
});
