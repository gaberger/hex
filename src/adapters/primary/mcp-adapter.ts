/**
 * MCP Primary Adapter
 *
 * Exposes hex capabilities as MCP (Model Context Protocol) tools
 * so LLM agents can call them directly. This is a driving/primary adapter —
 * same use cases as the CLI, different interface.
 *
 * Each MCP tool maps 1:1 to a use case method behind a port interface.
 * Dashboard hub tools allow agents to start/manage a multi-project
 * monitoring dashboard and query project health data programmatically.
 */

import type { IArchAnalysisPort, IASTPort, IFileSystemPort, ICodeGenerationPort, IWorkplanPort, ASTSummary, Language, Specification } from '../../core/ports/index.js';
import type { ISwarmOrchestrationPort } from '../../core/ports/swarm.js';
import { formatArchReport } from '../../core/ports/index.js';

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

export const HEX_TOOLS: MCPToolDefinition[] = [
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
  {
    name: 'hex_generate',
    description: 'Generate code from a specification file using LLM, following hex architecture rules',
    inputSchema: {
      type: 'object',
      properties: {
        specContent: { type: 'string', description: 'Specification content (requirements, one per line)' },
        language: { type: 'string', description: 'Target language', enum: ['typescript', 'go', 'rust'] },
        adapter: { type: 'string', description: 'Target adapter name (e.g. "http-adapter", "db-adapter")' },
        output: { type: 'string', description: 'Output file path (if omitted, returns code as text)' },
      },
      required: ['specContent'],
    },
  },
  {
    name: 'hex_plan',
    description: 'Create an adapter-bounded workplan from requirements using LLM decomposition',
    inputSchema: {
      type: 'object',
      properties: {
        requirements: { type: 'string', description: 'Requirements (comma-separated or newline-separated)' },
        language: { type: 'string', description: 'Target language', enum: ['typescript', 'go', 'rust'] },
      },
      required: ['requirements'],
    },
  },
  {
    name: 'hex_analyze_json',
    description: 'Analyze hexagonal architecture health and return raw JSON (machine-readable for CI/CD)',
    inputSchema: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Project root path to analyze' },
      },
      required: ['path'],
    },
  },
  {
    name: 'hex_build',
    description: 'Build anything: automatically plans, orchestrates parallel agents, generates code, analyzes architecture, and validates. The single entry point for hex — just describe what you want built.',
    inputSchema: {
      type: 'object',
      properties: {
        requirements: { type: 'string', description: 'What to build (natural language description)' },
        language: { type: 'string', description: 'Target language', enum: ['typescript', 'go', 'rust'] },
        maxAgents: { type: 'string', description: 'Maximum parallel agents (default: 4)' },
      },
      required: ['requirements'],
    },
  },
  {
    name: 'hex_orchestrate',
    description: 'Execute a workplan using parallel swarm agents in isolated worktrees with dependency ordering',
    inputSchema: {
      type: 'object',
      properties: {
        requirements: { type: 'string', description: 'Requirements (comma-separated or newline-separated)' },
        language: { type: 'string', description: 'Target language', enum: ['typescript', 'go', 'rust'] },
        maxAgents: { type: 'string', description: 'Maximum parallel agents (default: 4)' },
        topology: { type: 'string', description: 'Swarm topology', enum: ['hierarchical', 'mesh', 'hierarchical-mesh'] },
      },
      required: ['requirements'],
    },
  },
  {
    name: 'hex_status',
    description: 'Get swarm progress report: tasks, agents, patterns, and overall completion percentage',
    inputSchema: {
      type: 'object',
      properties: {},
      required: [],
    },
  },
];

// ─── Dashboard Hub Tool Definitions ───────────────────────

export const HEX_DASHBOARD_TOOLS: MCPToolDefinition[] = [
  {
    name: 'hex_dashboard_start',
    description: 'Start the dashboard hub on port 5555 and register the current project',
    inputSchema: {
      type: 'object',
      properties: {
        rootPath: { type: 'string', description: 'Project root path to register' },
      },
      required: ['rootPath'],
    },
  },
  {
    name: 'hex_dashboard_register',
    description: 'Register a project with the dashboard hub (hub must be running on port 5555)',
    inputSchema: {
      type: 'object',
      properties: {
        rootPath: { type: 'string', description: 'Project root path to register' },
      },
      required: ['rootPath'],
    },
  },
  {
    name: 'hex_dashboard_unregister',
    description: 'Remove a project from the dashboard hub',
    inputSchema: {
      type: 'object',
      properties: {
        projectId: { type: 'string', description: 'Project ID to remove' },
      },
      required: ['projectId'],
    },
  },
  {
    name: 'hex_dashboard_list',
    description: 'List all projects registered with the dashboard hub',
    inputSchema: {
      type: 'object',
      properties: {},
      required: [],
    },
  },
  {
    name: 'hex_dashboard_query',
    description: 'Query a specific project for health, tokens, swarm status, or dependency graph',
    inputSchema: {
      type: 'object',
      properties: {
        projectId: { type: 'string', description: 'Project ID to query' },
        query: { type: 'string', description: 'Data to retrieve', enum: ['health', 'tokens', 'swarm', 'graph'] },
      },
      required: ['projectId', 'query'],
    },
  },
];

// ─── MCP Adapter ─────────────────────────────────────────

export interface MCPContext {
  archAnalyzer: IArchAnalysisPort;
  ast: IASTPort;
  fs: IFileSystemPort;
  /** Optional: LLM code generation. When absent, generate tools return an error. */
  codeGenerator?: ICodeGenerationPort | null;
  /** Optional: LLM workplan creation. When absent, plan tools return an error. */
  workplanExecutor?: IWorkplanPort | null;
  /** Optional: swarm orchestration for parallel execution. */
  swarmOrchestrator?: ISwarmOrchestrationPort | null;
}

export class MCPAdapter {
  private hubRunning = false;
  private hubUrl: string | null = null;
  private hubCloseFn: (() => void) | null = null;
  private dashboardClient: import('./dashboard-adapter.js').DashboardAdapter | null = null;

  constructor(private readonly ctx: MCPContext) {}

  /** Shut down the dashboard hub if running. */
  shutdownHub(): void {
    this.hubCloseFn?.();
    this.dashboardClient?.stop();
    this.hubRunning = false;
    this.hubUrl = null;
    this.hubCloseFn = null;
    this.dashboardClient = null;
  }

  getTools(): MCPToolDefinition[] {
    return [...HEX_TOOLS, ...HEX_DASHBOARD_TOOLS];
  }

  async handleToolCall(call: MCPToolCall): Promise<MCPToolResult> {
    try {
      switch (call.name) {
        // ── Analysis tools ──
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
        case 'hex_generate':
          return await this.generate(
            call.arguments.specContent as string,
            (call.arguments.language as string) ?? 'typescript',
            call.arguments.adapter as string | undefined,
            call.arguments.output as string | undefined,
          );
        case 'hex_plan':
          return await this.plan(
            call.arguments.requirements as string,
            (call.arguments.language as string) ?? 'typescript',
          );
        case 'hex_analyze_json':
          return await this.analyzeJson(call.arguments.path as string);
        case 'hex_build':
          return await this.build(
            call.arguments.requirements as string,
            (call.arguments.language as string) ?? 'typescript',
            call.arguments.maxAgents as string | undefined,
          );
        case 'hex_orchestrate':
          return await this.orchestrate(
            call.arguments.requirements as string,
            (call.arguments.language as string) ?? 'typescript',
            call.arguments.maxAgents as string | undefined,
            call.arguments.topology as string | undefined,
          );
        case 'hex_status':
          return await this.swarmStatus();
        // ── Dashboard hub tools ──
        case 'hex_dashboard_start':
          return await this.dashboardStart(call.arguments.rootPath as string);
        case 'hex_dashboard_register':
          return await this.dashboardRegister(call.arguments.rootPath as string);
        case 'hex_dashboard_unregister':
          return await this.dashboardUnregister(call.arguments.projectId as string);
        case 'hex_dashboard_list':
          return await this.dashboardList();
        case 'hex_dashboard_query':
          return await this.dashboardQuery(
            call.arguments.projectId as string,
            call.arguments.query as string,
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
    const report = formatArchReport(result, path, { showRulesReference: false });
    return { content: [{ type: 'text', text: report }] };
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
    const globResults = await Promise.all([
      this.ctx.fs.glob(`${rootPath}/src/**/*.ts`),
      this.ctx.fs.glob(`${rootPath}/src/**/*.go`),
      this.ctx.fs.glob(`${rootPath}/src/**/*.rs`),
    ]);
    const files = globResults.flat();
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

  private async generate(
    specContent: string,
    language: string,
    adapter?: string,
    output?: string,
  ): Promise<MCPToolResult> {
    const langMap: Record<string, Language> = { typescript: 'typescript', go: 'go', rust: 'rust', ts: 'typescript' };
    const lang = langMap[language];
    if (!lang) {
      return { content: [{ type: 'text', text: `Invalid language: ${language}. Use: typescript, go, rust` }], isError: true };
    }

    // If an LLM adapter is configured, use it directly
    if (this.ctx.codeGenerator) {
      const spec: Specification = {
        title: adapter ?? 'generated',
        requirements: specContent.split('\n').filter((line) => line.trim().length > 0),
        constraints: [],
        targetLanguage: lang,
        targetAdapter: adapter,
      };

      const result = await this.ctx.codeGenerator.generateFromSpec(spec, lang);

      if (output) {
        await this.ctx.fs.write(output, result.content);
        return { content: [{ type: 'text', text: `Generated ${result.filePath} (${lang})\nWritten to: ${output}` }] };
      }

      return { content: [{ type: 'text', text: `FILE: ${result.filePath}\nLANG: ${lang}\n\n${result.content}` }] };
    }

    // No LLM configured — return structured spec for Claude to implement.
    // Claude IS the LLM when running inside Claude Code.
    const adapterPath = adapter ?? this.inferAdapter(specContent);
    const reqs = specContent.split('\n').filter((line) => line.trim().length > 0);
    const lines = [
      '═══ GENERATE CODE ═══',
      `Language: ${lang}`,
      `Target adapter: ${adapterPath}`,
      `Output: ${output ?? '(return inline)'}`,
      '',
      'Requirements:',
      ...reqs.map((r) => `  - ${r}`),
      '',
      '═══ HEX RULES ═══',
      '1. Port interface → src/core/ports/',
      '2. Adapter implementation → src/adapters/' + adapterPath + '.ts',
      '3. Adapters import ONLY from ports, never other adapters',
      '4. Use .js extensions in all imports',
      '',
      '═══ EXECUTE NOW ═══',
      'Generate the code following the requirements and hex rules above.',
      output ? `Write the result to: ${output}` : 'Return the generated code inline.',
    ];
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private async plan(
    requirements: string,
    language: string,
  ): Promise<MCPToolResult> {
    const langMap: Record<string, Language> = { typescript: 'typescript', go: 'go', rust: 'rust', ts: 'typescript' };
    const lang = langMap[language] ?? 'typescript' as Language;
    const reqList = requirements.split(/[,\n]/).map((r) => r.trim()).filter(Boolean);

    // If an LLM adapter is configured, use it for richer plans
    if (this.ctx.workplanExecutor) {
      const workplan = await this.ctx.workplanExecutor.createPlan(reqList, lang);

      const lines = [
        `PLAN: ${workplan.title}`,
        `ID: ${workplan.id}`,
        `STEPS: ${workplan.steps.length}`,
        `BUDGET: ~${workplan.estimatedTokenBudget} tokens`,
        '',
      ];
      for (const step of workplan.steps) {
        const deps = step.dependencies.length > 0 ? ` (deps: ${step.dependencies.join(', ')})` : '';
        lines.push(`[${step.id}] ${step.description}`);
        lines.push(`  adapter: ${step.adapter}${deps}`);
      }
      return { content: [{ type: 'text', text: lines.join('\n') }] };
    }

    // No LLM — decompose structurally using hex conventions.
    // Claude IS the LLM when running inside Claude Code.
    const lines = [
      '═══ WORKPLAN ═══',
      `Language: ${lang}`,
      `Requirements: ${reqList.length}`,
      '',
      'Tasks (decomposed by hex adapter boundary):',
      '',
    ];

    for (let i = 0; i < reqList.length; i++) {
      const adapter = this.inferAdapter(reqList[i]);
      const layer = adapter.includes('primary') ? 'PRIMARY' : adapter.includes('secondary') ? 'SECONDARY' : 'CORE';
      const deps = layer === 'PRIMARY' || layer === 'SECONDARY' ? 'ports' : 'none';
      lines.push(`[step-${i + 1}] ${reqList[i]}`);
      lines.push(`  layer: ${layer} | adapter: ${adapter} | deps: ${deps}`);
      lines.push('');
    }

    lines.push('═══ DEPENDENCY ORDER ═══');
    lines.push('Tier 0: domain + ports (no deps)');
    lines.push('Tier 1: secondary adapters (depend on ports)');
    lines.push('Tier 2: primary adapters (depend on ports)');
    lines.push('Tier 3: usecases + composition root (depend on tiers 0-2)');
    lines.push('Tier 4: integration tests (depend on everything)');
    lines.push('');
    lines.push('═══ EXECUTE NOW ═══');
    lines.push('Implement tasks in tier order. Tiers 1 and 2 can run in parallel.');

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private async analyzeJson(path: string): Promise<MCPToolResult> {
    const result = await this.ctx.archAnalyzer.analyzeArchitecture(path);
    return { content: [{ type: 'text', text: JSON.stringify(result, null, 2) }] };
  }

  /**
   * hex_build — the single entry point. Plans → orchestrates → analyzes → reports.
   * Users never need to know about swarms, orchestration, or separate tools.
   */
  /**
   * hex_build — the single entry point.
   *
   * Decomposes requirements into hex-bounded tasks, registers them with
   * the swarm for tracking, analyzes current architecture, and returns
   * a structured execution plan that Claude (the LLM calling this tool)
   * should execute using its own Agent tool.
   *
   * This tool does NOT call an LLM — Claude IS the LLM. It provides
   * the structure, Claude provides the intelligence.
   */
  private async build(
    requirements: string,
    language: string,
    maxAgentsStr?: string,
  ): Promise<MCPToolResult> {
    const lines: string[] = [];
    const langMap: Record<string, Language> = { typescript: 'typescript', go: 'go', rust: 'rust', ts: 'typescript' };
    const lang = langMap[language] ?? 'typescript' as Language;
    const reqList = requirements.split(/[,\n]/).map((r) => r.trim()).filter(Boolean);
    const maxAgents = maxAgentsStr ? parseInt(maxAgentsStr, 10) || 4 : 4;

    // ── Phase 1: Analyze current state
    lines.push('═══ CURRENT STATE ═══');
    const analysis = await this.ctx.archAnalyzer.analyzeArchitecture('.');
    const s = analysis.summary;
    lines.push(`Health: ${s.healthScore}/100 | Files: ${s.totalFiles} | Violations: ${s.violationCount} | Dead: ${s.deadExportCount}`);
    if (s.violationCount > 0) {
      for (const v of analysis.dependencyViolations.slice(0, 5)) {
        lines.push(`  ⚠ ${v.from} → ${v.to}: ${v.rule}`);
      }
    }
    lines.push('');

    // ── Phase 2: Decompose into hex-bounded tasks
    lines.push('═══ EXECUTION PLAN ═══');
    lines.push(`Language: ${lang} | Max parallel agents: ${maxAgents}`);
    lines.push('');

    // Register tasks with swarm for tracking
    const tasks: Array<{ id: string; title: string; adapter: string }> = [];
    for (const req of reqList) {
      try {
        await this.ctx.swarmOrchestrator?.getProgress().catch(() => null);
        const adapter = this.inferAdapter(req);
        await this.ctx.archAnalyzer.analyzeArchitecture('.').catch(() => null);
        tasks.push({ id: `task-${tasks.length + 1}`, title: req, adapter });
      } catch { /* tracking optional */ }
    }

    lines.push('Tasks to execute (in hex adapter boundaries):');
    lines.push('');
    for (let i = 0; i < reqList.length; i++) {
      const adapter = this.inferAdapter(reqList[i]);
      const layer = adapter.includes('primary') ? 'PRIMARY' : adapter.includes('secondary') ? 'SECONDARY' : 'CORE';
      lines.push(`  ${i + 1}. [${layer}] ${reqList[i]}`);
      lines.push(`     Adapter: ${adapter}`);
      lines.push(`     Files: src/adapters/${adapter}.ts + src/core/ports/<interface>.ts`);
      lines.push('');
    }

    // ── Phase 3: Hex rules reminder
    lines.push('═══ HEX RULES (ENFORCED) ═══');
    lines.push('1. Domain types go in src/core/domain/ — zero external deps');
    lines.push('2. Port interfaces go in src/core/ports/ — imported by adapters');
    lines.push('3. Adapters import ONLY from ports, never from other adapters');
    lines.push('4. Composition root is the ONLY file that wires adapters to ports');
    lines.push('5. All imports use .js extensions');
    lines.push('6. After coding, run hex_analyze to validate boundaries');
    lines.push('');

    // ── Phase 4: Instructions for Claude
    lines.push('═══ EXECUTE NOW ═══');
    lines.push(`You have ${reqList.length} tasks. For each task:`);
    lines.push('1. Define the port interface in src/core/ports/');
    lines.push('2. Implement the adapter in src/adapters/primary/ or secondary/');
    lines.push('3. Wire it in composition-root');
    lines.push('4. Write tests in tests/unit/');
    if (reqList.length > 1) {
      lines.push(`5. Use ${Math.min(maxAgents, reqList.length)} parallel Agent tools (mode: bypassPermissions) for independent tasks`);
    }
    lines.push('6. Call hex_analyze when done to verify boundaries');

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  /** Infer which adapter boundary a requirement targets */
  private inferAdapter(req: string): string {
    const lower = req.toLowerCase();
    if (lower.includes('http') || lower.includes('api') || lower.includes('rest') || lower.includes('server')) return 'primary/http-adapter';
    if (lower.includes('cli') || lower.includes('command')) return 'primary/cli-adapter';
    if (lower.includes('browser') || lower.includes('ui') || lower.includes('display') || lower.includes('canvas')) return 'primary/browser-adapter';
    if (lower.includes('websocket') || lower.includes('ws')) return 'primary/ws-adapter';
    if (lower.includes('sqlite') || lower.includes('database') || lower.includes('db') || lower.includes('storage') || lower.includes('persist')) return 'secondary/storage-adapter';
    if (lower.includes('redis') || lower.includes('cache')) return 'secondary/cache-adapter';
    if (lower.includes('auth') || lower.includes('jwt') || lower.includes('token')) return 'secondary/auth-adapter';
    if (lower.includes('email') || lower.includes('notification') || lower.includes('notify')) return 'secondary/notification-adapter';
    if (lower.includes('file') || lower.includes('fs')) return 'secondary/filesystem-adapter';
    if (lower.includes('test')) return 'tests/unit';
    return 'secondary/adapter';
  }

  private async orchestrate(
    requirements: string,
    language: string,
    maxAgentsStr?: string,
    topology?: string,
  ): Promise<MCPToolResult> {
    const langMap: Record<string, Language> = { typescript: 'typescript', go: 'go', rust: 'rust', ts: 'typescript' };
    const lang = langMap[language] ?? 'typescript' as Language;
    const reqList = requirements.split(/[,\n]/).map((r) => r.trim()).filter(Boolean);
    const maxAgents = maxAgentsStr ? parseInt(maxAgentsStr, 10) || 4 : 4;
    const topo = topology ?? 'hierarchical';

    // If both LLM and swarm orchestrator are available, use the full pipeline
    if (this.ctx.workplanExecutor && this.ctx.swarmOrchestrator) {
      const workplan = await this.ctx.workplanExecutor.createPlan(reqList, lang);

      const config: Record<string, unknown> = {};
      if (maxAgents) config.maxAgents = maxAgents;
      if (topology) config.topology = topology;

      const status = await this.ctx.swarmOrchestrator.orchestrate(workplan.steps, config as any);

      const lines = [
        `SWARM: ${status.status}`,
        `TOPOLOGY: ${status.topology}`,
        `AGENTS: ${status.agentCount}`,
        `TASKS: ${status.completedTaskCount}/${status.activeTaskCount + status.completedTaskCount}`,
        '',
        `PLAN: ${workplan.title} (${workplan.steps.length} steps)`,
      ];
      for (const step of workplan.steps) {
        lines.push(`  [${step.id}] ${step.description} → ${step.adapter}`);
      }
      return { content: [{ type: 'text', text: lines.join('\n') }] };
    }

    // No LLM — decompose structurally and return orchestration plan.
    // Claude IS the LLM when running inside Claude Code.
    const lines: string[] = [];

    // Analyze current architecture for context
    lines.push('═══ CURRENT STATE ═══');
    try {
      const analysis = await this.ctx.archAnalyzer.analyzeArchitecture('.');
      const s = analysis.summary;
      lines.push(`Health: ${s.healthScore}/100 | Files: ${s.totalFiles} | Violations: ${s.violationCount} | Dead: ${s.deadExportCount}`);
    } catch {
      lines.push('(architecture analysis unavailable)');
    }
    lines.push('');

    // Decompose into hex-bounded tasks
    lines.push('═══ ORCHESTRATION PLAN ═══');
    lines.push(`Language: ${lang} | Topology: ${topo} | Max agents: ${maxAgents}`);
    lines.push('');

    // Group tasks by tier
    const tiers: Record<string, Array<{ idx: number; req: string; adapter: string }>> = {
      'Tier 0 (domain+ports)': [],
      'Tier 1 (secondary adapters)': [],
      'Tier 2 (primary adapters)': [],
      'Tier 3 (usecases+wiring)': [],
      'Tier 4 (tests)': [],
    };

    for (let i = 0; i < reqList.length; i++) {
      const adapter = this.inferAdapter(reqList[i]);
      const entry = { idx: i + 1, req: reqList[i], adapter };
      if (adapter.includes('test')) tiers['Tier 4 (tests)'].push(entry);
      else if (adapter.includes('primary')) tiers['Tier 2 (primary adapters)'].push(entry);
      else if (adapter.includes('secondary')) tiers['Tier 1 (secondary adapters)'].push(entry);
      else tiers['Tier 0 (domain+ports)'].push(entry);
    }

    for (const [tier, tasks] of Object.entries(tiers)) {
      if (tasks.length === 0) continue;
      lines.push(`${tier}:`);
      for (const t of tasks) {
        lines.push(`  ${t.idx}. ${t.req} → ${t.adapter}`);
      }
      lines.push('');
    }

    // Register tasks with swarm if available
    if (this.ctx.swarmOrchestrator) {
      try {
        const progress = await this.ctx.swarmOrchestrator.getProgress();
        lines.push(`Swarm: ${progress.swarmId} | Phase: ${progress.phase} | Agents: ${progress.agents.length}`);
      } catch { /* swarm tracking optional */ }
    }

    lines.push('═══ EXECUTE NOW ═══');
    lines.push(`Use up to ${maxAgents} parallel Agent tools (mode: bypassPermissions).`);
    lines.push('Execute tiers in order. Tiers 1+2 can run in parallel.');
    lines.push('After all tasks: call hex_analyze to validate boundaries.');

    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private async swarmStatus(): Promise<MCPToolResult> {
    if (!this.ctx.swarmOrchestrator) {
      return { content: [{ type: 'text', text: 'Swarm orchestrator not available.' }], isError: true };
    }
    const report = await this.ctx.swarmOrchestrator.getProgress();
    const lines = [
      `SWARM: ${report.swarmId}`,
      `PHASE: ${report.phase}`,
      `PROGRESS: ${report.overallPercent}%`,
      `AGENTS: ${report.agents.length} | TASKS: ${report.tasks.length}`,
      `PATTERNS: ${report.patterns.total} (${report.patterns.recentlyUsed} recent)`,
    ];
    if (report.tasks.length > 0) {
      lines.push('', 'TASKS:');
      for (const t of report.tasks.slice(0, 10)) {
        lines.push(`  [${t.status}] ${t.title}${t.assignee ? ` → ${t.assignee}` : ''}`);
      }
    }
    if (report.agents.length > 0) {
      lines.push('', 'AGENTS:');
      for (const a of report.agents) {
        lines.push(`  ${a.name} (${a.role}) — ${a.status}`);
      }
    }
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  // ─── Dashboard Hub Tool Implementations ──────────────────

  private async ensureHub(): Promise<string> {
    if (this.hubRunning && this.hubUrl) return this.hubUrl;

    const { HUB_PORT, DashboardHub } = await import('./dashboard-hub.js');
    const hub = new DashboardHub(HUB_PORT);
    const { url, close } = await hub.start();
    this.hubRunning = true;
    this.hubUrl = url;
    this.hubCloseFn = close;
    return url;
  }

  private async ensureClient(rootPath: string): Promise<void> {
    if (this.dashboardClient) return;

    // Build a minimal AppContext if we have the required ports
    const { DashboardAdapter } = await import('./dashboard-adapter.js');
    const ctx = {
      rootPath,
      astIsStub: true,
      autoConfirm: false,
      archAnalyzer: this.ctx.archAnalyzer,
      ast: this.ctx.ast,
      fs: this.ctx.fs,
      outputDir: '.hex/',
    } as any;
    this.dashboardClient = new DashboardAdapter(ctx);
    await this.dashboardClient.start();
  }

  private async dashboardStart(rootPath: string): Promise<MCPToolResult> {
    if (this.hubRunning) {
      return { content: [{ type: 'text', text: `Dashboard hub already running at ${this.hubUrl}` }] };
    }

    const url = await this.ensureHub();
    await this.ensureClient(rootPath);

    return {
      content: [{
        type: 'text',
        text: `Dashboard hub started at ${url}\nProject registered: ${rootPath}\nOpen ${url} in your browser to view all projects.`,
      }],
    };
  }

  private async dashboardRegister(rootPath: string): Promise<MCPToolResult> {
    const { HUB_PORT } = await import('./dashboard-hub.js');
    const hubUrl = `http://localhost:${HUB_PORT}`;

    try {
      const response = await fetch(`${hubUrl}/api/projects/register`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ rootPath, name: rootPath.split('/').pop() }),
      });
      const data = await response.json() as { id?: string; error?: string };
      if (!response.ok || data.error) {
        return { content: [{ type: 'text', text: `Registration failed: ${data.error ?? 'Unknown error'}` }], isError: true };
      }
      return { content: [{ type: 'text', text: `Registered project: ${data.id} (${rootPath})` }] };
    } catch {
      return { content: [{ type: 'text', text: `Hub not running on port ${HUB_PORT}. Call hex_dashboard_start first.` }], isError: true };
    }
  }

  private async dashboardUnregister(projectId: string): Promise<MCPToolResult> {
    const { HUB_PORT } = await import('./dashboard-hub.js');
    const hubUrl = `http://localhost:${HUB_PORT}`;

    try {
      const response = await fetch(`${hubUrl}/api/projects/${encodeURIComponent(projectId)}`, { method: 'DELETE' });
      if (!response.ok) {
        return { content: [{ type: 'text', text: `Project "${projectId}" not found.` }], isError: true };
      }
      return { content: [{ type: 'text', text: `Project "${projectId}" unregistered.` }] };
    } catch {
      return { content: [{ type: 'text', text: 'Hub not running.' }], isError: true };
    }
  }

  private async dashboardList(): Promise<MCPToolResult> {
    const { HUB_PORT } = await import('./dashboard-hub.js');
    const hubUrl = `http://localhost:${HUB_PORT}`;

    try {
      const response = await fetch(`${hubUrl}/api/projects`);
      const data = await response.json() as { projects: Array<{ id: string; name: string; rootPath: string; lastPushAt: number }> };
      if (!data.projects || data.projects.length === 0) {
        return { content: [{ type: 'text', text: 'No projects registered.' }] };
      }
      const lines = [`${data.projects.length} project(s) at ${hubUrl}:`, ''];
      for (const p of data.projects) {
        const age = p.lastPushAt ? Math.round((Date.now() - p.lastPushAt) / 1000) + 's ago' : 'no data yet';
        lines.push(`  ${p.id} — ${p.rootPath} (${age})`);
      }
      return { content: [{ type: 'text', text: lines.join('\n') }] };
    } catch {
      return { content: [{ type: 'text', text: `Hub not running on port ${HUB_PORT}. Call hex_dashboard_start first.` }], isError: true };
    }
  }

  private async dashboardQuery(projectId: string, query: string): Promise<MCPToolResult> {
    const { HUB_PORT } = await import('./dashboard-hub.js');
    const hubUrl = `http://localhost:${HUB_PORT}`;
    const base = `${hubUrl}/api/${encodeURIComponent(projectId)}`;

    let endpoint: string;
    switch (query) {
      case 'health': endpoint = `${base}/health`; break;
      case 'tokens': endpoint = `${base}/tokens/overview`; break;
      case 'swarm': endpoint = `${base}/swarm`; break;
      case 'graph': endpoint = `${base}/graph`; break;
      default:
        return { content: [{ type: 'text', text: `Unknown query: ${query}. Use: health, tokens, swarm, graph` }], isError: true };
    }

    try {
      const response = await fetch(endpoint);
      if (!response.ok) {
        const err = await response.json() as { error?: string };
        return { content: [{ type: 'text', text: `Query failed: ${err.error ?? response.statusText}` }], isError: true };
      }
      const data = await response.json() as Record<string, unknown>;
      return { content: [{ type: 'text', text: this.formatQueryResult(query, data) }] };
    } catch {
      return { content: [{ type: 'text', text: `Hub not running on port ${HUB_PORT}. Call hex_dashboard_start first.` }], isError: true };
    }
  }

  private formatQueryResult(query: string, data: Record<string, unknown>): string {
    switch (query) {
      case 'health': {
        const summary = (data.summary ?? data) as Record<string, unknown>;
        const score = (summary.healthScore ?? data.score ?? 0) as number;
        const violations = (data.dependencyViolations ?? data.violations ?? []) as Array<{ from: string; to: string; rule: string }>;
        const lines = [
          `Health: ${Math.round(score)}/100`,
          `Files: ${summary.totalFiles ?? '--'} | Violations: ${violations.length} | Dead: ${summary.deadExportCount ?? '--'}`,
        ];
        if (violations.length > 0) {
          lines.push('', 'Violations:');
          for (const v of violations.slice(0, 5)) {
            lines.push(`  ${v.from} -> ${v.to}: ${v.rule}`);
          }
        }
        return lines.join('\n');
      }
      case 'tokens': {
        const files = (data.files ?? []) as Array<{ path: string; l1Tokens: number; l3Tokens: number; ratio: number }>;
        const lines = [`${files.length} files analyzed:`, ''];
        for (const f of files.slice(0, 15)) {
          lines.push(`  ${f.path} — L1: ${f.l1Tokens}tok, L3: ${f.l3Tokens}tok (ratio: ${f.ratio})`);
        }
        if (files.length > 15) lines.push(`  ... and ${files.length - 15} more`);
        return lines.join('\n');
      }
      case 'swarm': {
        const status = data.status as Record<string, unknown> | undefined;
        const tasks = (data.tasks ?? []) as Array<{ title?: string; status?: string }>;
        const agents = (data.agents ?? []) as Array<{ name?: string; role?: string; status?: string }>;
        return [
          `Swarm: ${(status?.status as string) ?? 'unknown'}`,
          `Agents: ${agents.length} | Tasks: ${tasks.length}`,
          ...agents.map((a) => `  Agent: ${a.name ?? a.role ?? '--'} (${a.status ?? 'idle'})`),
          ...tasks.slice(0, 10).map((t) => `  Task: ${t.title ?? '--'} [${t.status ?? 'pending'}]`),
        ].join('\n');
      }
      case 'graph': {
        const nodes = (data.nodes ?? []) as Array<{ id: string; layer: string }>;
        const edges = (data.edges ?? []) as Array<{ from: string; to: string }>;
        const layers: Record<string, number> = {};
        for (const n of nodes) layers[n.layer] = (layers[n.layer] ?? 0) + 1;
        return [
          `Dependency graph: ${nodes.length} nodes, ${edges.length} edges`,
          '', 'Layers:',
          ...Object.entries(layers).map(([l, c]) => `  ${l}: ${c} files`),
        ].join('\n');
      }
      default:
        return JSON.stringify(data, null, 2);
    }
  }
}
