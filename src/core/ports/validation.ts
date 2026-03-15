/**
 * Post-Build Validation Port
 *
 * Catches semantic bugs that compile-lint-test pipelines miss.
 * Runs behavioral specs, property tests, and smoke tests against
 * the built application.
 */

// ─── Behavioral Specification ────────────────────────────

export interface BehavioralSpec {
  id: string;
  description: string;            // "When player taps, bird moves upward"
  category: 'input' | 'physics' | 'state' | 'rendering' | 'audio' | 'persistence';
  assertions: BehavioralAssertion[];
}

export interface BehavioralAssertion {
  given: string;                   // "bird is at y=300, velocity=0"
  when: string;                    // "flap() is called"
  then: string;                    // "bird velocity is negative (upward)"
  testFn?: string;                 // optional: code to execute for automated check
}

// ─── Property Test Specification ─────────────────────────

export interface PropertySpec {
  id: string;
  description: string;            // "flap always produces upward movement"
  property: string;               // the invariant as code
  domain: string;                 // what inputs to generate
  numTrials: number;              // how many random inputs to try
}

// ─── Smoke Test ──────────────────────────────────────────

export interface SmokeScenario {
  id: string;
  description: string;            // "basic gameplay: tap 10 times, score > 0"
  steps: SmokeStep[];
  expectedOutcome: string;
}

export interface SmokeStep {
  action: 'wait' | 'input' | 'assert';
  detail: string;                 // "tick(16)" or "flap()" or "score > 0"
  durationMs?: number;
}

// ─── Sign Convention Contract ────────────────────────────

export interface SignConvention {
  coordinate: string;             // "Y axis"
  positiveDirection: string;      // "downward (screen coordinates)"
  forces: Array<{
    name: string;                 // "gravity"
    sign: 'positive' | 'negative'; // "positive (pushes down)"
    description: string;
  }>;
}

// ─── Validation Results ──────────────────────────────────

export interface ValidationVerdict {
  passed: boolean;
  behavioralResults: Array<{
    spec: BehavioralSpec;
    passed: boolean;
    failures: string[];
  }>;
  propertyResults: Array<{
    spec: PropertySpec;
    passed: boolean;
    counterexample?: string;
  }>;
  smokeResults: Array<{
    scenario: SmokeScenario;
    passed: boolean;
    failedAtStep?: number;
    error?: string;
  }>;
  signConventionAudit: {
    consistent: boolean;
    issues: string[];
  };
  overallScore: number;           // 0-100
}

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
