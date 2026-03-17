/**
 * Dual Swarm Comparator Use Case
 *
 * Runs two agent executors (Claude Code vs Anthropic API) in parallel
 * against the same specification, then scores and compares the results.
 *
 * Each executor gets its own isolated worktree to prevent conflicts.
 * After execution, both results are evaluated on:
 * - Build success (does it compile?)
 * - Test pass rate
 * - Architecture health (hex analyze score)
 * - Token efficiency (tokens per successful outcome)
 * - Speed (wall clock time)
 */

import type { IAgentExecutorPort } from '../ports/agent-executor.js';
import type { IBuildPort, IArchAnalysisPort, IWorktreePort } from '../ports/index.js';
import type {
  AgentTask,
  ComparisonReport,
  ComparisonEntry,
  ExecutorBackend,
} from '../domain/agent-executor-types.js';

export interface DualSwarmComparatorDeps {
  claudeCodeExecutor: IAgentExecutorPort;
  anthropicApiExecutor: IAgentExecutorPort;
  build: IBuildPort;
  archAnalyzer: IArchAnalysisPort;
  worktree: IWorktreePort;
}

export class DualSwarmComparator {
  constructor(private readonly deps: DualSwarmComparatorDeps) {}

  /**
   * Run both executors against the same task specification.
   * Each gets an isolated worktree; results are scored and compared.
   */
  async compare(
    specification: string,
    taskTemplate: Omit<AgentTask, 'id' | 'projectPath'>,
    onProgress?: (backend: ExecutorBackend, chunk: string) => void,
  ): Promise<ComparisonReport> {
    const { claudeCodeExecutor, anthropicApiExecutor, worktree } = this.deps;

    // Create isolated worktrees for each executor
    const ccWorktree = await worktree.create('compare/claude-code');
    const apiWorktree = await worktree.create('compare/anthropic-api');

    try {
      // Build tasks with isolated project paths
      const ccTask: AgentTask = {
        ...taskTemplate,
        id: `compare-cc-${Date.now()}`,
        prompt: specification,
        projectPath: ccWorktree.absolutePath,
      };

      const apiTask: AgentTask = {
        ...taskTemplate,
        id: `compare-api-${Date.now()}`,
        prompt: specification,
        projectPath: apiWorktree.absolutePath,
      };

      // Load context for both executors
      await Promise.all([
        claudeCodeExecutor.loadContext(ccWorktree.absolutePath),
        anthropicApiExecutor.loadContext(apiWorktree.absolutePath),
      ]);

      // Run both in parallel
      const [ccResult, apiResult] = await Promise.all([
        claudeCodeExecutor.executeWithProgress(ccTask, (chunk) => {
          onProgress?.('claude-code', chunk);
        }),
        anthropicApiExecutor.executeWithProgress(apiTask, (chunk) => {
          onProgress?.('anthropic-api', chunk);
        }),
      ]);

      // Score both results
      const [ccEntry, apiEntry] = await Promise.all([
        this.scoreResult('claude-code', ccTask, ccResult, ccWorktree.absolutePath),
        this.scoreResult('anthropic-api', apiTask, apiResult, apiWorktree.absolutePath),
      ]);

      // Determine winner
      const winner = this.determineWinner(ccEntry, apiEntry);

      const report: ComparisonReport = {
        id: `compare-${Date.now()}`,
        createdAt: new Date().toISOString(),
        specification,
        entries: [ccEntry, apiEntry],
        winner,
        summary: {
          tokenEfficiency: {
            claudeCode: ccResult.metrics.totalInputTokens + ccResult.metrics.totalOutputTokens,
            anthropicApi: apiResult.metrics.totalInputTokens + apiResult.metrics.totalOutputTokens,
          },
          speed: {
            claudeCode: ccResult.metrics.durationMs,
            anthropicApi: apiResult.metrics.durationMs,
          },
          quality: {
            claudeCode: this.computeQualityScore(ccEntry),
            anthropicApi: this.computeQualityScore(apiEntry),
          },
        },
      };

      return report;
    } finally {
      // Cleanup worktrees
      await worktree.cleanup(ccWorktree).catch(() => {});
      await worktree.cleanup(apiWorktree).catch(() => {});
    }
  }

  // ── Scoring ───────────────────────────────────────────

  private async scoreResult(
    backend: ExecutorBackend,
    task: AgentTask,
    result: import('../domain/agent-executor-types.js').AgentResult,
    projectPath: string,
  ): Promise<ComparisonEntry> {
    const { build, archAnalyzer } = this.deps;

    const project = {
      name: `compare-${backend}`,
      rootPath: projectPath,
      language: 'typescript' as const,
      adapters: [],
    };

    // Attempt build
    let buildSuccess = false;
    try {
      const buildResult = await build.compile(project);
      buildSuccess = buildResult.success;
    } catch { /* build failed */ }

    // Attempt tests
    let testPassRate = 0;
    try {
      const testResult = await build.test(project, {
        name: 'all',
        filePaths: [],
        type: 'unit',
      });
      const total = testResult.passed + testResult.failed + testResult.skipped;
      testPassRate = total > 0 ? testResult.passed / total : 0;
    } catch { /* tests failed */ }

    // Architecture analysis
    let archHealthScore = 0;
    try {
      const analysis = await archAnalyzer.analyzeArchitecture(projectPath);
      archHealthScore = analysis.summary.healthScore;
    } catch { /* analysis failed */ }

    return {
      backend,
      task,
      result,
      buildSuccess,
      testPassRate,
      archHealthScore,
    };
  }

  private computeQualityScore(entry: ComparisonEntry): number {
    // Weighted composite: build (40%) + tests (35%) + arch health (25%)
    const buildScore = entry.buildSuccess ? 100 : 0;
    const testScore = entry.testPassRate * 100;
    const archScore = entry.archHealthScore;
    return Math.round(buildScore * 0.4 + testScore * 0.35 + archScore * 0.25);
  }

  private determineWinner(
    cc: ComparisonEntry,
    api: ComparisonEntry,
  ): ExecutorBackend | 'tie' {
    const ccScore = this.computeQualityScore(cc);
    const apiScore = this.computeQualityScore(api);

    // Require > 5 point difference to declare a winner
    if (Math.abs(ccScore - apiScore) <= 5) return 'tie';
    return ccScore > apiScore ? 'claude-code' : 'anthropic-api';
  }
}
