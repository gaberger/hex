import { describe, it, expect } from 'bun:test';
import { runCLI } from '../../src/adapters/primary/cli-adapter.js';
import type { AppContext } from '../../src/core/ports/app-context.js';

/** Minimal mock AppContext for CLI go tests */
function mockCtx(): AppContext {
  return {
    rootPath: '/tmp/test-project',
    autoConfirm: false,
    outputDir: '/tmp/test-project/.hex',
    astIsStub: true,
    archAnalyzer: {} as any,
    summaryService: {} as any,
    notificationOrchestrator: null,
    llm: null,
    codeGenerator: null,
    workplanExecutor: null,
    swarmOrchestrator: {} as any,
    fs: {
      read: async () => '// ports content',
      write: async () => {},
      exists: async () => false,
      glob: async () => [],
    },
    git: {
      commit: async () => 'abc123',
      createBranch: async () => {},
      diff: async () => '',
      currentBranch: async () => 'main',
    },
    worktree: {
      create: async (branch: string) => ({ absolutePath: `/tmp/wt-${branch}`, branch }),
      merge: async () => ({ success: true, conflicts: [], commitHash: 'def456' }),
      cleanup: async () => {},
      list: async () => [],
    },
    build: {} as any,
    ast: {} as any,
    eventBus: null,
    notifier: {} as any,
    swarm: {} as any,
    registry: {} as any,
    broadcaster: {} as any,
    secrets: {
      resolveSecret: async () => ({ ok: false as const, error: 'not set' }),
      hasSecret: async () => false,
      listSecrets: async () => [],
    },
  };
}

describe('hex go', () => {
  it('shows usage when no prompt is provided (G08)', async () => {
    const result = await runCLI(['go'], mockCtx(), () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Usage: hex go');
  });

  it('shows the session banner with prompt and mode (G01)', async () => {
    const result = await runCLI(
      ['go', 'add user auth', '--yolo', '--dry-run'],
      mockCtx(),
      () => {},
    );
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('hex go');
    expect(result.output).toContain('autonomous coding');
    expect(result.output).toContain('YOLO');
  });

  it('creates a worktree by default (G02)', async () => {
    let createdBranch = '';
    const ctx = mockCtx();
    ctx.worktree.create = async (branch: string) => {
      createdBranch = branch;
      return { absolutePath: `/tmp/wt-${branch}`, branch };
    };

    await runCLI(['go', 'test feature', '--dry-run'], ctx, () => {});
    expect(createdBranch).toContain('hex-go/');
    expect(createdBranch).toContain('test-feature');
  });

  it('skips worktree with --no-worktree flag (G02 negative)', async () => {
    let worktreeCreated = false;
    const ctx = mockCtx();
    ctx.worktree.create = async () => {
      worktreeCreated = true;
      return { absolutePath: '/tmp/wt', branch: 'test' };
    };

    const result = await runCLI(
      ['go', 'fix bug', '--no-worktree', '--dry-run'],
      ctx,
      () => {},
    );
    expect(worktreeCreated).toBe(false);
    expect(result.output).toContain('--no-worktree');
  });

  it('generates a slug from the prompt for the branch name', async () => {
    let createdBranch = '';
    const ctx = mockCtx();
    ctx.worktree.create = async (branch: string) => {
      createdBranch = branch;
      return { absolutePath: `/tmp/wt-${branch}`, branch };
    };

    await runCLI(
      ['go', 'Add Stripe payments!!!', '--dry-run'],
      ctx,
      () => {},
    );
    expect(createdBranch).toMatch(/^hex-go\/add-stripe-payments/);
  });

  it('dry-run exits 0 without launching agent', async () => {
    const result = await runCLI(
      ['go', 'some task', '--dry-run'],
      mockCtx(),
      () => {},
    );
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Dry run');
    expect(result.output).not.toContain('Launching Claude Code');
  });

  it('shows review mode in banner', async () => {
    const result = await runCLI(
      ['go', 'refactor auth', '--review', '--dry-run'],
      mockCtx(),
      () => {},
    );
    expect(result.output).toContain('REVIEW');
  });
});
