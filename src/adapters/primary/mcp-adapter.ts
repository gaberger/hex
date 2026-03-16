/**
 * MCP Primary Adapter
 *
 * Exposes hex-intf capabilities as MCP (Model Context Protocol) tools
 * so LLM agents can call them directly. This is a driving/primary adapter —
 * same use cases as the CLI, different interface.
 *
 * Each MCP tool maps 1:1 to a use case method behind a port interface.
 * Dashboard hub tools allow agents to start/manage a multi-project
 * monitoring dashboard and query project health data programmatically.
 */

import type { IArchAnalysisPort, IASTPort, IFileSystemPort, ICodeGenerationPort, IWorkplanPort, ASTSummary, Language, Specification } from '../../core/ports/index.js';
import type { AppContextFactory } from '../../core/ports/app-context.js';
import type { IRegistryPort } from '../../core/ports/registry.js';

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
];

// ─── Dashboard Hub Tool Definitions ───────────────────────

export const HEX_DASHBOARD_TOOLS: MCPToolDefinition[] = [
  {
    name: 'hex_dashboard_start',
    description: 'Start the multi-project dashboard hub server and register the initial project',
    inputSchema: {
      type: 'object',
      properties: {
        rootPath: { type: 'string', description: 'Initial project root path to register' },
        port: { type: 'string', description: 'HTTP port (default: 3847)' },
      },
      required: ['rootPath'],
    },
  },
  {
    name: 'hex_dashboard_register',
    description: 'Register an additional project with the running dashboard hub',
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
  /** Optional: needed for dashboard hub tools. When absent, hub tools return an error. */
  contextFactory?: AppContextFactory;
  /** Optional: project registry for port allocation. When absent, falls back to port 3847. */
  registry?: IRegistryPort;
  /** Optional: LLM code generation. When absent, generate tools return an error. */
  codeGenerator?: ICodeGenerationPort | null;
  /** Optional: LLM workplan creation. When absent, plan tools return an error. */
  workplanExecutor?: IWorkplanPort | null;
}

export class MCPAdapter {
  private hub: import('./dashboard-hub.js').DashboardHub | null = null;
  private hubUrl: string | null = null;
  private hubCloseFn: (() => void) | null = null;

  constructor(private readonly ctx: MCPContext) {}

  /** Shut down the dashboard hub if running. */
  shutdownHub(): void {
    this.hubCloseFn?.();
    this.hub = null;
    this.hubUrl = null;
    this.hubCloseFn = null;
  }

  getTools(): MCPToolDefinition[] {
    return [...HEX_INTF_TOOLS, ...HEX_DASHBOARD_TOOLS];
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
        // ── Dashboard hub tools ──
        case 'hex_dashboard_start':
          return await this.dashboardStart(
            call.arguments.rootPath as string,
            call.arguments.port as string | undefined,
          );
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
    if (!this.ctx.codeGenerator) {
      return { content: [{ type: 'text', text: 'LLM not configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.' }], isError: true };
    }

    const langMap: Record<string, Language> = { typescript: 'typescript', go: 'go', rust: 'rust', ts: 'typescript' };
    const lang = langMap[language];
    if (!lang) {
      return { content: [{ type: 'text', text: `Invalid language: ${language}. Use: typescript, go, rust` }], isError: true };
    }

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

  private async plan(
    requirements: string,
    language: string,
  ): Promise<MCPToolResult> {
    if (!this.ctx.workplanExecutor) {
      return { content: [{ type: 'text', text: 'LLM not configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.' }], isError: true };
    }

    const langMap: Record<string, Language> = { typescript: 'typescript', go: 'go', rust: 'rust', ts: 'typescript' };
    const lang = langMap[language] ?? 'typescript' as Language;

    const reqList = requirements.split(/[,\n]/).map((r) => r.trim()).filter(Boolean);
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

  private async analyzeJson(path: string): Promise<MCPToolResult> {
    const result = await this.ctx.archAnalyzer.analyzeArchitecture(path);
    return { content: [{ type: 'text', text: JSON.stringify(result, null, 2) }] };
  }

  // ─── Dashboard Hub Tool Implementations ──────────────────

  private requireFactory(): AppContextFactory {
    if (!this.ctx.contextFactory) {
      throw new Error('Dashboard hub requires a contextFactory. Pass it when constructing MCPAdapter.');
    }
    return this.ctx.contextFactory;
  }

  private requireRegistry(): IRegistryPort {
    if (!this.ctx.registry) {
      throw new Error('Dashboard hub requires a registry. Pass it when constructing MCPAdapter.');
    }
    return this.ctx.registry;
  }

  /**
   * Register with the project registry to get an assigned port,
   * then start the hub on that port.
   */
  private async ensureHub(rootPath: string): Promise<import('./dashboard-hub.js').DashboardHub> {
    if (this.hub) return this.hub;
    const factory = this.requireFactory();
    const registry = this.requireRegistry();

    // Register with the registry server to get a slot + port
    const projectName = rootPath.split('/').pop() ?? 'unknown';
    const registration = await registry.register(rootPath, projectName);
    const port = registration.port;

    // Write local identity so the project knows its ID
    await registry.writeLocalIdentity(rootPath, {
      id: registration.id,
      name: registration.name,
      createdAt: registration.createdAt,
    });

    const { DashboardHub } = await import('./dashboard-hub.js');
    const hub = new DashboardHub(factory, port);
    const { url, close } = await hub.start();
    this.hub = hub;
    this.hubUrl = url;
    this.hubCloseFn = close;
    return hub;
  }

  private async dashboardStart(rootPath: string, portStr?: string): Promise<MCPToolResult> {
    if (this.hub) {
      return { content: [{ type: 'text', text: `Dashboard hub already running at ${this.hubUrl}` }] };
    }

    // If an explicit port is given, validate it but still register
    if (portStr) {
      const port = parseInt(portStr, 10);
      if (isNaN(port) || port < 1 || port > 65535) {
        return { content: [{ type: 'text', text: 'Invalid port. Must be 1-65535.' }], isError: true };
      }
    }

    const hub = await this.ensureHub(rootPath);
    const slot = await hub.registerProject(rootPath);

    // Touch the registry to update lastSeenAt
    const registry = this.requireRegistry();
    const reg = await registry.findByPath(rootPath);
    if (reg) await registry.touch(reg.id);

    return {
      content: [{
        type: 'text',
        text: [
          `Dashboard hub started at ${this.hubUrl}`,
          `Registered project: ${slot.id} (${slot.ctx.rootPath})`,
          `Port assigned by registry: ${reg?.port ?? 'unknown'}`,
        ].join('\n'),
      }],
    };
  }

  private async dashboardRegister(rootPath: string): Promise<MCPToolResult> {
    const hub = await this.ensureHub(rootPath);

    // Register with the project registry to track this project
    const registry = this.requireRegistry();
    const projectName = rootPath.split('/').pop() ?? 'unknown';
    const registration = await registry.register(rootPath, projectName);

    const slot = await hub.registerProject(rootPath);
    return {
      content: [{
        type: 'text',
        text: [
          `Registered project: ${slot.id} (${slot.ctx.rootPath})`,
          `Registry ID: ${registration.id}`,
          `Assigned port: ${registration.port}`,
        ].join('\n'),
      }],
    };
  }

  private async dashboardUnregister(projectId: string): Promise<MCPToolResult> {
    if (!this.hub) {
      return { content: [{ type: 'text', text: 'Dashboard hub is not running.' }], isError: true };
    }
    const removed = this.hub.unregisterProject(projectId);
    if (!removed) {
      return { content: [{ type: 'text', text: `Project "${projectId}" not found in hub.` }], isError: true };
    }

    // Also remove from registry if possible
    if (this.ctx.registry) {
      const projects = await this.ctx.registry.list();
      const match = projects.find((p) => p.name === projectId || p.id === projectId);
      if (match) {
        await this.ctx.registry.unregister(match.id);
      }
    }

    return { content: [{ type: 'text', text: `Project "${projectId}" unregistered from hub and registry.` }] };
  }

  private async dashboardList(): Promise<MCPToolResult> {
    // List from registry (source of truth) — works even if hub isn't running
    if (this.ctx.registry) {
      const projects = await this.ctx.registry.list();
      if (projects.length === 0) {
        return { content: [{ type: 'text', text: 'No projects registered.' }] };
      }
      const hubRunning = this.hub ? ` (hub at ${this.hubUrl})` : ' (hub not running)';
      const lines = [`${projects.length} project(s) in registry${hubRunning}:`, ''];
      for (const p of projects) {
        const age = Math.round((Date.now() - p.lastSeenAt) / 1000);
        lines.push(`  ${p.name} — ${p.rootPath} (port: ${p.port}, status: ${p.status}, ${age}s ago)`);
      }
      return { content: [{ type: 'text', text: lines.join('\n') }] };
    }

    // Fallback: query the hub's HTTP API
    if (!this.hub) {
      return { content: [{ type: 'text', text: 'No registry and hub is not running. Call hex_dashboard_start first.' }], isError: true };
    }
    const response = await fetch(`${this.hubUrl}/api/projects`);
    const data = await response.json() as { projects: Array<{ id: string; rootPath: string; astIsStub: boolean; registeredAt: number }> };
    if (!data.projects || data.projects.length === 0) {
      return { content: [{ type: 'text', text: 'No projects registered.' }] };
    }
    const lines = [`${data.projects.length} project(s) registered at ${this.hubUrl}:`, ''];
    for (const p of data.projects) {
      const ast = p.astIsStub ? 'stub' : 'real';
      lines.push(`  ${p.id} — ${p.rootPath} (AST: ${ast})`);
    }
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  }

  private async dashboardQuery(projectId: string, query: string): Promise<MCPToolResult> {
    if (!this.hub) {
      return { content: [{ type: 'text', text: 'Dashboard hub is not running. Call hex_dashboard_start first.' }], isError: true };
    }
    const base = `${this.hubUrl}/api/${encodeURIComponent(projectId)}`;
    let endpoint: string;
    switch (query) {
      case 'health': endpoint = `${base}/health`; break;
      case 'tokens': endpoint = `${base}/tokens/overview`; break;
      case 'swarm': endpoint = `${base}/swarm`; break;
      case 'graph': endpoint = `${base}/graph`; break;
      default:
        return { content: [{ type: 'text', text: `Unknown query: ${query}. Use: health, tokens, swarm, graph` }], isError: true };
    }

    const response = await fetch(endpoint);
    if (!response.ok) {
      const err = await response.json() as { error?: string };
      return { content: [{ type: 'text', text: `Query failed: ${err.error ?? response.statusText}` }], isError: true };
    }
    const data = await response.json() as Record<string, unknown>;
    return { content: [{ type: 'text', text: this.formatQueryResult(query, data) }] };
  }

  private formatQueryResult(query: string, data: Record<string, unknown>): string {
    switch (query) {
      case 'health': {
        const score = (data.score ?? data.healthScore ?? 0) as number;
        const violations = Array.isArray(data.violations) ? data.violations as Array<{ from: string; to: string; rule: string }> : [];
        const dead = Array.isArray(data.deadExports) ? data.deadExports.length : 0;
        const lines = [
          `Health: ${Math.round(score)}/100`,
          `Files: ${data.totalFiles ?? '--'} | Violations: ${violations.length} | Dead exports: ${dead}`,
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
        const lines = [
          `Swarm: ${(status?.status as string) ?? 'unknown'}`,
          `Agents: ${agents.length} | Tasks: ${tasks.length}`,
        ];
        for (const a of agents) {
          lines.push(`  Agent: ${a.name ?? a.role ?? '--'} (${a.status ?? 'idle'})`);
        }
        for (const t of tasks.slice(0, 10)) {
          lines.push(`  Task: ${t.title ?? '--'} [${t.status ?? 'pending'}]`);
        }
        return lines.join('\n');
      }
      case 'graph': {
        const nodes = (data.nodes ?? []) as Array<{ id: string; layer: string }>;
        const edges = (data.edges ?? []) as Array<{ from: string; to: string }>;
        const layers: Record<string, number> = {};
        for (const n of nodes) {
          layers[n.layer] = (layers[n.layer] ?? 0) + 1;
        }
        const lines = [
          `Dependency graph: ${nodes.length} nodes, ${edges.length} edges`,
          '',
          'Layers:',
        ];
        for (const [layer, count] of Object.entries(layers)) {
          lines.push(`  ${layer}: ${count} files`);
        }
        return lines.join('\n');
      }
      default:
        return JSON.stringify(data, null, 2);
    }
  }
}
