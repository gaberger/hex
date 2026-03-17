import { describe, it, expect, afterEach } from 'bun:test';
import { existsSync, mkdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { randomBytes } from 'node:crypto';
import { runCLI } from '../../src/adapters/primary/cli-adapter.js';
import type { AppContext } from '../../src/core/ports/app-context.js';
import type { ISecretsPort } from '../../src/core/ports/secrets.js';
import type { IArchAnalysisPort, IASTPort, IFileSystemPort } from '../../src/core/ports/index.js';
import { LocalVaultAdapter } from '../../src/adapters/secondary/local-vault-adapter.js';

// ── Helpers ─────────────────────────────────────────────

function tempDir(): string {
  const dir = join(tmpdir(), `hex-vault-cli-test-${randomBytes(6).toString('hex')}`);
  mkdirSync(join(dir, '.hex'), { recursive: true });
  return dir;
}

function makeCtx(rootPath: string, secrets: ISecretsPort): AppContext {
  const archAnalyzer: IArchAnalysisPort = {
    buildDependencyGraph: async () => [],
    findDeadExports: async () => [],
    validateHexBoundaries: async () => [],
    detectCircularDeps: async () => [],
    analyzeArchitecture: async () => ({
      deadExports: [], orphanFiles: [], dependencyViolations: [],
      circularDeps: [], unusedPorts: [], unusedAdapters: [],
      summary: { totalFiles: 0, totalExports: 0, deadExportCount: 0, violationCount: 0, circularCount: 0, healthScore: 100 },
    }),
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
    mtime: async () => 0,
  };

  return {
    rootPath,
    archAnalyzer, ast, fs,
    astIsStub: false, codeGenerator: null, workplanExecutor: null,
    summaryService: { summarizeFile: async () => ast.extractSummary('', 'L1'), summarizeProject: async () => [] },
    outputDir: join(rootPath, '.hex'),
    secrets,
    autoConfirm: false,
  } as unknown as AppContext;
}

function envSecretsMock(): ISecretsPort {
  return {
    resolveSecret: async () => ({ ok: false, error: 'not set' }),
    hasSecret: async (key: string) => key === 'PATH',
    listSecrets: async () => [],
  };
}

// ── Tests ───────────────────────────────────────────────

const dirs: string[] = [];

afterEach(() => {
  for (const d of dirs) {
    rmSync(d, { recursive: true, force: true });
  }
  dirs.length = 0;
});

describe('CLI secrets init', () => {
  it('creates vault and config files', async () => {
    const dir = tempDir(); dirs.push(dir);
    const ctx = makeCtx(dir, envSecretsMock());

    const result = await runCLI(['secrets', 'init', '--password', 'test-pw'], ctx, () => {});

    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('Vault created');
    expect(existsSync(join(dir, '.hex/vault.enc'))).toBe(true);
    expect(existsSync(join(dir, '.hex/secrets.json'))).toBe(true);
  });

  it('refuses to overwrite existing vault', async () => {
    const dir = tempDir(); dirs.push(dir);
    const vaultPath = join(dir, '.hex/vault.enc');
    LocalVaultAdapter.createVault(vaultPath, 'pw');

    const ctx = makeCtx(dir, envSecretsMock());
    const result = await runCLI(['secrets', 'init', '--password', 'pw'], ctx, () => {});

    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('already exists');
  });

  it('requires a password', async () => {
    const dir = tempDir(); dirs.push(dir);
    const oldPw = process.env['HEX_VAULT_PASSWORD'];
    delete process.env['HEX_VAULT_PASSWORD'];

    const ctx = makeCtx(dir, envSecretsMock());
    const result = await runCLI(['secrets', 'init'], ctx, () => {});

    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('password required');

    if (oldPw) process.env['HEX_VAULT_PASSWORD'] = oldPw;
  });
});

describe('CLI secrets set/get/remove', () => {
  function setupVault(dir: string): ISecretsPort {
    const vaultPath = join(dir, '.hex/vault.enc');
    const { writeFileSync } = require('node:fs');
    LocalVaultAdapter.createVault(vaultPath, 'test-pw');
    writeFileSync(
      join(dir, '.hex/secrets.json'),
      JSON.stringify({ backend: 'local-vault', localVault: { path: vaultPath } }),
    );
    process.env['HEX_VAULT_PASSWORD'] = 'test-pw';
    return new LocalVaultAdapter(vaultPath, 'test-pw');
  }

  it('set adds a secret to the vault', async () => {
    const dir = tempDir(); dirs.push(dir);
    const secrets = setupVault(dir);
    const ctx = makeCtx(dir, secrets);

    const result = await runCLI(['secrets', 'set', 'API_KEY', 'sk-123'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('saved');

    // Verify it's actually in the vault
    const resolved = await secrets.resolveSecret('API_KEY');
    expect(resolved).toEqual({ ok: true, value: 'sk-123' });
  });

  it('get retrieves a secret value', async () => {
    const dir = tempDir(); dirs.push(dir);
    const secrets = setupVault(dir);
    const adapter = new LocalVaultAdapter(join(dir, '.hex/vault.enc'), 'test-pw');
    adapter.addSecret('MY_KEY', 'my-value');

    // Re-create secrets port to pick up the new secret
    const freshSecrets = new LocalVaultAdapter(join(dir, '.hex/vault.enc'), 'test-pw');
    const ctx = makeCtx(dir, freshSecrets);

    const result = await runCLI(['secrets', 'get', 'MY_KEY'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('my-value');
  });

  it('get returns error for missing key', async () => {
    const dir = tempDir(); dirs.push(dir);
    const secrets = setupVault(dir);
    const ctx = makeCtx(dir, secrets);

    const result = await runCLI(['secrets', 'get', 'NOPE'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('not found');
  });

  it('remove deletes a secret', async () => {
    const dir = tempDir(); dirs.push(dir);
    const secrets = setupVault(dir);
    const adapter = new LocalVaultAdapter(join(dir, '.hex/vault.enc'), 'test-pw');
    adapter.addSecret('DEL_ME', 'gone');

    const freshSecrets = new LocalVaultAdapter(join(dir, '.hex/vault.enc'), 'test-pw');
    const ctx = makeCtx(dir, freshSecrets);

    const result = await runCLI(['secrets', 'remove', 'DEL_ME'], ctx, () => {});
    expect(result.exitCode).toBe(0);
    expect(result.output).toContain('removed');

    // Verify it's gone
    const check = new LocalVaultAdapter(join(dir, '.hex/vault.enc'), 'test-pw');
    expect(await check.hasSecret('DEL_ME')).toBe(false);
  });

  it('remove returns error for missing key', async () => {
    const dir = tempDir(); dirs.push(dir);
    const secrets = setupVault(dir);
    const ctx = makeCtx(dir, secrets);

    const result = await runCLI(['secrets', 'remove', 'NOPE'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('not found');
  });

  it('set requires key and value args', async () => {
    const dir = tempDir(); dirs.push(dir);
    const secrets = setupVault(dir);
    const ctx = makeCtx(dir, secrets);

    const result = await runCLI(['secrets', 'set'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('Usage');
  });
});

describe('CLI secrets unknown command shows help', () => {
  it('shows available commands', async () => {
    const dir = tempDir(); dirs.push(dir);
    const ctx = makeCtx(dir, envSecretsMock());

    const result = await runCLI(['secrets', 'bogus'], ctx, () => {});
    expect(result.exitCode).toBe(1);
    expect(result.output).toContain('init');
    expect(result.output).toContain('set');
    expect(result.output).toContain('get');
    expect(result.output).toContain('remove');
  });
});
