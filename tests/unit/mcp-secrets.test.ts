import { describe, it, expect } from 'bun:test';
import { MCPAdapter, HEX_TOOLS } from '../../src/adapters/primary/mcp-adapter.js';
import type { MCPContext } from '../../src/adapters/primary/mcp-adapter.js';
import type { ISecretsPort, SecretMetadata, SecretResult } from '../../src/core/ports/secrets.js';
import type { SecretContext } from '../../src/core/domain/secrets-types.js';
import type { IArchAnalysisPort, IASTPort, IFileSystemPort } from '../../src/core/ports/index.js';

// ── Mock Helpers ────────────────────────────────────────

function stubArch(): IArchAnalysisPort {
  return {
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
}

function stubAST(): IASTPort {
  return {
    extractSummary: async (fp, lvl) => ({
      filePath: fp, language: 'typescript', level: lvl,
      exports: [], imports: [], dependencies: [], lineCount: 0, tokenEstimate: 0,
    }),
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
  };
}

function stubFS(): IFileSystemPort {
  return {
    read: async () => '', write: async () => {},
    exists: async () => false, glob: async () => [],
    mtime: async () => 0,
  };
}

const SAMPLE_SECRETS: SecretMetadata[] = [
  { key: 'ANTHROPIC_API_KEY', environment: 'dev', version: 3, createdAt: '2025-01-01', updatedAt: '2025-03-10' },
  { key: 'DATABASE_URL', environment: 'dev', version: 1, createdAt: '2025-02-01', updatedAt: '2025-02-01' },
  { key: 'JWT_SECRET', environment: 'prod', version: 2, createdAt: '2025-01-15', updatedAt: '2025-03-01' },
];

const SECRET_VALUES: Record<string, string> = {
  ANTHROPIC_API_KEY: 'sk-ant-api03-very-long-secret-key-here',
  DATABASE_URL: 'postgres://admin:super_secret_password@db.example.com:5432/myapp',
  JWT_SECRET: 'jwt-hmac-sha256-secret-key-do-not-share',
};

function fullSecretsMock(): ISecretsPort {
  return {
    async resolveSecret(key: string): Promise<SecretResult> {
      const val = SECRET_VALUES[key];
      return val ? { ok: true, value: val } : { ok: false, error: `Secret "${key}" not found` };
    },
    async hasSecret(key: string): Promise<boolean> {
      return key in SECRET_VALUES;
    },
    async listSecrets(): Promise<SecretMetadata[]> {
      return SAMPLE_SECRETS;
    },
  };
}

function emptySecretsMock(): ISecretsPort {
  return {
    async resolveSecret(): Promise<SecretResult> {
      return { ok: false, error: 'not configured' };
    },
    async hasSecret(): Promise<boolean> {
      return false;
    },
    async listSecrets(): Promise<SecretMetadata[]> {
      return [];
    },
  };
}

function throwingSecretsMock(): ISecretsPort {
  return {
    async resolveSecret(): Promise<SecretResult> {
      throw new Error('connection refused');
    },
    async hasSecret(): Promise<boolean> {
      throw new Error('connection refused');
    },
    async listSecrets(): Promise<SecretMetadata[]> {
      throw new Error('connection refused');
    },
  };
}

function makeMCPContext(secrets?: ISecretsPort | null): MCPContext {
  return {
    archAnalyzer: stubArch(),
    ast: stubAST(),
    fs: stubFS(),
    secrets: secrets ?? null,
  };
}

// ── Tool Definition Tests ───────────────────────────────

describe('MCP Secrets Tool Definitions', () => {
  it('hex_secrets_status tool is registered', () => {
    const tool = HEX_TOOLS.find((t) => t.name === 'hex_secrets_status');
    expect(tool).toBeDefined();
    expect(tool!.description).toContain('secrets');
    expect(tool!.inputSchema.required).toEqual([]);
  });

  it('hex_secrets_has tool requires key parameter', () => {
    const tool = HEX_TOOLS.find((t) => t.name === 'hex_secrets_has');
    expect(tool).toBeDefined();
    expect(tool!.inputSchema.required).toContain('key');
    expect(tool!.inputSchema.properties['key']).toBeDefined();
  });

  it('hex_secrets_resolve tool requires key parameter', () => {
    const tool = HEX_TOOLS.find((t) => t.name === 'hex_secrets_resolve');
    expect(tool).toBeDefined();
    expect(tool!.inputSchema.required).toContain('key');
  });

  it('MCPAdapter.getTools() includes secrets tools', () => {
    const adapter = new MCPAdapter(makeMCPContext());
    const tools = adapter.getTools();
    const secretsTools = tools.filter((t) => t.name.startsWith('hex_secrets'));
    expect(secretsTools).toHaveLength(3);
  });
});

// ── hex_secrets_status Tests ────────────────────────────

describe('hex_secrets_status', () => {
  it('returns error when secrets backend is null', async () => {
    const adapter = new MCPAdapter(makeMCPContext(null));
    const result = await adapter.handleToolCall({ name: 'hex_secrets_status', arguments: {} });
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toContain('not configured');
  });

  it('lists available keys with metadata', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({ name: 'hex_secrets_status', arguments: {} });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('3 key(s) available');
    expect(result.content[0].text).toContain('ANTHROPIC_API_KEY');
    expect(result.content[0].text).toContain('DATABASE_URL');
    expect(result.content[0].text).toContain('JWT_SECRET');
  });

  it('shows zero keys for empty backend', async () => {
    const adapter = new MCPAdapter(makeMCPContext(emptySecretsMock()));
    const result = await adapter.handleToolCall({ name: 'hex_secrets_status', arguments: {} });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('0 key(s) available');
  });

  it('NEVER exposes secret values in status output', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({ name: 'hex_secrets_status', arguments: {} });
    const text = result.content[0].text;
    for (const val of Object.values(SECRET_VALUES)) {
      expect(text).not.toContain(val);
    }
  });

  it('handles backend error gracefully', async () => {
    const adapter = new MCPAdapter(makeMCPContext(throwingSecretsMock()));
    const result = await adapter.handleToolCall({ name: 'hex_secrets_status', arguments: {} });
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toContain('connection refused');
  });
});

// ── hex_secrets_has Tests ───────────────────────────────

describe('hex_secrets_has', () => {
  it('returns true for existing secret', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_has',
      arguments: { key: 'ANTHROPIC_API_KEY' },
    });
    expect(result.content[0].text).toContain('exists');
  });

  it('returns false for missing secret', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_has',
      arguments: { key: 'NONEXISTENT_KEY' },
    });
    expect(result.content[0].text).toContain('not found');
  });

  it('returns error when secrets backend is null', async () => {
    const adapter = new MCPAdapter(makeMCPContext(null));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_has',
      arguments: { key: 'ANY_KEY' },
    });
    expect(result.isError).toBe(true);
  });

  it('does NOT expose the secret value', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_has',
      arguments: { key: 'ANTHROPIC_API_KEY' },
    });
    expect(result.content[0].text).not.toContain(SECRET_VALUES['ANTHROPIC_API_KEY']);
  });
});

// ── hex_secrets_resolve Tests ───────────────────────────

describe('hex_secrets_resolve', () => {
  it('resolves existing secret with masked value', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_resolve',
      arguments: { key: 'ANTHROPIC_API_KEY' },
    });
    expect(result.content[0].text).toContain('resolved');
    expect(result.content[0].text).toContain('masked');
    // Should contain asterisks (masking)
    expect(result.content[0].text).toContain('*');
  });

  it('masks middle portion — shows only first/last 4 chars', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_resolve',
      arguments: { key: 'ANTHROPIC_API_KEY' },
    });
    const text = result.content[0].text;
    const value = SECRET_VALUES['ANTHROPIC_API_KEY'];
    // First 4 chars should be visible
    expect(text).toContain(value.slice(0, 4));
    // Last 4 chars should be visible
    expect(text).toContain(value.slice(-4));
    // Full value must NOT be present
    expect(text).not.toContain(value);
  });

  it('returns error for missing secret', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_resolve',
      arguments: { key: 'DOES_NOT_EXIST' },
    });
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toContain('not found');
  });

  it('returns error when backend is null', async () => {
    const adapter = new MCPAdapter(makeMCPContext(null));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_resolve',
      arguments: { key: 'ANY' },
    });
    expect(result.isError).toBe(true);
  });

  it('NEVER returns the full unmasked secret value (security)', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    for (const key of Object.keys(SECRET_VALUES)) {
      const result = await adapter.handleToolCall({
        name: 'hex_secrets_resolve',
        arguments: { key },
      });
      expect(result.content[0].text).not.toContain(SECRET_VALUES[key]);
    }
  });

  it('handles short secrets (≤8 chars) with full masking', async () => {
    const shortSecrets: ISecretsPort = {
      async resolveSecret(): Promise<SecretResult> {
        return { ok: true, value: 'abc' };
      },
      async hasSecret() { return true; },
      async listSecrets() { return []; },
    };
    const adapter = new MCPAdapter(makeMCPContext(shortSecrets));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_resolve',
      arguments: { key: 'SHORT' },
    });
    expect(result.content[0].text).toContain('********');
    expect(result.content[0].text).not.toContain('abc');
  });
});

// ── Cross-cutting Tests ─────────────────────────────────

describe('MCP Secrets security invariants', () => {
  it('no secrets tool leaks full values across all operations', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));

    const results = await Promise.all([
      adapter.handleToolCall({ name: 'hex_secrets_status', arguments: {} }),
      adapter.handleToolCall({ name: 'hex_secrets_has', arguments: { key: 'DATABASE_URL' } }),
      adapter.handleToolCall({ name: 'hex_secrets_resolve', arguments: { key: 'DATABASE_URL' } }),
    ]);

    const allText = results.map((r) => r.content[0].text).join('\n');
    for (const val of Object.values(SECRET_VALUES)) {
      expect(allText).not.toContain(val);
    }
  });

  it('unknown tool name returns isError', async () => {
    const adapter = new MCPAdapter(makeMCPContext(fullSecretsMock()));
    const result = await adapter.handleToolCall({
      name: 'hex_secrets_delete',
      arguments: { key: 'X' },
    });
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toContain('Unknown tool');
  });
});
