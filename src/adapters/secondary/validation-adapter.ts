/**
 * Validation Adapter — local heuristic-based implementation of IValidationPort.
 *
 * Generates specs from keyword analysis and returns skeleton verdicts.
 * No LLM dependency — pure heuristic extraction.
 */

import { readFile } from 'node:fs/promises';
import type {
  IValidationPort,
  BehavioralSpec,
  PropertySpec,
  SmokeScenario,
  SmokeStep,
  SignConvention,
  ValidationVerdict,
} from '../../core/ports/validation.js';

const SIGN_KEYWORDS = ['gravity', 'velocity', 'force', 'acceleration', 'speed', 'friction', 'drag', 'impulse', 'thrust'];

let idCounter = 0;
function nextId(prefix: string): string {
  return `${prefix}-${++idCounter}`;
}

export class ValidationAdapter implements IValidationPort {
  async generateBehavioralSpecs(problemDescription: string): Promise<BehavioralSpec[]> {
    const specs: BehavioralSpec[] = [];
    const sentences = problemDescription.split(/[.!?\n]+/).map((s) => s.trim()).filter(Boolean);

    for (const sentence of sentences) {
      const lower = sentence.toLowerCase();
      const category = this.categorize(lower);
      specs.push({
        id: nextId('bspec'),
        description: sentence,
        category,
        assertions: [{
          given: 'the system is in its initial state',
          when: `${sentence}`,
          then: 'the expected behavior occurs',
        }],
      });
    }
    if (specs.length === 0) {
      specs.push({
        id: nextId('bspec'),
        description: problemDescription,
        category: 'state',
        assertions: [{
          given: 'the system exists',
          when: 'the feature is invoked',
          then: 'it completes without error',
        }],
      });
    }
    return specs;
  }

  async generatePropertySpecs(domainFunctions: string[]): Promise<PropertySpec[]> {
    return domainFunctions.map((fn) => ({
      id: nextId('pspec'),
      description: `${fn} returns a value of the expected type`,
      property: `typeof ${fn}(input) !== 'undefined'`,
      domain: 'arbitrary valid input',
      numTrials: 100,
    }));
  }

  async generateSmokeScenarios(behavioralSpecs: BehavioralSpec[]): Promise<SmokeScenario[]> {
    return behavioralSpecs.map((spec) => {
      const steps: SmokeStep[] = [];
      for (const assertion of spec.assertions) {
        steps.push({ action: 'wait', detail: `Set up: ${assertion.given}`, durationMs: 0 });
        steps.push({ action: 'input', detail: `Execute: ${assertion.when}` });
        steps.push({ action: 'assert', detail: `Verify: ${assertion.then}` });
      }
      return {
        id: nextId('smoke'),
        description: `Smoke test for: ${spec.description}`,
        steps,
        expectedOutcome: 'All assertions pass',
      };
    });
  }

  async auditSignConventions(domainPath: string): Promise<SignConvention[]> {
    let source: string;
    try {
      source = await readFile(domainPath, 'utf-8');
    } catch {
      return [];
    }
    const conventions: SignConvention[] = [];
    const lower = source.toLowerCase();
    const forces: SignConvention['forces'] = [];

    for (const keyword of SIGN_KEYWORDS) {
      if (lower.includes(keyword)) {
        const isNegative = this.hasNegativeAssignment(source, keyword);
        forces.push({
          name: keyword,
          sign: isNegative ? 'negative' : 'positive',
          description: `${keyword} appears to be ${isNegative ? 'negative' : 'positive'} in source`,
        });
      }
    }
    if (forces.length > 0) {
      conventions.push({
        coordinate: 'Y axis',
        positiveDirection: 'downward (screen coordinates)',
        forces,
      });
    }
    return conventions;
  }

  async validate(
    specs: BehavioralSpec[],
    properties: PropertySpec[],
    scenarios: SmokeScenario[],
    conventions: SignConvention[],
  ): Promise<ValidationVerdict> {
    return {
      passed: false,
      behavioralResults: specs.map((spec) => ({ spec, passed: false, failures: ['Not yet validated'] })),
      propertyResults: properties.map((spec) => ({ spec, passed: false })),
      smokeResults: scenarios.map((scenario) => ({ scenario, passed: false, error: 'Not yet validated' })),
      signConventionAudit: {
        consistent: conventions.length === 0,
        issues: conventions.length > 0 ? ['Sign conventions detected but not yet validated'] : [],
      },
      overallScore: 0,
    };
  }

  // ── Private helpers ──────────────────────────────────────

  private categorize(text: string): BehavioralSpec['category'] {
    if (/click|tap|key|press|input|button/.test(text)) return 'input';
    if (/gravity|velocity|force|collision|physics|bounce/.test(text)) return 'physics';
    if (/render|draw|display|show|canvas|screen/.test(text)) return 'rendering';
    if (/sound|audio|music|beep/.test(text)) return 'audio';
    if (/save|load|store|persist|database/.test(text)) return 'persistence';
    return 'state';
  }

  private hasNegativeAssignment(source: string, keyword: string): boolean {
    const pattern = new RegExp(`${keyword}\\s*[:=]\\s*-`, 'i');
    return pattern.test(source);
  }
}
