import { describe, it, expect } from 'bun:test';
import { runCLI } from '../../src/adapters/primary/cli-adapter.js';
import type { AppContext } from '../../src/core/ports/app-context.js';
import type { ISecretsPort, SecretMetadata } from '../../src/core/ports/secrets.js';
import type {
  IArchAnalysisPort,
  IASTPort,
  IFileSystemPort,
  ArchAnalysisResult,
} from '../../src/core/ports/index.js';

// ─── Mock Helpers ────────────────────────────────────────

function mockArchResult(): ArchAnalysisResult {
  return {
    deadExports: [], orphanFiles: [], dependencyViolations: [],
    circularDeps: [], unusedPorts: [], unusedAdapters: [],
    summary: {
      totalFiles: 0, totalExports: 0, deadExportCount: 0,
      violationCount: 0, circularCount: 0, healthScore: 100,
    },
  };
}

/** EnvSecretsAdapter-like mock: returns [] for listSecrets, true for hasSecret('PATH') */
function envSecretsMock(): ISecretsPort {
  return {
    resolveSecret: async () => ({ ok: false, error: 'not set' }),
    hasSecret: async (key: string) => key === 'PATH',
    listSecrets: async () => [],
  };
}

/** Infisical-like mock: returns metadata for listSecrets */
function infisicalSecretsMock(secrets: SecretMetadata[]): ISecretsPort {
  return {
    resolveSecret: async () => ({ ok: false, error: 'use listSecrets' }),
    hasSecret: async () => false,
    listSecrets: async () => secrets,
  };
}

const SAMPLE_SECRETS: SecretMetadata[] = [
  { key: 'ANTHROPIC_API_KEY', environment: 'dev', version: 3, createdAt: '2025-01-01', updatedAt: '2025-03-10' },
  { key: 'DATABASE_URL', environment: 'dev', version: 1, createdAt: '2025-02-01', updatedAt: '2025-02-01' },
];

/** Known secret values that must NEVER appear in CLI output */
const KNOWN_SECRET_VALUES = [
  'sk-ant-1234567890abcdef',
  'postgres://user:pass@localhost/db',
];

function makeCtx(secrets: ISecretsPort): AppContext {
  const archAnalyzer: IArchAnalysisPort = {
    buildDependencyGraph: async () => [],
    findDeadExports: async () => [],
    validateHexBoundaries: async () => [],
    detectCircularDeps: async () => [],
    analyzeArchitecture: async () => mockArchResult(),
  };
  const ast: IASTPort = {
    extractSummary: async (fp, lvl) => ({
      filePath: fp, language: 'typescript', level: lvl,
      exports: [], imports: [], dependencies: [], lineCount: 0, tokenEstimate: 0,
    }),
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
  };
  const fs: IFileSystemPort = {
    read: async () => '', write: async () => {},
    exists: async () => false, glob: async () => [],
  };

  return {
    archAnalyzer, ast, fs, rootPath: '/test',
    astIsStub: false, codeGenerator: null, workplanExecutor: null,
    summaryService: { summarizeFile: async () => ast.extractSummary('', 'L1'), summarizeProject: async () => [] },
    secrets,
  } as unknown as AppContext;
}

// ─── Tests ───────────────────────────────────────────────

describe('CLI secrets command', () => {
  it('secrets status outputs "Backend:" line for env backend', async () => {
    const ctx = makeCtx(envSecretsMock());
    const result = await runCLI(['secrets', 'status'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Backend:');
    expect(result.output).toContain('Environment variables');
  });

  it('secrets status outputs "Backend:" for infisical backend', async () => {
    const ctx = makeCtx(infisicalSecretsMock(SAMPLE_SECRETS));
    const result = await runCLI(['secrets', 'status'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Backend:');
    expect(result.output).toContain('Infisical');
    expect(result.output).toContain('2 keys accessible');
  });

  it('secrets list with EnvSecretsAdapter shows "requires Infisical" message', async () => {
    const ctx = makeCtx(envSecretsMock());
    const result = await runCLI(['secrets', 'list'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('requires Infisical or local vault backend');
  });

  it('secrets list --json outputs valid JSON array', async () => {
    const ctx = makeCtx(infisicalSecretsMock(SAMPLE_SECRETS));
    const result = await runCLI(['secrets', 'list', '--json'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    const parsed = JSON.parse(result.output);
    expect(Array.isArray(parsed)).toBe(true);
    expect(parsed).toHaveLength(2);
    expect(parsed[0].key).toBe('ANTHROPIC_API_KEY');
  });

  it('secrets list --json for env backend outputs empty JSON array', async () => {
    const ctx = makeCtx(envSecretsMock());
    const result = await runCLI(['secrets', 'list', '--json'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    const parsed = JSON.parse(result.output);
    expect(Array.isArray(parsed)).toBe(true);
    expect(parsed).toHaveLength(0);
  });

  it('secrets list output NEVER contains secret values (S15 security)', async () => {
    // Create a secrets mock that has metadata but resolveSecret returns known values
    const secretsWithValues: ISecretsPort = {
      resolveSecret: async (key: string) => {
        const valueMap: Record<string, string> = {
          ANTHROPIC_API_KEY: KNOWN_SECRET_VALUES[0],
          DATABASE_URL: KNOWN_SECRET_VALUES[1],
        };
        const v = valueMap[key];
        return v ? { ok: true, value: v } : { ok: false, error: 'not found' };
      },
      hasSecret: async () => true,
      listSecrets: async () => SAMPLE_SECRETS,
    };

    const ctx = makeCtx(secretsWithValues);

    // Test both plain and JSON output modes
    const plain = await runCLI(['secrets', 'list'], ctx, () => {});
    const json = await runCLI(['secrets', 'list', '--json'], ctx, () => {});

    for (const secretValue of KNOWN_SECRET_VALUES) {
      expect(plain.output).not.toContain(secretValue);
      expect(json.output).not.toContain(secretValue);
    }

    // Also verify no "value" field appears in JSON output
    const parsed = JSON.parse(json.output);
    for (const entry of parsed) {
      expect(entry).not.toHaveProperty('value');
    }
  });

  it('secrets with unknown subcommand returns exit code 1', async () => {
    const ctx = makeCtx(envSecretsMock());
    const result = await runCLI(['secrets', 'nope'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Unknown secrets command');
  });

  it('secrets with no subcommand defaults to status', async () => {
    const ctx = makeCtx(envSecretsMock());
    const result = await runCLI(['secrets'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Backend:');
  });
});
