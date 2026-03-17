/**
 * Post-Build Validation Port
 *
 * Catches semantic bugs that compile-lint-test pipelines miss.
 * Runs behavioral specs, property tests, and smoke tests against
 * the built application.
 *
 * Value types are owned by domain/validation-types.ts and re-exported here.
 */

import type {
  BehavioralSpec,
  PropertySpec,
  SmokeScenario,
  SignConvention,
  ValidationVerdict,
} from '../domain/validation-types.js';

// ─── Re-export domain types for public API stability ────

export type {
  BehavioralSpec,
  PropertySpec,
  SmokeScenario,
  SmokeStep,
  SignConvention,
  ValidationVerdict,
} from '../domain/validation-types.js';

// ─── Input Port (Primary / Driving) ──────────────────────

export interface IValidationPort {
  /** Generate behavioral specs from a problem description */
  generateBehavioralSpecs(
    problemDescription: string,
  ): Promise<BehavioralSpec[]>;

  /** Generate property test specs for domain functions */
  generatePropertySpecs(
    domainFunctions: string[],
  ): Promise<PropertySpec[]>;

  /** Generate smoke test scenarios */
  generateSmokeScenarios(
    behavioralSpecs: BehavioralSpec[],
  ): Promise<SmokeScenario[]>;

  /** Extract sign conventions from domain code */
  auditSignConventions(domainPath: string): Promise<SignConvention[]>;

  /** Run full post-build validation */
  validate(
    specs: BehavioralSpec[],
    properties: PropertySpec[],
    scenarios: SmokeScenario[],
    conventions: SignConvention[],
  ): Promise<ValidationVerdict>;
}
