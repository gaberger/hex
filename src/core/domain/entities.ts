/**
 * Domain Entities for hex-intf
 *
 * Pure domain objects with no external dependencies.
 * All state changes emit domain events for traceability.
 */

import type {
  Language,
  CodeUnit,
  LintError,
  BuildResult,
  TestResult,
  WorkplanStep,
} from './value-objects.js';

// ─── Domain Events ───────────────────────────────────────

export type DomainEvent =
  // ── Code Lifecycle ──
  | { type: 'CodeGenerated'; payload: { filePath: string; language: Language; tokenCount: number } }
  | { type: 'LintPassed'; payload: { filePath: string; warningCount: number } }
  | { type: 'LintFailed'; payload: { filePath: string; errors: LintError[] } }
  | { type: 'TestsPassed'; payload: { suite: string; passed: number; duration: number } }
  | { type: 'TestsFailed'; payload: { suite: string; failures: string[] } }
  | { type: 'BuildSucceeded'; payload: { duration: number } }
  | { type: 'BuildFailed'; payload: { errors: string[] } }
  // ── Workflow Lifecycle ──
  | { type: 'WorkplanCreated'; payload: { planId: string; stepCount: number } }
  | { type: 'StepCompleted'; payload: { stepId: string; status: 'passed' | 'failed' } }
  | { type: 'SwarmSpawned'; payload: { agentCount: number; topology: string } }
  // ── Developer Awareness ──
  | { type: 'DecisionRequested'; payload: { agentName: string; question: string; optionCount: number } }
  | { type: 'DecisionResolved'; payload: { agentName: string; selectedOption: string; respondedBy: 'human' | 'auto_timeout' | 'escalation_agent' } }
  | { type: 'AgentStalled'; payload: { agentName: string; adapter: string; stalledSince: number } }
  | { type: 'QualityRegressed'; payload: { agentName: string; previousScore: number; currentScore: number; iteration: number } }
  | { type: 'PhaseCompleted'; payload: { phase: 'plan' | 'execute' | 'integrate' | 'package'; duration: number } };

// ─── Quality Score ───────────────────────────────────────

export class QualityScore {
  constructor(
    readonly compileSuccess: boolean,
    readonly lintErrorCount: number,
    readonly lintWarningCount: number,
    readonly testsPassed: number,
    readonly testsFailed: number,
    readonly tokenEfficiency: number, // ratio: summary tokens / source tokens
  ) {}

  get score(): number {
    if (!this.compileSuccess) return 0;
    const lintPenalty = this.lintErrorCount * 10 + this.lintWarningCount * 2;
    const testScore = this.testsPassed / Math.max(1, this.testsPassed + this.testsFailed);
    const efficiency = Math.min(1, this.tokenEfficiency * 5); // 20% ratio = perfect
    return Math.max(0, Math.min(100, testScore * 60 + efficiency * 20 + Math.max(0, 20 - lintPenalty)));
  }

  get passing(): boolean {
    return this.compileSuccess && this.lintErrorCount === 0 && this.testsFailed === 0;
  }
}

// ─── Feedback Loop ───────────────────────────────────────

export class FeedbackLoop {
  private iterations: FeedbackIteration[] = [];
  readonly maxIterations: number;

  constructor(maxIterations = 5) {
    this.maxIterations = maxIterations;
  }

  record(iteration: FeedbackIteration): void {
    this.iterations.push(iteration);
  }

  get current(): FeedbackIteration | undefined {
    return this.iterations[this.iterations.length - 1];
  }

  get iterationCount(): number {
    return this.iterations.length;
  }

  get canRetry(): boolean {
    return this.iterations.length < this.maxIterations;
  }

  get isConverging(): boolean {
    if (this.iterations.length < 2) return true;
    const prev = this.iterations[this.iterations.length - 2];
    const curr = this.iterations[this.iterations.length - 1];
    return curr.quality.score >= prev.quality.score;
  }

  toEvents(): DomainEvent[] {
    return this.iterations.flatMap((it) => it.events);
  }
}

export interface FeedbackIteration {
  code: CodeUnit;
  build: BuildResult;
  lintErrors: LintError[];
  testResult: TestResult;
  quality: QualityScore;
  events: DomainEvent[];
}

// ─── Task Graph ──────────────────────────────────────────

export class TaskGraph {
  private steps: Map<string, WorkplanStep> = new Map();

  addStep(step: WorkplanStep): void {
    this.steps.set(step.id, step);
  }

  getReady(): WorkplanStep[] {
    return Array.from(this.steps.values()).filter((step) => {
      return step.dependencies.every((depId) => {
        const dep = this.steps.get(depId);
        return dep !== undefined; // In real impl, check completion status
      });
    });
  }

  getStep(id: string): WorkplanStep | undefined {
    return this.steps.get(id);
  }

  get size(): number {
    return this.steps.size;
  }

  /** Returns steps in topological order for sequential execution */
  topologicalSort(): WorkplanStep[] {
    const visited = new Set<string>();
    const result: WorkplanStep[] = [];

    const visit = (id: string): void => {
      if (visited.has(id)) return;
      visited.add(id);
      const step = this.steps.get(id);
      if (!step) return;
      for (const depId of step.dependencies) {
        visit(depId);
      }
      result.push(step);
    };

    for (const id of this.steps.keys()) {
      visit(id);
    }

    return result;
  }
}
