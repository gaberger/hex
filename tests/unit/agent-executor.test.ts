import { describe, it, expect, mock } from 'bun:test';
import type { IAgentExecutorPort } from '../../src/core/ports/agent-executor.js';
import type { AgentTask, AgentResult, AgentMetrics, AgentContext, ExecutorBackend } from '../../src/core/domain/agent-executor-types.js';
import { DualSwarmComparator } from '../../src/core/usecases/dual-swarm-comparator.js';
import type { IBuildPort, IArchAnalysisPort, IWorktreePort, WorktreePath, BuildResult, LintResult, TestResult, ArchAnalysisResult, Project, TestSuite } from '../../src/core/ports/index.js';

// ── Mock Factories ─────────────────────────────────────

function mockMetrics(overrides?: Partial<AgentMetrics>): AgentMetrics {
  return {
    totalInputTokens: 1000,
    totalOutputTokens: 500,
    totalTurns: 3,
    totalToolCalls: 5,
    toolCallBreakdown: { read_file: 2, write_file: 2, bash: 1 },
    durationMs: 10000,
    model: 'claude-sonnet-4-20250514',
    ...overrides,
  };
}

function mockResult(taskId: string, overrides?: Partial<AgentResult>): AgentResult {
  return {
    taskId,
    status: 'success',
    output: 'Task completed successfully',
    filesChanged: ['src/main.ts'],
    toolCalls: [],
    metrics: mockMetrics(),
    ...overrides,
  };
}

function mockExecutor(backend: ExecutorBackend, resultOverrides?: Partial<AgentResult>): IAgentExecutorPort {
  return {
    backend,
    async loadContext(): Promise<AgentContext> {
      return {
        systemPrompt: 'test prompt',
        sources: [],
        projectPath: '/tmp/test',
        claudeMd: '',
      };
    },
    async execute(task: AgentTask): Promise<AgentResult> {
      return mockResult(task.id, resultOverrides);
    },
    async executeWithProgress(task: AgentTask, onProgress: (chunk: string) => void): Promise<AgentResult> {
      onProgress('Working...');
      return mockResult(task.id, resultOverrides);
    },
  };
}

function mockBuild(buildSuccess = true, testPassRate = 1.0): IBuildPort {
  return {
    async compile(): Promise<BuildResult> {
      return { success: buildSuccess, errors: [], duration: 1000 };
    },
    async lint(): Promise<LintResult> {
      return { success: true, errors: [], warningCount: 0, errorCount: 0 };
    },
    async test(): Promise<TestResult> {
      const passed = Math.round(testPassRate * 10);
      const failed = 10 - passed;
      return { success: failed === 0, passed, failed, skipped: 0, duration: 2000, failures: [] };
    },
  };
}

function mockArchAnalyzer(healthScore = 85): IArchAnalysisPort {
  return {
    async buildDependencyGraph() { return []; },
    async findDeadExports() { return []; },
    async validateHexBoundaries() { return []; },
    async detectCircularDeps() { return []; },
    async analyzeArchitecture(): Promise<ArchAnalysisResult> {
      return {
        deadExports: [],
        orphanFiles: [],
        dependencyViolations: [],
        circularDeps: [],
        unusedPorts: [],
        unusedAdapters: [],
        summary: {
          totalFiles: 10,
          totalExports: 50,
          deadExportCount: 0,
          violationCount: 0,
          circularCount: 0,
          healthScore,
        },
      };
    },
  };
}

function mockWorktree(): IWorktreePort {
  let counter = 0;
  return {
    async create(branch: string): Promise<WorktreePath> {
      return { absolutePath: `/tmp/worktree-${++counter}`, branch };
    },
    async merge() { return { success: true, conflicts: [], commitHash: 'abc123' }; },
    async cleanup() {},
    async list() { return []; },
  };
}

// ── Tests ──────────────────────────────────────────────

describe('DualSwarmComparator', () => {
  it('runs both executors and produces a comparison report', async () => {
    const comparator = new DualSwarmComparator({
      claudeCodeExecutor: mockExecutor('claude-code'),
      anthropicApiExecutor: mockExecutor('anthropic-api'),
      build: mockBuild(),
      archAnalyzer: mockArchAnalyzer(),
      worktree: mockWorktree(),
    });

    const report = await comparator.compare(
      'Create a hello world app',
      { prompt: 'Create a hello world app', role: 'coder' },
    );

    expect(report.entries).toHaveLength(2);
    expect(report.entries[0].backend).toBe('claude-code');
    expect(report.entries[1].backend).toBe('anthropic-api');
    expect(report.specification).toBe('Create a hello world app');
    expect(report.id).toBeTruthy();
    expect(report.createdAt).toBeTruthy();
  });

  it('declares a tie when scores are within 5 points', async () => {
    const comparator = new DualSwarmComparator({
      claudeCodeExecutor: mockExecutor('claude-code'),
      anthropicApiExecutor: mockExecutor('anthropic-api'),
      build: mockBuild(true, 1.0),
      archAnalyzer: mockArchAnalyzer(85),
      worktree: mockWorktree(),
    });

    const report = await comparator.compare(
      'Build a calculator',
      { prompt: 'Build a calculator', role: 'coder' },
    );

    // Both get same build, test, arch scores → tie
    expect(report.winner).toBe('tie');
  });

  it('declares anthropic-api winner when claude-code build fails', async () => {
    // Need separate build mocks for each worktree
    let callCount = 0;
    const build: IBuildPort = {
      async compile(): Promise<BuildResult> {
        callCount++;
        // First call (claude-code) fails, second (anthropic-api) succeeds
        return { success: callCount > 1, errors: [], duration: 1000 };
      },
      async lint(): Promise<LintResult> {
        return { success: true, errors: [], warningCount: 0, errorCount: 0 };
      },
      async test(): Promise<TestResult> {
        return { success: true, passed: 10, failed: 0, skipped: 0, duration: 2000, failures: [] };
      },
    };

    const comparator = new DualSwarmComparator({
      claudeCodeExecutor: mockExecutor('claude-code'),
      anthropicApiExecutor: mockExecutor('anthropic-api'),
      build,
      archAnalyzer: mockArchAnalyzer(85),
      worktree: mockWorktree(),
    });

    const report = await comparator.compare(
      'Build something',
      { prompt: 'Build something', role: 'coder' },
    );

    expect(report.winner).toBe('anthropic-api');
  });

  it('tracks token efficiency in summary', async () => {
    const comparator = new DualSwarmComparator({
      claudeCodeExecutor: mockExecutor('claude-code', {
        metrics: mockMetrics({ totalInputTokens: 5000, totalOutputTokens: 2000 }),
      }),
      anthropicApiExecutor: mockExecutor('anthropic-api', {
        metrics: mockMetrics({ totalInputTokens: 3000, totalOutputTokens: 1000 }),
      }),
      build: mockBuild(),
      archAnalyzer: mockArchAnalyzer(),
      worktree: mockWorktree(),
    });

    const report = await comparator.compare(
      'Optimize code',
      { prompt: 'Optimize code', role: 'coder' },
    );

    expect(report.summary.tokenEfficiency.claudeCode).toBe(7000);
    expect(report.summary.tokenEfficiency.anthropicApi).toBe(4000);
  });

  it('calls progress callback for both backends', async () => {
    const progress: Array<{ backend: string; chunk: string }> = [];
    const comparator = new DualSwarmComparator({
      claudeCodeExecutor: mockExecutor('claude-code'),
      anthropicApiExecutor: mockExecutor('anthropic-api'),
      build: mockBuild(),
      archAnalyzer: mockArchAnalyzer(),
      worktree: mockWorktree(),
    });

    await comparator.compare(
      'Test progress',
      { prompt: 'Test progress', role: 'coder' },
      (backend, chunk) => progress.push({ backend, chunk }),
    );

    expect(progress.length).toBeGreaterThanOrEqual(2);
    expect(progress.some((p) => p.backend === 'claude-code')).toBe(true);
    expect(progress.some((p) => p.backend === 'anthropic-api')).toBe(true);
  });

  it('cleans up worktrees even on error', async () => {
    let cleanupCount = 0;
    const wt: IWorktreePort = {
      async create(branch: string): Promise<WorktreePath> {
        return { absolutePath: `/tmp/wt-${branch}`, branch };
      },
      async merge() { return { success: true, conflicts: [], commitHash: 'abc' }; },
      async cleanup() { cleanupCount++; },
      async list() { return []; },
    };

    const failingExecutor: IAgentExecutorPort = {
      backend: 'claude-code',
      async loadContext() { return { systemPrompt: '', sources: [], projectPath: '', claudeMd: '' }; },
      async execute() { throw new Error('boom'); },
      async executeWithProgress() { throw new Error('boom'); },
    };

    const comparator = new DualSwarmComparator({
      claudeCodeExecutor: failingExecutor,
      anthropicApiExecutor: mockExecutor('anthropic-api'),
      build: mockBuild(),
      archAnalyzer: mockArchAnalyzer(),
      worktree: wt,
    });

    try {
      await comparator.compare('test', { prompt: 'test', role: 'coder' });
    } catch {
      // Expected
    }

    // Both worktrees should be cleaned up
    expect(cleanupCount).toBe(2);
  });
});

describe('AgentExecutorPort contract', () => {
  it('mock executor fulfills the port interface', () => {
    const executor = mockExecutor('anthropic-api');
    expect(executor.backend).toBe('anthropic-api');
    expect(typeof executor.loadContext).toBe('function');
    expect(typeof executor.execute).toBe('function');
    expect(typeof executor.executeWithProgress).toBe('function');
  });

  it('AgentResult contains required metrics fields', async () => {
    const executor = mockExecutor('claude-code');
    const result = await executor.execute({
      id: 'test-1',
      prompt: 'do something',
      projectPath: '/tmp',
    });

    expect(result.taskId).toBe('test-1');
    expect(result.status).toBe('success');
    expect(result.metrics.totalInputTokens).toBeGreaterThan(0);
    expect(result.metrics.totalOutputTokens).toBeGreaterThan(0);
    expect(result.metrics.model).toBeTruthy();
  });
});
