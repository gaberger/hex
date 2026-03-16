/**
 * Workplan Executor use case -- implements IWorkplanPort.
 *
 * Decomposes requirements into adapter-bounded tasks using LLM,
 * then executes them respecting dependency ordering via TaskGraph.
 */
import type {
  IASTPort,
  IFileSystemPort,
  ILLMPort,
  IWorkplanPort,
  Language,
  Message,
  StepResult,
  TokenBudget,
  Workplan,
  WorkplanStep,
} from '../ports/index.js';
import type { ISwarmPort } from '../ports/swarm.js';
import { TaskGraph } from '../domain/entities.js';

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
  ) {}

  async createPlan(requirements: string[], lang: Language): Promise<Workplan> {
    const projectContext = await this.summarizeProject();

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
        ].join('\n'),
      },
      {
        role: 'user',
        content: [
          `## Project Context (${lang})`,
          projectContext,
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

    const completed = new Set<string>();
    const sorted = graph.topologicalSort();

    for (const step of sorted) {
      const depsReady = step.dependencies.every((d) => completed.has(d));
      if (!depsReady) {
        yield { stepId: step.id, status: 'failed', errors: ['Unmet dependencies'] };
        continue;
      }

      yield { stepId: step.id, status: 'running' };

      try {
        const task = await this.swarm.createTask({
          title: step.description,
          agentRole: 'coder',
          adapter: step.adapter,
        });

        await this.swarm.spawnAgent(`worker-${step.id}`, 'coder', task.id);
        await this.swarm.completeTask(task.id, `Completed: ${step.description}`);
        completed.add(step.id);

        yield { stepId: step.id, status: 'passed' };
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        yield { stepId: step.id, status: 'failed', errors: [message] };
      }
    }
  }

  // ── Private helpers ───────────────────────────────────────

  private async summarizeProject(): Promise<string> {
    const files = await this.fs.glob('src/**/*.ts');
    const filtered = files.filter(
      (f) => !f.includes('node_modules') && !f.includes('dist') && !f.includes('.test.'),
    );

    const lines: string[] = [];
    for (const file of filtered.slice(0, 20)) {
      try {
        const summary = await this.ast.extractSummary(file, 'L1');
        const exports = summary.exports.map((e) => `${e.kind} ${e.name}`).join(', ');
        lines.push(`- ${file}: ${exports || '(no exports)'}`);
      } catch {
        lines.push(`- ${file}: (parse error)`);
      }
    }
    return lines.join('\n');
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
