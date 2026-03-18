/**
 * Workplan Executor use case -- implements IWorkplanPort.
 *
 * Decomposes requirements into adapter-bounded tasks using LLM,
 * then executes them respecting dependency ordering via TaskGraph.
 */
import type {
  IASTPort,
  ICodeGenerationPort,
  IFileSystemPort,
  ILLMPort,
  IWorkplanPort,
  Language,
  Message,
  Specification,
  StepResult,
  TokenBudget,
  Workplan,
  WorkplanStep,
} from '../ports/index.js';
import type { ISwarmPort } from '../ports/swarm.js';
import type { ICoordinationPort } from '../ports/coordination.js';
import { TaskGraph } from '../domain/entities.js';
import { TaskConflictError } from '../domain/errors.js';

const PLAN_BUDGET: TokenBudget = {
  maxTokens: 16000,
  reservedForResponse: 4096,
  available: 11904,
};

export class WorkplanExecutor implements IWorkplanPort {
  constructor(
    private readonly llm: ILLMPort,
    private readonly ast: IASTPort,
    private readonly fs: IFileSystemPort,
    private readonly swarm: ISwarmPort,
    /** Optional code generator — when provided, executePlan generates real code per step */
    private readonly codeGenerator?: ICodeGenerationPort,
    /** Language for code generation — defaults to 'typescript' */
    private readonly defaultLang: Language = 'typescript',
    private readonly coordination: ICoordinationPort | null = null,
  ) {}

  async createPlan(requirements: string[], lang: Language): Promise<Workplan> {
    const [projectContext, relevantPatterns] = await Promise.all([
      this.summarizeProject(),
      this.findRelevantPatterns(requirements),
    ]);

    const patternHints = relevantPatterns.length > 0
      ? [
          '',
          '## Learned Patterns (from prior successful runs)',
          ...relevantPatterns.map((p) =>
            `- [${p.category}] ${p.name} (confidence: ${p.confidence}): ${p.content.slice(0, 200)}`,
          ),
        ]
      : [];

    const messages: Message[] = [
      {
        role: 'system',
        content: [
          'You are a technical planner for a hexagonal architecture project.',
          'Decompose requirements into adapter-bounded steps.',
          'Each step should target a specific adapter or use case boundary.',
          '',
          'Respond in JSON format:',
          '{ "title": "...", "steps": [{ "id": "step-1", "description": "...", "adapter": "...", "dependencies": [] }] }',
          '',
          'Rules:',
          '- Each step targets one adapter boundary (e.g., "secondary/llm-adapter")',
          '- Dependencies reference other step IDs',
          '- Order steps so dependencies come first',
          '- If learned patterns are provided, prefer approaches that worked before',
        ].join('\n'),
      },
      {
        role: 'user',
        content: [
          `## Project Context (${lang})`,
          projectContext,
          ...patternHints,
          '',
          '## Requirements',
          ...requirements.map((r, i) => `${i + 1}. ${r}`),
          '',
          'Create a workplan.',
        ].join('\n'),
      },
    ];

    const response = await this.llm.prompt(PLAN_BUDGET, messages);
    return this.parsePlanResponse(response.content);
  }

  async *executePlan(plan: Workplan): AsyncGenerator<StepResult> {
    const graph = new TaskGraph();
    for (const step of plan.steps) {
      graph.addStep(step);
    }

    // Start a tracked session so progress is visible in dashboard
    let session: { sessionId: string } | null = null;
    try {
      session = await this.swarm.sessionStart('workplan-executor', {
        planId: plan.id,
        planTitle: plan.title,
        stepCount: plan.steps.length,
      });
    } catch { /* session tracking is best-effort */ }

    // ── Recover prior progress from AgentDB ────────────────
    // If a previous execution crashed, completed steps are persisted
    // under workplan/<planId>/progress. Hydrate the completed set so
    // we skip those steps on resume.
    const completed = new Set<string>();
    const failed = new Set<string>();

    const priorProgress = await this.recoverProgress(plan.id);
    if (priorProgress) {
      for (const stepId of priorProgress.completed) completed.add(stepId);
      for (const stepId of priorProgress.failed) failed.add(stepId);
      if (completed.size > 0 || failed.size > 0) {
        process.stderr.write(
          `[hex] Resuming workplan ${plan.id}: ${completed.size} completed, ${failed.size} failed — skipping those steps\n`,
        );
      }
    }

    const sorted = graph.topologicalSort();

    for (const step of sorted) {
      // ── Skip already-completed steps on resume ─────────
      if (completed.has(step.id)) {
        yield { stepId: step.id, status: 'passed' };
        continue;
      }
      // Re-attempt previously failed steps (user may have fixed the issue)

      const depsReady = step.dependencies.every((d) => completed.has(d));
      if (!depsReady) {
        failed.add(step.id);
        step.status = 'failed';
        step.error = 'Unmet dependencies';
        step.completedAt = new Date().toISOString();
        await this.persistProgress(plan.id, completed, failed);
        yield { stepId: step.id, status: 'failed', errors: ['Unmet dependencies'] };
        continue;
      }

      yield { stepId: step.id, status: 'running' };
      step.status = 'running';

      let task: { id: string } | null = null;
      try {
        // Register task in swarm for tracking/dashboard visibility
        task = await this.swarm.createTask({
          title: step.description,
          agentRole: 'coder',
          adapter: step.adapter,
        });
        // Claim task via coordination (prevents duplicate work across instances)
        if (this.coordination) {
          const claim = await this.coordination.claimTask(task.id);
          if (!claim.claimed) {
            const holder = claim.conflict?.instanceId ?? 'unknown';
            throw new TaskConflictError(task.id, holder);
          }
        }

        // Pass coordination context to agent via swarm memory
        if (this.coordination) {
          try {
            const [unstaged, activities] = await Promise.all([
              this.coordination.getUnstagedAcrossInstances(),
              this.coordination.getActivities(10),
            ]);
            await this.swarm.memoryStore({
              key: `task:${task.id}:coordination`,
              value: JSON.stringify({ unstaged, activities }),
              namespace: 'hex',
              tags: ['coordination-context'],
            });
          } catch { /* coordination context is best-effort */ }
        }

        await this.swarm.spawnAgent(`worker-${step.id}`, 'coder', task.id);

        // Execute: generate real code if code generator is available
        let output;
        if (this.codeGenerator) {
          const spec: Specification = {
            title: step.description,
            requirements: [step.description],
            constraints: [
              `Target adapter boundary: ${step.adapter}`,
              'Follow hex architecture rules strictly',
            ],
            targetLanguage: this.defaultLang,
            targetAdapter: step.adapter,
          };

          // Build context from completed steps' outputs
          const priorOutputs = [...completed].map(id => {
            const priorStep = sorted.find(s => s.id === id);
            return priorStep ? `- [${id}] ${priorStep.description} (${priorStep.adapter})` : '';
          }).filter(Boolean);

          if (priorOutputs.length > 0) {
            spec.constraints.push(
              `Already completed:\n${priorOutputs.join('\n')}`,
            );
          }

          output = await this.codeGenerator.generateFromSpec(spec, this.defaultLang);
        }

        await this.swarm.completeTask(task.id, `Completed: ${step.description}`);
        // Release task claim and publish activity
        if (this.coordination) {
          await this.coordination.releaseTask(task.id).catch(() => {});
          await this.coordination.publishActivity('task-complete', {
            taskId: task.id, stepId: step.id, adapter: step.adapter,
          }).catch(() => {});
        }
        completed.add(step.id);
        step.status = 'passed';
        step.completedAt = new Date().toISOString();

        // ── Persist progress to AgentDB after each step ──
        await this.persistProgress(plan.id, completed, failed);

        // Store successful step as a learned pattern
        await this.storeStepPattern(step, 'success');

        yield { stepId: step.id, status: 'passed', output };
      } catch (err) {
        // Release task claim on failure
        if (this.coordination && task) {
          await this.coordination.releaseTask(task.id).catch(() => {});
        }
        const message = err instanceof Error ? err.message : String(err);
        failed.add(step.id);
        step.status = 'failed';
        step.error = message;
        step.completedAt = new Date().toISOString();

        // ── Persist progress to AgentDB after failure ────
        await this.persistProgress(plan.id, completed, failed);

        // Record failure pattern for future avoidance
        await this.storeStepPattern(step, 'failure', message);

        yield { stepId: step.id, status: 'failed', errors: [message] };
      }
    }

    // End the session with a summary
    if (session) {
      try {
        await this.swarm.hierarchicalStore(
          'workplan', plan.id, 'summary',
          JSON.stringify({ completed: completed.size, failed: failed.size, total: sorted.length }),
          ['workplan', 'summary'],
        );
        await this.swarm.sessionEnd(session.sessionId);
      } catch { /* best-effort */ }
    }
  }

  // ── Progress Persistence (AgentDB) ──────────────────────

  /** Shape of the persisted progress record in AgentDB. */
  private static readonly PROGRESS_KEY = 'progress';

  /**
   * Persist step completion state to AgentDB so a crashed session can resume.
   * Stored at: workplan/<planId>/progress
   */
  private async persistProgress(
    planId: string,
    completed: Set<string>,
    failed: Set<string>,
  ): Promise<void> {
    try {
      const payload = JSON.stringify({
        completed: [...completed],
        failed: [...failed],
        updatedAt: new Date().toISOString(),
      });
      await this.swarm.hierarchicalStore(
        'workplan', planId, WorkplanExecutor.PROGRESS_KEY, payload,
        ['workplan-progress', 'crash-recovery'],
      );
    } catch {
      // Progress persistence is best-effort — never break the execution loop
    }
  }

  /**
   * Recover prior execution progress from AgentDB.
   * Returns null if no prior progress exists (fresh plan).
   */
  private async recoverProgress(
    planId: string,
  ): Promise<{ completed: string[]; failed: string[] } | null> {
    try {
      const entries = await this.swarm.hierarchicalRecall(
        'workplan', planId, WorkplanExecutor.PROGRESS_KEY,
      );
      if (entries.length === 0) return null;
      // Most recent entry (hierarchicalRecall returns newest-first)
      const parsed = JSON.parse(entries[0].value) as {
        completed?: string[];
        failed?: string[];
      };
      return {
        completed: parsed.completed ?? [],
        failed: parsed.failed ?? [],
      };
    } catch {
      return null;
    }
  }

  // ── Private helpers ───────────────────────────────────────

  private async summarizeProject(): Promise<string> {
    const globResults = await Promise.all([
      this.fs.glob('src/**/*.ts'),
      this.fs.glob('src/**/*.go'),
      this.fs.glob('src/**/*.rs'),
      this.fs.glob('internal/**/*.go'),
      this.fs.glob('cmd/**/*.go'),
    ]);
    const allFiles = globResults.flat().filter(
      (f) => !f.includes('node_modules') && !f.includes('dist') && !f.includes('.test.') && !f.includes('_test.go'),
    );

    const sections: string[] = [];

    // Tier 1: L0 overview of ALL source files (cheap — just filenames)
    sections.push('### Project Files (L0)');
    sections.push(...allFiles.map(f => `- ${f}`));
    sections.push('');

    // Tier 2: L1 detail for ports + usecases (the planning-critical layers)
    const criticalFiles = allFiles.filter(f =>
      f.includes('/ports/') || f.includes('/usecases/') || f.includes('/domain/'),
    );

    if (criticalFiles.length > 0) {
      sections.push('### Core Layer Detail (L1)');
      for (const file of criticalFiles.slice(0, 30)) {
        try {
          const summary = await this.ast.extractSummary(file, 'L1');
          const exports = summary.exports.map((e) => `${e.kind} ${e.name}`).join(', ');
          sections.push(`- ${file}: ${exports || '(no exports)'}`);
        } catch {
          sections.push(`- ${file}: (parse error)`);
        }
      }
    }

    return sections.join('\n');
  }

  private async findRelevantPatterns(requirements: string[]) {
    try {
      const query = requirements.join(' ').slice(0, 200);
      return await this.swarm.patternSearch(query, 'workplan', 5);
    } catch {
      return [];
    }
  }

  private async storeStepPattern(
    step: { id: string; description: string; adapter: string },
    outcome: 'success' | 'failure',
    error?: string,
  ): Promise<void> {
    try {
      const pattern = await this.swarm.patternStore({
        name: `${outcome}: ${step.adapter}`,
        category: 'workplan',
        content: JSON.stringify({
          stepId: step.id,
          description: step.description,
          adapter: step.adapter,
          outcome,
          ...(error ? { error } : {}),
        }),
        confidence: outcome === 'success' ? 0.8 : 0.2,
        tags: ['workplan', step.adapter, outcome],
      });

      if (outcome === 'failure' && pattern.id) {
        await this.swarm.patternFeedback({
          patternId: pattern.id,
          outcome: 'failure',
          score: 0.2,
          context: step.adapter,
          details: error,
        });
      }
    } catch { /* pattern storage is best-effort */ }
  }

  private parsePlanResponse(content: string): Workplan {
    const jsonMatch = content.match(/\{[\s\S]*\}/);
    if (!jsonMatch) {
      throw new Error('LLM response did not contain valid JSON workplan');
    }

    const raw: unknown = JSON.parse(jsonMatch[0]);
    if (typeof raw !== 'object' || raw === null) {
      throw new Error('LLM response JSON is not an object');
    }
    const parsed = raw as {
      title?: string;
      steps?: Array<{
        id: string;
        description: string;
        adapter: string;
        dependencies?: string[];
      }>;
    };
    if (parsed.steps !== undefined && !Array.isArray(parsed.steps)) {
      throw new Error('LLM response "steps" field is not an array');
    }

    const steps: WorkplanStep[] = (parsed.steps ?? []).map((s) => ({
      id: s.id,
      description: s.description,
      adapter: s.adapter,
      dependencies: s.dependencies ?? [],
    }));

    return {
      id: `plan-${Date.now()}`,
      title: parsed.title ?? 'Untitled Workplan',
      steps,
      estimatedTokenBudget: steps.length * PLAN_BUDGET.reservedForResponse,
    };
  }
}
