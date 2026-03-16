/**
 * MCP Adapter — Unit Tests
 *
 * Tests the MCP adapter's tool registry, routing, and error handling.
 * Uses mock ports (London-school TDD) — no real file system or tree-sitter.
 */

import { describe, it, expect, beforeEach } from 'bun:test';
import { MCPAdapter, HEX_TOOLS, HEX_DASHBOARD_TOOLS } from '../../src/adapters/primary/mcp-adapter.js';
import type { MCPContext, MCPToolCall } from '../../src/adapters/primary/mcp-adapter.js';
import type { IArchAnalysisPort, IASTPort, IFileSystemPort } from '../../src/core/ports/index.js';

// ── Mock Ports ────────────────────────────────────────────

function mockAST(): IASTPort {
  return {
    async extractSummary(filePath, level) {
      return {
        filePath,
        language: 'typescript',
        level: level ?? 'L1',
        exports: [{ kind: 'function', name: 'hello' }],
        imports: [{ from: './utils.js', names: ['greet'] }],
        dependencies: [],
        lineCount: 42,
        tokenEstimate: 128,
      };
    },
    diffStructural() {
      return { added: [], removed: [], modified: [] };
    },
  };
}

function mockFS(): IFileSystemPort {
  return {
    async readFile() { return ''; },
    async writeFile() {},
    async exists() { return true; },
    async glob() { return ['src/index.ts', 'src/core/domain/entities.ts']; },
    async mkdir() {},
    async stat() { return { isFile: true, isDirectory: false, size: 100 }; },
  } as unknown as IFileSystemPort;
}

function mockArchAnalyzer(): IArchAnalysisPort {
  return {
    async analyzeArchitecture() {
      return {
        summary: {
          healthScore: 92,
          totalFiles: 10,
          totalExports: 25,
          deadExportCount: 1,
          violationCount: 0,
          circularCount: 0,
        },
        deadExports: [{ filePath: 'src/old.ts', exportName: 'unused', kind: 'function' }],
        dependencyViolations: [],
        circularDeps: [],
        orphanFiles: [],
        unusedPorts: [],
        unusedAdapters: [],
      };
    },
    async validateHexBoundaries() { return []; },
    async findDeadExports() { return []; },
    async buildDependencyGraph() { return []; },
  } as unknown as IArchAnalysisPort;
}

function createCtx(): MCPContext {
  return {
    archAnalyzer: mockArchAnalyzer(),
    ast: mockAST(),
    fs: mockFS(),
  };
}

// ── Tests ─────────────────────────────────────────────────

describe('MCPAdapter', () => {
  let adapter: MCPAdapter;

  beforeEach(() => {
    adapter = new MCPAdapter(createCtx());
  });

  // ── Tool Registry ──

  it('returns all analysis + dashboard tools from getTools()', () => {
    const tools = adapter.getTools();
    const expectedCount = HEX_TOOLS.length + HEX_DASHBOARD_TOOLS.length;
    expect(tools.length).toBe(expectedCount);
  });

  it('every tool has name, description, and inputSchema', () => {
    for (const tool of adapter.getTools()) {
      expect(tool.name).toBeTruthy();
      expect(tool.description).toBeTruthy();
      expect(tool.inputSchema.type).toBe('object');
      expect(tool.inputSchema.properties).toBeDefined();
      expect(Array.isArray(tool.inputSchema.required)).toBe(true);
    }
  });

  it('tool names are unique', () => {
    const names = adapter.getTools().map((t) => t.name);
    expect(new Set(names).size).toBe(names.length);
  });

  it('all tool names use snake_case with hex_ prefix', () => {
    for (const tool of adapter.getTools()) {
      expect(tool.name).toMatch(/^hex_[a-z_]+$/);
    }
  });

  // ── Analysis Tool Routing ──

  it('hex_analyze returns health score', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_analyze',
      arguments: { path: '.' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('Score:    92/100');
  });

  it('hex_summarize returns file summary', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_summarize',
      arguments: { filePath: 'src/index.ts', level: 'L1' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('FILE: src/index.ts');
    expect(result.content[0].text).toContain('TOKENS: ~128');
  });

  it('hex_summarize defaults to L1 when level omitted', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_summarize',
      arguments: { filePath: 'src/index.ts' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('FILE: src/index.ts');
  });

  it('hex_validate_boundaries returns clean result', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_validate_boundaries',
      arguments: { path: '.' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('All hexagonal boundary rules respected');
  });

  it('hex_scaffold returns directory listing', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_scaffold',
      arguments: { name: 'my-app' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('my-app/src/core/domain');
    expect(result.content[0].text).toContain('my-app/src/adapters/primary');
  });

  // ── Error Handling ──

  it('unknown tool returns isError', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_nonexistent',
      arguments: {},
    });
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toContain('Unknown tool');
  });

  it('tool errors are caught and returned as isError', async () => {
    const ctx = createCtx();
    ctx.archAnalyzer = {
      ...ctx.archAnalyzer,
      async analyzeArchitecture() { throw new Error('tree-sitter crashed'); },
    } as unknown as IArchAnalysisPort;
    const adapter2 = new MCPAdapter(ctx);
    const result = await adapter2.handleToolCall({
      name: 'hex_analyze',
      arguments: { path: '.' },
    });
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toContain('tree-sitter crashed');
  });

  // ── Dashboard Hub Tools ──
  // These tests verify the adapter returns a meaningful response
  // regardless of whether the hub is running or not.

  it('hex_dashboard_list returns a response', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_dashboard_list',
      arguments: {},
    });
    expect(result.content[0].text.length).toBeGreaterThan(0);
  });

  it('hex_dashboard_register returns a response for a path', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_dashboard_register',
      arguments: { rootPath: '/tmp/test' },
    });
    expect(result.content[0].text.length).toBeGreaterThan(0);
  });

  it('hex_dashboard_unregister handles non-existent project', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_dashboard_unregister',
      arguments: { projectId: 'nonexistent-project-xyz' },
    });
    // Either hub is down ("not running") or project not found
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/not found|not running/i);
  });

  it('hex_dashboard_query handles non-existent project', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_dashboard_query',
      arguments: { projectId: 'nonexistent-project-xyz', query: 'health' },
    });
    // Either hub is down ("not running") or project not found
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toMatch(/not found|not running|failed/i);
  });

  // ── Generate & Plan Tools ──

  it('hex_generate returns structured plan without codeGenerator (Claude IS the LLM)', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_generate',
      arguments: { specContent: 'Build a REST API' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('GENERATE CODE');
    expect(result.content[0].text).toContain('EXECUTE NOW');
  });

  it('hex_generate returns generated code when codeGenerator present', async () => {
    const ctx = createCtx();
    ctx.codeGenerator = {
      async generateFromSpec() {
        return { filePath: 'src/adapters/primary/http-adapter.ts', language: 'typescript', content: 'export class HttpAdapter {}' };
      },
      async refineFromFeedback() { return { filePath: '', language: 'typescript', content: '' }; },
    };
    const adapterWithGen = new MCPAdapter(ctx);
    const result = await adapterWithGen.handleToolCall({
      name: 'hex_generate',
      arguments: { specContent: 'Build a REST API', language: 'typescript' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('HttpAdapter');
  });

  it('hex_generate rejects invalid language', async () => {
    const ctx = createCtx();
    ctx.codeGenerator = { async generateFromSpec() { return {} as any; }, async refineFromFeedback() { return {} as any; } };
    const adapterWithGen = new MCPAdapter(ctx);
    const result = await adapterWithGen.handleToolCall({
      name: 'hex_generate',
      arguments: { specContent: 'Build a thing', language: 'python' },
    });
    expect(result.isError).toBe(true);
    expect(result.content[0].text).toContain('Invalid language');
  });

  it('hex_plan returns structured workplan without workplanExecutor (Claude IS the LLM)', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_plan',
      arguments: { requirements: 'Add user auth' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('WORKPLAN');
    expect(result.content[0].text).toContain('EXECUTE NOW');
  });

  it('hex_plan returns workplan when executor present', async () => {
    const ctx = createCtx();
    ctx.workplanExecutor = {
      async createPlan() {
        return {
          id: 'wp-1', title: 'Auth Plan', estimatedTokenBudget: 5000,
          steps: [{ id: 's1', description: 'Add auth port', adapter: 'ports', dependencies: [] }],
        };
      },
      async *executePlan() {},
    };
    const adapterWithPlan = new MCPAdapter(ctx);
    const result = await adapterWithPlan.handleToolCall({
      name: 'hex_plan',
      arguments: { requirements: 'Add user auth' },
    });
    expect(result.isError).toBeUndefined();
    expect(result.content[0].text).toContain('Auth Plan');
    expect(result.content[0].text).toContain('Add auth port');
  });

  it('hex_analyze_json returns raw JSON', async () => {
    const result = await adapter.handleToolCall({
      name: 'hex_analyze_json',
      arguments: { path: '.' },
    });
    expect(result.isError).toBeUndefined();
    const parsed = JSON.parse(result.content[0].text);
    expect(parsed.summary.healthScore).toBe(92);
  });

  // ── shutdownHub ──

  it('shutdownHub is safe to call when hub is not running', () => {
    expect(() => adapter.shutdownHub()).not.toThrow();
  });
});

// ── Tool Definition Tests ─────────────────────────────────

describe('HEX_DASHBOARD_TOOLS', () => {
  it('defines 5 dashboard tools', () => {
    expect(HEX_DASHBOARD_TOOLS.length).toBe(5);
  });

  it('hex_dashboard_start requires rootPath', () => {
    const tool = HEX_DASHBOARD_TOOLS.find((t) => t.name === 'hex_dashboard_start');
    expect(tool).toBeDefined();
    expect(tool!.inputSchema.required).toContain('rootPath');
  });

  it('hex_dashboard_query requires projectId and query', () => {
    const tool = HEX_DASHBOARD_TOOLS.find((t) => t.name === 'hex_dashboard_query');
    expect(tool).toBeDefined();
    expect(tool!.inputSchema.required).toContain('projectId');
    expect(tool!.inputSchema.required).toContain('query');
  });

  it('hex_dashboard_query restricts query to valid enum values', () => {
    const tool = HEX_DASHBOARD_TOOLS.find((t) => t.name === 'hex_dashboard_query');
    expect(tool!.inputSchema.properties.query.enum).toEqual(['health', 'tokens', 'swarm', 'graph']);
  });

  it('hex_dashboard_list requires no parameters', () => {
    const tool = HEX_DASHBOARD_TOOLS.find((t) => t.name === 'hex_dashboard_list');
    expect(tool!.inputSchema.required).toEqual([]);
  });
});
