/**
 * MCP Primary Adapter
 *
 * Exposes hex-intf capabilities as MCP (Model Context Protocol) tools
 * so LLM agents can call them directly. This is a driving/primary adapter —
 * same use cases as the CLI, different interface.
 *
 * Each MCP tool maps 1:1 to a use case method behind a port interface.
 */

import type { IArchAnalysisPort, IASTPort, IFileSystemPort, ASTSummary } from '../../core/ports/index.js';

// ─── MCP Tool Definitions ────────────────────────────────

export interface MCPToolDefinition {
  name: string;
  description: string;
  inputSchema: {
    type: 'object';
    properties: Record<string, { type: string; description: string; enum?: string[] }>;
    required: string[];
  };
}

export interface MCPToolCall {
  name: string;
  arguments: Record<string, unknown>;
}

export interface MCPToolResult {
  content: Array<{ type: 'text'; text: string }>;
  isError?: boolean;
}

// ─── Tool Registry ───────────────────────────────────────

export const HEX_INTF_TOOLS: MCPToolDefinition[] = [
  {
    name: 'hex_analyze',
    description: 'Analyze hexagonal architecture health: dead code, boundary violations, circular deps',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Project root path to analyze' },
      },
      required: ['path'],
    },
  },
  {
    name: 'hex_summarize',
    description: 'Extract token-efficient AST summary of a file at L0-L3 detail level',
    inputSchema: {
      type: 'object',
      properties: {
        filePath: { type: 'string', description: 'File to summarize' },
        level: { type: 'string', description: 'Summary detail level', enum: ['L0', 'L1', 'L2', 'L3'] },
      },
      required: ['filePath'],
    },
  },
  {
    name: 'hex_summarize_project',
    description: 'Get L1 summaries of all source files in a project for context loading',
    inputSchema: {
      type: 'object',
      properties: {
        rootPath: { type: 'string', description: 'Project root path' },
        level: { type: 'string', description: 'Summary level for all files', enum: ['L0', 'L1', 'L2'] },
      },
      required: ['rootPath'],
    },
  },
  {
    name: 'hex_validate_boundaries',
    description: 'Check if imports respect hexagonal architecture layer rules',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Project root path' },
      },
      required: ['path'],
    },
  },
  {
    name: 'hex_dead_exports',
    description: 'Find exported symbols that no other file imports',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Project root path' },
      },
      required: ['path'],
    },
  },
  {
    name: 'hex_scaffold',
    description: 'Generate hexagonal architecture directory structure for a new project',
    inputSchema: {
      type: 'object',
      properties: {
        language: { type: 'string', description: 'Target language', enum: ['typescript', 'go', 'rust'] },
        name: { type: 'string', description: 'Project name' },
      },
      required: ['name'],
    },
  },
];

// ─── MCP Adapter ─────────────────────────────────────────

export interface MCPContext {
  archAnalyzer: IArchAnalysisPort;
  ast: IASTPort;
  fs: IFileSystemPort;
}

export class MCPAdapter {
  constructor(private readonly ctx: MCPContext) {}

  getTools(): MCPToolDefinition[] {
    return HEX_INTF_TOOLS;
  }

  async handleToolCall(call: MCPToolCall): Promise<MCPToolResult> {
    try {
      switch (call.name) {
        case 'hex_analyze':
          return await this.analyze(call.arguments.path as string);
        case 'hex_summarize':
          return await this.summarize(
            call.arguments.filePath as string,
            (call.arguments.level as ASTSummary['level']) ?? 'L1',
          );
        case 'hex_summarize_project':
          return await this.summarizeProject(
            call.arguments.rootPath as string,
            (call.arguments.level as ASTSummary['level']) ?? 'L1',
          );
        case 'hex_validate_boundaries':
          return await this.validateBoundaries(call.arguments.path as string);
        case 'hex_dead_exports':
          return await this.deadExports(call.arguments.path as string);
        case 'hex_scaffold':
          return this.scaffold(
            call.arguments.name as string,
            (call.arguments.language as string) ?? 'typescript',
          );
        default:
          return { content: [{ type: 'text', text: `Unknown tool: ${call.name}` }], isError: true };
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      return { content: [{ type: 'text', text: `Error: ${msg}` }], isError: true };
    }
  }

  // ─── Tool Implementations ──────────────────────────────

  private async analyze(path: string): Promise<MCPToolResult> {
    const result = await this.ctx.archAnalyzer.analyzeArchitecture(path);
    const s = result.summary;
    const lines = [
      `Health: ${s.healthScore}/100`,
      `Files: ${s.totalFiles} | Exports: ${s.totalExports}`,
      `Dead exports: ${s.deadExportCount} | Violations: ${s.violationCount} | Circular: ${s.circularCount}`,
    ];
    if (result.dependencyViolations.length > 0) {
      lines.push('', 'Violations:');
      for (const v of result.dependencyViolations.slice(0, 5)) {
        lines.push(`  ${v.from} -> ${v.to}: ${v.rule}`);
      }
    }
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private async summarize(filePath: string, level: ASTSummary['level']): Promise<MCPToolResult> {
    const summary = await this.ctx.ast.extractSummary(filePath, level);
    const lines = [`FILE: ${summary.filePath}`, `LANG: ${summary.language}`, `LINES: ${summary.lineCount}`, `TOKENS: ~${summary.tokenEstimate}`];
    if (summary.exports.length > 0) {
      lines.push('EXPORTS:');
      for (const e of summary.exports) {
        lines.push(`  ${e.kind} ${e.name}${e.signature ? `: ${e.signature}` : ''}`);
      }
    }
    if (summary.imports.length > 0) {
      lines.push('IMPORTS:');
      for (const i of summary.imports) {
        lines.push(`  [${i.names.join(', ')}] from ${i.from}`);
      }
    }
    if (summary.raw) lines.push('', summary.raw);
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private async summarizeProject(rootPath: string, level: ASTSummary['level']): Promise<MCPToolResult> {
    const files = await this.ctx.fs.glob(`${rootPath}/src/**/*.ts`);
    const summaries: string[] = [];
    for (const f of files) {
      const s = await this.ctx.ast.extractSummary(f, level);
      const exports = s.exports.map((e) => `${e.kind} ${e.name}`).join(', ');
      summaries.push(`${s.filePath} (${s.lineCount}L, ~${s.tokenEstimate}tok) → ${exports || 'no exports'}`);
    }
    return { content: [{ type: 'text', text: summaries.join('\n') }] };
  }

  private async validateBoundaries(path: string): Promise<MCPToolResult> {
    const violations = await this.ctx.archAnalyzer.validateHexBoundaries(path);
    if (violations.length === 0) {
      return { content: [{ type: 'text', text: 'All hexagonal boundary rules respected.' }] };
    }
    const lines = [`${violations.length} violations found:`, ''];
    for (const v of violations) {
      lines.push(`${v.fromLayer} -> ${v.toLayer}: ${v.from} imports ${v.to}`);
      lines.push(`  Rule: ${v.rule}`);
    }
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private async deadExports(path: string): Promise<MCPToolResult> {
    const dead = await this.ctx.archAnalyzer.findDeadExports(path);
    if (dead.length === 0) {
      return { content: [{ type: 'text', text: 'No dead exports found.' }] };
    }
    const lines = [`${dead.length} dead exports:`, ''];
    for (const d of dead) {
      lines.push(`  ${d.filePath}: ${d.exportName} (${d.kind})`);
    }
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private scaffold(name: string, language: string): MCPToolResult {
    const dirs = [
      `${name}/src/core/domain`,
      `${name}/src/core/ports`,
      `${name}/src/core/usecases`,
      `${name}/src/adapters/primary`,
      `${name}/src/adapters/secondary`,
      `${name}/src/infrastructure`,
      `${name}/tests/unit`,
      `${name}/tests/integration`,
      `${name}/config`,
      `${name}/skills`,
      `${name}/agents`,
    ];
    return {
      content: [{
        type: 'text',
        text: `Scaffold for "${name}" (${language}):\n\nDirectories:\n${dirs.map((d) => `  mkdir -p ${d}`).join('\n')}\n\nNext: create port interfaces in src/core/ports/`,
      }],
    };
  }
}
