/**
 * Smoke Tests — CLI Lifecycle
 *
 * Verifies that the CLI can actually start, parse commands, and produce
 * meaningful output. These are the "can it start?" tests that catch
 * wiring failures the unit tests (which mock everything) would miss.
 *
 * Smoke tests use minimal mocks — just enough to avoid real I/O.
 */

import { describe, it, expect } from 'bun:test';
import { runCLI } from '../../src/adapters/primary/cli-adapter.js';
import type { AppContext } from '../../src/core/ports/app-context.js';
import type {
  IArchAnalysisPort,
  IASTPort,
  IFileSystemPort,
  IGitPort,
  IWorktreePort,
  IBuildPort,
  ArchAnalysisResult,
  ASTSummary,
} from '../../src/core/ports/index.js';

// ── Minimal stub context (no real I/O, but all required fields) ──

function stubContext(): AppContext {
  const archAnalyzer: IArchAnalysisPort = {
    buildDependencyGraph: async () => [],
    findDeadExports: async () => [],
    validateHexBoundaries: async () => [],
    detectCircularDeps: async () => [],
    analyzeArchitecture: async () => ({
      deadExports: [], orphanFiles: [], dependencyViolations: [],
      circularDeps: [], unusedPorts: [], unusedAdapters: [],
      summary: {
        totalFiles: 5, totalExports: 10, deadExportCount: 0,
        violationCount: 0, circularCount: 0, healthScore: 100,
      },
    }),
  };

  const ast: IASTPort = {
    extractSummary: async (fp, lvl) => ({
      filePath: fp, language: 'typescript', level: lvl,
      exports: [], imports: [], dependencies: [],
      lineCount: 10, tokenEstimate: 50,
    }),
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
  };

  const fs: IFileSystemPort = {
    read: async () => '', write: async () => {},
    exists: async (p: string) => p === 'package.json' || p === 'src',
    glob: async () => [],
    mtime: async () => 0,
  };

  const git: IGitPort = {
    commit: async () => 'abc1234',
    createBranch: async () => {},
    diff: async () => '',
    currentBranch: async () => 'main',
    statusEntries: async () => [],
    worktreeEntries: async () => [],
    findEmbeddedRepos: async () => [],
  };

  const worktree: IWorktreePort = {
    create: async () => '/tmp/worktree' as any,
    merge: async () => ({ success: true, conflicts: [] }),
    cleanup: async () => {},
    list: async () => [],
  };

  const build: IBuildPort = {
    compile: async () => ({ success: true, errors: [] }),
    lint: async () => ({ passed: true, errors: [] }),
    test: async () => ({ passed: true, failures: [], duration: 0 }),
  };

  const noopSwarm = {
    init: async () => ({ status: 'idle' as const }),
    createTask: async (t: any) => t,
    completeTask: async () => {},
    spawnAgent: async () => ({ id: '1', name: 'test', role: 'coder' as any, status: 'idle' as any }),
    terminateAgent: async () => {},
    getStatus: async () => ({ agents: [], tasks: [], topology: 'mesh' as const }),
    patternStore: async (p: any) => p,
    patternSearch: async () => [],
    patternFeedback: async () => {},
    patternStats: async () => ({ total: 0, categories: {} }),
    memoryStore: async () => {},
    memoryRetrieve: async () => null,
    memorySearch: async () => [],
    hierarchicalStore: async () => {},
    hierarchicalRecall: async () => [],
    consolidate: async () => ({ merged: 0, removed: 0 }),
    contextSynthesize: async () => '',
    getProgressReport: async () => ({ tasks: [], agents: [], summary: '' }),
  };

  return {
    rootPath: '/smoke-test',
    astIsStub: true,
    autoConfirm: true,
    archAnalyzer,
    summaryService: {
      summarizeFile: async (fp, lvl) => ({
        filePath: fp, language: 'typescript', level: lvl,
        exports: [], imports: [], dependencies: [],
        lineCount: 10, tokenEstimate: 50,
      }),
      summarizeProject: async () => [],
    },
    notificationOrchestrator: null,
    llm: null,
    codeGenerator: null,
    workplanExecutor: null,
    swarmOrchestrator: {
      orchestrate: async () => ({ success: true, results: [] }),
      getProgress: async () => ({ completed: 0, total: 0, tasks: [] }),
    },
    fs, git, worktree, build, ast,
    eventBus: null,
    notifier: { emit: async () => {} },
    swarm: noopSwarm as any,
    registry: {
      register: async () => {},
      unregister: async () => {},
      list: async () => [],
      get: async () => null,
    },
    secrets: {
      getSecret: async () => ({ found: false }),
      listSecrets: async () => [],
    } as any,
    checkpoint: {
      save: async () => {},
      load: async () => null,
      list: async () => [],
      clear: async () => {},
    } as any,
    scaffold: {
      scaffold: async () => ({ success: true, path: '/tmp' }),
      detectRuntime: async () => null,
    } as any,
    validator: null,
    serialization: {
      serialize: async (d: any) => JSON.stringify(d),
      deserialize: async (s: any) => JSON.parse(s),
    } as any,
    wasmBridge: null,
    ffi: null,
    serviceMesh: null,
    schema: {
      validate: async () => ({ valid: true, errors: [] }),
      generateSchema: async () => ({}),
    } as any,
    version: {
      getCliVersion: () => ({ major: 0, minor: 1, patch: 0, toString: () => '0.1.0' }),
      getHubVersion: async () => null,
      getVersionInfo: async () => ({ cli: { major: 0, minor: 1, patch: 0, toString: () => '0.1.0' }, hub: null, mismatch: false }),
    },
    hubLauncher: null,
    vaultManager: {
      createVault() {},
      addSecret() { throw new Error('No vault open'); },
      removeSecret() { throw new Error('No vault open'); },
    },
    hubCommandSender: null,
    anthropicExecutor: null,
    claudeCodeExecutor: null,
    outputDir: '.hex/',
  };
}

// ── Smoke: CLI starts and responds to basic commands ────────

describe('Smoke: CLI lifecycle', () => {
  it('help command exits 0 and prints usage', async () => {
    const ctx = stubContext();
    const result = await runCLI(['help'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Usage:');
  });

  it('--help flag exits 0', async () => {
    const ctx = stubContext();
    const result = await runCLI(['--help'], ctx, () => {});
    expect(result.exitCode).toBe(0);
  });

  it('version command exits 0', async () => {
    const ctx = stubContext();
    const result = await runCLI(['version'], ctx, () => {});
    expect(result.exitCode).toBe(0);
  });

  it('analyze command completes without crash', async () => {
    const ctx = stubContext();
    const result = await runCLI(['analyze'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('100');
  });

  it('summarize command with valid args completes', async () => {
    const ctx = stubContext();
    const result = await runCLI(['summarize', 'src/foo.ts'], ctx, () => {});
    expect(result.exitCode).toBe(0);
  });

  it('unknown command exits 1 with error message', async () => {
    const ctx = stubContext();
    const result = await runCLI(['nonexistent-command'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Unknown command');
  });

  it('no command defaults to help', async () => {
    const ctx = stubContext();
    const result = await runCLI([], ctx, () => {});
    // Should either show help or prompt — not crash
    expect(result.exitCode).toBe(0);
  });

  it('status command completes', async () => {
    const ctx = stubContext();
    const result = await runCLI(['status'], ctx, () => {});
    // Status may exit 0 or 1 depending on swarm state, but must not throw
    expect(typeof result.exitCode).toBe('number');
  });

  it('validate command runs architecture check', async () => {
    const ctx = stubContext();
    const result = await runCLI(['validate'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Phase 1');
    expect(result.output).toContain('PASS');
  });

  it('validate command reports violations as FAIL', async () => {
    const ctx = stubContext();
    ctx.archAnalyzer.analyzeArchitecture = async () => ({
      deadExports: [], orphanFiles: [], dependencyViolations: [],
      circularDeps: [['a.ts', 'b.ts', 'a.ts']], unusedPorts: [], unusedAdapters: [],
      summary: {
        totalFiles: 5, totalExports: 10, deadExportCount: 0,
        violationCount: 0, circularCount: 1, healthScore: 80,
      },
    });
    const result = await runCLI(['validate'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('FAIL');
  });

  it('build is an alias for go (shows usage without args)', async () => {
    const ctx = stubContext();
    const result = await runCLI(['build'], ctx, () => {});
    // Without a prompt, go/build shows usage and exits 1
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Usage:');
  });

  it('scaffold command runs without crash', async () => {
    const ctx = stubContext();
    const result = await runCLI(['scaffold', 'test-project'], ctx, () => {});
    // Scaffold delegates to init; may fail on missing dirs but must not throw
    expect(typeof result.exitCode).toBe('number');
  });

  it('orchestrate without args shows usage', async () => {
    const ctx = stubContext();
    const result = await runCLI(['orchestrate'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Usage:');
  });

  it('help text includes new commands', async () => {
    const ctx = stubContext();
    const result = await runCLI(['help'], ctx, () => {});
    expect(result.output).toContain('build');
    expect(result.output).toContain('scaffold');
    expect(result.output).toContain('validate');
    expect(result.output).toContain('orchestrate');
  });
});
