/**
 * CLI Primary Adapter
 *
 * The main entry point for human interaction. Receives an AppContext
 * via constructor injection and delegates to the appropriate use case.
 *
 * Subcommands:
 *   analyze [path]                  Architecture analysis
 *   summarize <file> [--level L]    AST summary at L0-L3
 *   status                          Swarm progress dashboard
 *   init [--lang ts|go|rust]        Scaffold a new hex project
 *   help                            Print usage
 */

import type {
  ASTSummary,
  Language,
  Specification,
} from '../../core/ports/index.js';
import type { AppContext } from '../../core/ports/app-context.js';

/** Result from runCLI — captures output for testing */
export interface CLIResult {
  exitCode: number;
  output: string;
}

/**
 * Functional CLI entry point — testable with captured output.
 * Pass a writeFn to capture output instead of printing to stdout.
 */
export async function runCLI(
  argv: string[],
  ctx: AppContext,
  writeFn: (msg: string) => void = (msg) => process.stdout.write(msg + '\n'),
): Promise<CLIResult> {
  const lines: string[] = [];
  const write = (msg: string) => { lines.push(msg); writeFn(msg); };
  const adapter = new CLIAdapter(ctx, write);
  const exitCode = await adapter.run(argv);
  return { exitCode, output: lines.join('\n') };
}

// ── Types ───────────────────────────────────────────────

interface ParsedArgs {
  command: string;
  positional: string[];
  flags: Map<string, string>;
}

// ── Arg Parser ──────────────────────────────────────────

function parseArgs(argv: string[]): ParsedArgs {
  const command = argv[0] ?? 'help';
  const positional: string[] = [];
  const flags = new Map<string, string>();

  for (let i = 1; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith('--')) {
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next !== undefined && !next.startsWith('--')) {
        flags.set(key, next);
        i++;
      } else {
        flags.set(key, 'true');
      }
    } else {
      positional.push(arg);
    }
  }

  return { command, positional, flags };
}

// ── Init Templates ──────────────────────────────────────

const TEMPLATES = {
  ports: (ext: string) =>
    ext === 'ts'
      ? [
          '// Define your port interfaces here',
          '// Example:',
          '// export interface IMyPort {',
          '//   doSomething(input: string): Promise<string>;',
          '// }',
          'export {};',
          '',
        ].join('\n')
      : '',

  domain: (ext: string) =>
    ext === 'ts'
      ? ['// Define your domain entities and value objects here', 'export {};', ''].join('\n')
      : '',

  compositionRoot: (ext: string) =>
    ext === 'ts'
      ? [
          'export interface AppContext {',
          '  rootPath: string;',
          '}',
          '',
          'export async function createAppContext(rootPath: string): Promise<AppContext> {',
          '  return { rootPath };',
          '}',
          '',
        ].join('\n')
      : '',

  tsconfig: JSON.stringify(
    {
      compilerOptions: {
        target: 'ES2022',
        module: 'ESNext',
        moduleResolution: 'bundler',
        strict: true,
        esModuleInterop: true,
        skipLibCheck: true,
        outDir: 'dist',
        rootDir: 'src',
        declaration: true,
        declarationMap: true,
        sourceMap: true,
      },
      include: ['src'],
      exclude: ['node_modules', 'dist'],
    },
    null,
    2,
  ) + '\n',

  packageJson: JSON.stringify(
    {
      name: 'my-hex-project',
      version: '0.1.0',
      type: 'module',
      scripts: {
        dev: 'bun run --watch src/index.ts',
        test: 'bun test',
        build: 'bun build src/index.ts --outdir dist --target node',
        check: 'tsc --noEmit',
      },
      devDependencies: {
        typescript: '^5.0.0',
      },
    },
    null,
    2,
  ) + '\n',

  gitignore: [
    'node_modules/',
    'dist/',
    '.env',
    '.hex-intf/',
    '*.tsbuildinfo',
    '',
  ].join('\n'),

  readme: [
    '# My Hex Project',
    '',
    'Scaffolded with [hex-intf](https://github.com/your-org/hex-intf).',
    '',
    '## Quick Start',
    '',
    '```bash',
    'bun install',
    'bun run dev',
    '```',
    '',
    '## Commands',
    '',
    '| Command | Description |',
    '|---------|-------------|',
    '| `bun run dev` | Start dev server with watch |',
    '| `bun test` | Run tests |',
    '| `bun run build` | Build for production |',
    '| `bun run check` | Type-check without emitting |',
    '',
    '## Architecture',
    '',
    '```',
    'src/',
    '  core/',
    '    domain/        Domain entities and value objects',
    '    ports/         Port interfaces (input + output)',
    '    usecases/      Use case implementations',
    '  adapters/',
    '    primary/       Driving adapters (CLI, HTTP, etc.)',
    '    secondary/     Driven adapters (DB, FS, API, etc.)',
    '  infrastructure/  Cross-cutting concerns',
    '  composition-root.ts',
    '```',
    '',
  ].join('\n'),

  claudeMd: (lang: string) => [
    `# Hexagonal Architecture Project`,
    '',
    '## Behavioral Rules',
    '',
    '- ALWAYS read a file before editing it',
    '- NEVER commit secrets, credentials, or .env files',
    `- ALWAYS run \`${lang === 'ts' ? 'bun test' : lang === 'go' ? 'go test ./...' : 'cargo test'}\` after making code changes`,
    `- ALWAYS run \`${lang === 'ts' ? 'bun run build' : lang === 'go' ? 'go build ./...' : 'cargo build'}\` before committing`,
    '',
    '## Hexagonal Architecture Rules (ENFORCED)',
    '',
    '1. **domain/** must only import from **domain/**',
    '2. **ports/** may import from **domain/** but nothing else',
    '3. **usecases/** may import from **domain/** and **ports/** only',
    '4. **adapters/primary/** may import from **ports/** only',
    '5. **adapters/secondary/** may import from **ports/** only',
    '6. **adapters must NEVER import other adapters** (cross-adapter coupling)',
    '7. **composition-root** is the ONLY file that imports from adapters',
    ...(lang === 'ts' ? ['8. All relative imports MUST use `.js` extensions (NodeNext module resolution)'] : []),
    '',
    '## File Organization',
    '',
    '```',
    'src/',
    '  core/',
    '    domain/          # Pure business logic, zero external deps',
    '    ports/           # Typed interfaces (contracts between layers)',
    '    usecases/        # Application logic composing ports',
    '  adapters/',
    '    primary/         # Driving adapters (CLI, HTTP, browser input)',
    '    secondary/       # Driven adapters (DB, API, filesystem)',
    '  composition-root   # Wires adapters to ports (single DI point)',
    '```',
    '',
    '## Security',
    '',
    '- Never commit `.env` files — use `.env.example`',
    '- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.',
    '',
    '## On Startup',
    '',
    'A SessionStart hook runs `scripts/hex-startup.sh` which outputs project status. You MUST:',
    '',
    '1. Read the hook output (it appears in a system-reminder) to understand project progress',
    '2. Read `PRD.md` for the full project scope',
    '3. **Immediately present** the user with:',
    '   - Project name and goal (from PRD)',
    '   - Pipeline progress (which hex layers are done vs todo)',
    '   - The recommended next step',
    '   - Ask what they would like to work on',
    '4. Do NOT wait for the user to ask — proactively guide them',
    '',
    '## Development Pipeline (follow this order)',
    '',
    '1. **Domain** — Define entities and value objects in `domain/`',
    '2. **Ports** — Define typed interfaces (contracts) in `ports/`',
    '3. **Use Cases** — Implement business logic in `usecases/`, importing only domain + ports',
    '4. **Adapters** — Implement primary (input) and secondary (output) adapters',
    '5. **Composition Root** — Wire adapters to ports in `composition-root`',
    '6. **Tests** — Unit tests (London-school mocks) + integration tests',
    '7. **Validate** — Run `hex-intf analyze .` to check architecture health',
    '',
  ].join('\n'),
} as const;

// ── CLI Adapter ─────────────────────────────────────────

export class CLIAdapter {
  constructor(
    private readonly ctx: AppContext,
    private readonly writeLn: (text: string) => void = (t) => process.stdout.write(t + '\n'),
  ) {}

  async run(argv: string[]): Promise<number> {
    const args = parseArgs(argv);

    try {
      switch (args.command) {
        case 'analyze':
          return await this.analyze(args);
        case 'summarize':
          return await this.summarize(args);
        case 'generate':
          return await this.generate(args);
        case 'plan':
          return await this.plan(args);
        case 'dashboard':
          return await this.dashboard(args);
        case 'hub':
          return await this.hub(args);
        case 'status':
          return await this.status();
        case 'setup':
          return await this.setup();
        case 'init':
          return await this.init(args);
        case 'mcp':
          return await this.mcp();
        case 'projects':
          return await this.projects();
        case 'help':
        case '--help':
        case '-h':
          return this.help();
        default:
          this.writeLn(`Unknown command: ${args.command}`);
          this.writeLn('Run "hex-intf help" for usage.');
          return 1;
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.writeLn(`Error: ${message}`);
      return 1;
    }
  }

  // ── analyze ─────────────────────────────────────────

  private async analyze(args: ParsedArgs): Promise<number> {
    const targetPath = args.positional[0] ?? '.';
    const jsonMode = args.flags.has('json');

    const result = await this.ctx.archAnalyzer.analyzeArchitecture(targetPath);
    const s = result.summary;

    // Machine-readable JSON output for CI/CD pipelines
    if (jsonMode) {
      this.writeLn(JSON.stringify(result, null, 2));
      return s.healthScore >= 50 ? 0 : 1;
    }

    if (this.ctx.astIsStub) {
      this.writeLn('\u26a0 Running without tree-sitter grammars \u2014 results may be incomplete');
      this.writeLn('');
    }

    this.writeLn(`Analyzing architecture at: ${targetPath}`);
    this.writeLn('');

    this.writeLn(`Files scanned:    ${s.totalFiles}`);
    this.writeLn(`Total exports:    ${s.totalExports}`);
    this.writeLn(`Health score:     ${s.healthScore}/100`);
    this.writeLn('');

    if (s.deadExportCount > 0) {
      this.writeLn(`Dead exports (${s.deadExportCount}):`);
      for (const d of result.deadExports.slice(0, 10)) {
        this.writeLn(`  ${d.filePath}: ${d.exportName} (${d.kind})`);
      }
      if (result.deadExports.length > 10) {
        this.writeLn(`  ... and ${result.deadExports.length - 10} more`);
      }
      this.writeLn('');
    }

    if (s.violationCount > 0) {
      this.writeLn(`Hex boundary violations (${s.violationCount}):`);
      for (const v of result.dependencyViolations.slice(0, 10)) {
        this.writeLn(`  ${v.from} -> ${v.to}`);
        this.writeLn(`    ${v.rule}`);
      }
      if (result.dependencyViolations.length > 10) {
        this.writeLn(`  ... and ${result.dependencyViolations.length - 10} more`);
      }
      this.writeLn('');
    }

    if (s.circularCount > 0) {
      this.writeLn(`Circular dependencies (${s.circularCount}):`);
      for (const cycle of result.circularDeps.slice(0, 5)) {
        this.writeLn(`  ${cycle.join(' -> ')}`);
      }
      this.writeLn('');
    }

    if (result.orphanFiles.length > 0) {
      this.writeLn(`Orphan files (${result.orphanFiles.length}):`);
      for (const f of result.orphanFiles.slice(0, 10)) {
        this.writeLn(`  ${f}`);
      }
      this.writeLn('');
    }

    return s.healthScore >= 50 ? 0 : 1;
  }

  // ── summarize ───────────────────────────────────────

  private async summarize(args: ParsedArgs): Promise<number> {
    const filePath = args.positional[0];
    if (!filePath) {
      this.writeLn('Usage: hex-intf summarize <file> [--level L0|L1|L2|L3]');
      return 1;
    }

    const levelStr = args.flags.get('level') ?? 'L1';
    const validLevels: ASTSummary['level'][] = ['L0', 'L1', 'L2', 'L3'];
    if (!validLevels.includes(levelStr as ASTSummary['level'])) {
      this.writeLn(`Invalid level: ${levelStr}. Must be one of: ${validLevels.join(', ')}`);
      return 1;
    }
    const level = levelStr as ASTSummary['level'];

    const summary = await this.ctx.ast.extractSummary(filePath, level);

    this.writeLn(`File:     ${summary.filePath}`);
    this.writeLn(`Language: ${summary.language}`);
    this.writeLn(`Level:    ${summary.level}`);
    this.writeLn(`Lines:    ${summary.lineCount}`);
    this.writeLn(`Tokens:   ${summary.tokenEstimate}`);
    this.writeLn('');

    if (summary.exports.length > 0) {
      this.writeLn(`Exports (${summary.exports.length}):`);
      for (const exp of summary.exports) {
        const sig = exp.signature ? `: ${exp.signature}` : '';
        this.writeLn(`  ${exp.kind} ${exp.name}${sig}`);
      }
      this.writeLn('');
    }

    if (summary.imports.length > 0) {
      this.writeLn(`Imports (${summary.imports.length}):`);
      for (const imp of summary.imports) {
        this.writeLn(`  { ${imp.names.join(', ')} } from '${imp.from}'`);
      }
      this.writeLn('');
    }

    if (summary.raw) {
      this.writeLn('Raw AST:');
      this.writeLn(summary.raw);
    }

    return 0;
  }

  // ── generate ────────────────────────────────────────

  private async generate(args: ParsedArgs): Promise<number> {
    if (!this.ctx.codeGenerator) {
      this.writeLn('LLM not configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.');
      return 1;
    }

    const specFile = args.positional[0];
    if (!specFile) {
      this.writeLn('Usage: hex-intf generate <spec-file> [--adapter <name>] [--lang ts|go|rust]');
      return 1;
    }

    const langStr = args.flags.get('lang') ?? 'ts';
    const langMap: Record<string, Language> = { ts: 'typescript', go: 'go', rust: 'rust' };
    const lang = langMap[langStr];
    if (!lang) {
      this.writeLn(`Invalid language: ${langStr}. Must be one of: ts, go, rust`);
      return 1;
    }

    const adapterName = args.flags.get('adapter');

    const content = await this.ctx.fs.read(specFile);
    const spec: Specification = {
      title: specFile,
      requirements: content.split('\n').filter((line) => line.trim().length > 0),
      constraints: [],
      targetLanguage: lang,
      targetAdapter: adapterName,
    };

    const result = await this.ctx.codeGenerator.generateFromSpec(spec, lang);

    const outputFile = args.flags.get('output');
    if (outputFile) {
      await this.ctx.fs.write(outputFile, result.content);
      this.writeLn(`Generated: ${result.filePath}`);
      this.writeLn(`Written to: ${outputFile}`);
    } else {
      this.writeLn(`Generated: ${result.filePath}`);
      this.writeLn(`Language:  ${result.language}`);
      this.writeLn('');
      this.writeLn(result.content);
    }

    return 0;
  }

  // ── plan ───────────────────────────────────────────

  private async plan(args: ParsedArgs): Promise<number> {
    if (!this.ctx.workplanExecutor) {
      this.writeLn('LLM not configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.');
      return 1;
    }

    const requirements = args.positional;
    if (requirements.length === 0) {
      this.writeLn('Usage: hex-intf plan <requirements...>');
      return 1;
    }

    const langStr = args.flags.get('lang') ?? 'ts';
    const langMap: Record<string, Language> = { ts: 'typescript', go: 'go', rust: 'rust' };
    const lang = langMap[langStr] ?? 'typescript' as Language;

    const workplan = await this.ctx.workplanExecutor.createPlan(requirements, lang);

    this.writeLn(`Workplan: ${workplan.title}`);
    this.writeLn(`ID:       ${workplan.id}`);
    this.writeLn(`Steps:    ${workplan.steps.length}`);
    this.writeLn(`Budget:   ~${workplan.estimatedTokenBudget} tokens`);
    this.writeLn('');

    for (const step of workplan.steps) {
      const deps = step.dependencies.length > 0
        ? ` (depends on: ${step.dependencies.join(', ')})`
        : '';
      this.writeLn(`  [${step.id}] ${step.description}`);
      this.writeLn(`    adapter: ${step.adapter}${deps}`);
    }

    return 0;
  }

  // ── dashboard ───────────────────────────────────────

  private async dashboard(args: ParsedArgs): Promise<number> {
    const explicitPort = args.flags.get('port');

    // Register with the project registry to get an assigned port
    const ctx = this.ctx as AppContext;
    const projectName = ctx.rootPath.split('/').pop() ?? 'unknown';
    const registration = await ctx.registry.register(ctx.rootPath, projectName);
    const port = explicitPort ? parseInt(explicitPort, 10) : registration.port;

    if (isNaN(port) || port < 1 || port > 65535) {
      this.writeLn('Invalid port number. Must be 1-65535.');
      return 1;
    }

    // Write local identity so the project knows its registry ID
    await ctx.registry.writeLocalIdentity(ctx.rootPath, {
      id: registration.id,
      name: registration.name,
      createdAt: registration.createdAt,
    });

    // Dynamic import to avoid loading http server when not needed
    const { DashboardAdapter } = await import('./dashboard-adapter.js');
    const dashboard = new DashboardAdapter(ctx, port);
    const { url } = await dashboard.start();
    this.writeLn(`Dashboard running at ${url}`);
    this.writeLn(`Registry: ${registration.id} (port ${registration.port})`);

    // Wire notification orchestrator → dashboard SSE broadcast
    if (this.ctx.notificationOrchestrator) {
      this.ctx.notificationOrchestrator.addListener((notification) => {
        dashboard.broadcast(notification.level, {
          id: notification.id,
          level: notification.level,
          message: notification.title,
          detail: notification.detail,
          source: notification.source,
          timestamp: notification.timestamp,
          context: notification.context,
        });
      });
      this.writeLn('Notifications connected to dashboard SSE.');
    } else {
      this.writeLn('No notification orchestrator — SSE events will be limited.');
    }

    this.writeLn('Press Ctrl+C to stop.');

    // Keep the process alive until interrupted
    await new Promise(() => {});
    return 0;
  }

  // ── hub (multi-project dashboard) ───────────────────

  private async hub(args: ParsedArgs): Promise<number> {
    const port = parseInt(args.flags.get('port') ?? '3847', 10);
    if (isNaN(port) || port < 1 || port > 65535) {
      this.writeLn('Invalid port number. Must be 1-65535.');
      return 1;
    }

    // Dynamic import to avoid loading http server when not needed
    const { DashboardHub } = await import('./dashboard-hub.js');
    // Inject the composition root factory — keeps the hub hex-clean
    const { createAppContext: factory } = await import('../../composition-root.js');

    const hub = new DashboardHub(factory, port);
    const { url } = await hub.start();
    this.writeLn(`Dashboard Hub running at ${url}`);

    // Register the current project as the first project
    const slot = await hub.registerProject(this.ctx.rootPath);
    this.writeLn(`Registered project: ${slot.id} (${this.ctx.rootPath})`);

    // Register additional projects from --project flags
    const extraProjects = args.positional;
    for (const projectPath of extraProjects) {
      try {
        const s = await hub.registerProject(projectPath);
        this.writeLn(`Registered project: ${s.id} (${s.ctx.rootPath})`);
      } catch (err) {
        this.writeLn(`Failed to register ${projectPath}: ${err instanceof Error ? err.message : String(err)}`);
      }
    }

    this.writeLn('');
    this.writeLn('Register more projects via POST /api/projects/register { "rootPath": "..." }');
    this.writeLn('Press Ctrl+C to stop.');

    await new Promise(() => {});
    return 0;
  }

  // ── status ──────────────────────────────────────────

  private async status(): Promise<number> {
    this.writeLn('Swarm status: use "hex-intf analyze" to check project health.');
    return 0;
  }

  // ── mcp ────────────────────────────────────────────
  // Starts hex-intf as a stdio MCP server so any project can use it:
  //   npx hex-intf mcp
  // Or in .claude/settings.local.json:
  //   { "mcpServers": { "hex-intf": { "command": "npx", "args": ["hex-intf", "mcp"] } } }

  private async mcp(): Promise<number> {
    const { MCPAdapter, HEX_INTF_TOOLS, HEX_DASHBOARD_TOOLS } = await import('./mcp-adapter.js');

    const adapter = new MCPAdapter({
      archAnalyzer: this.ctx.archAnalyzer,
      ast: this.ctx.ast,
      fs: this.ctx.fs,
      codeGenerator: this.ctx.codeGenerator,
      workplanExecutor: this.ctx.workplanExecutor,
      swarmOrchestrator: this.ctx.swarmOrchestrator,
    });

    const allTools = adapter.getTools();

    // JSON-RPC stdio MCP server — reads from stdin, writes to stdout
    const readline = await import('node:readline');
    const rl = readline.createInterface({ input: process.stdin });

    const send = (msg: unknown) => {
      process.stdout.write(JSON.stringify(msg) + '\n');
    };

    // Log to stderr so it doesn't interfere with MCP protocol on stdout
    process.stderr.write(`[hex-intf] MCP server started with ${allTools.length} tools\n`);

    rl.on('line', async (line: string) => {
      let request: { jsonrpc: string; id?: number | string; method: string; params?: Record<string, unknown> };
      try {
        request = JSON.parse(line);
      } catch {
        return; // Ignore malformed lines
      }

      const { id, method, params } = request;

      switch (method) {
        case 'initialize':
          send({
            jsonrpc: '2.0', id,
            result: {
              protocolVersion: '2024-11-05',
              capabilities: { tools: {} },
              serverInfo: { name: 'hex-intf', version: '1.0.0' },
            },
          });
          break;

        case 'notifications/initialized':
          // Client ack — no response needed
          break;

        case 'tools/list':
          send({
            jsonrpc: '2.0', id,
            result: {
              tools: allTools.map((t) => ({
                name: t.name,
                description: t.description,
                inputSchema: t.inputSchema,
              })),
            },
          });
          break;

        case 'tools/call': {
          const toolName = (params as any)?.name as string;
          const toolArgs = (params as any)?.arguments as Record<string, unknown> ?? {};
          const result = await adapter.handleToolCall({ name: toolName, arguments: toolArgs });
          send({
            jsonrpc: '2.0', id,
            result: { content: result.content, isError: result.isError },
          });
          break;
        }

        default:
          send({
            jsonrpc: '2.0', id,
            error: { code: -32601, message: `Method not found: ${method}` },
          });
      }
    });

    // Keep the process alive
    await new Promise<void>((resolve) => {
      rl.on('close', resolve);
      process.on('SIGINT', () => { adapter.shutdownHub(); resolve(); });
      process.on('SIGTERM', () => { adapter.shutdownHub(); resolve(); });
    });

    return 0;
  }

  // ── init ────────────────────────────────────────────

  private async init(args: ParsedArgs): Promise<number> {
    const skipWizard = args.flags.get('skip-wizard') === 'true';

    // ── Phase 1: Check for existing PRD or run wizard ──

    let scope: { name: string; summary: string; lang: string };

    if (await this.ctx.fs.exists('PRD.md')) {
      this.writeLn('Found existing PRD.md — skipping wizard.');
      scope = await this.parsePrd();
    } else if (skipWizard) {
      scope = { name: 'my-hex-project', summary: 'A hexagonal architecture project', lang: args.flags.get('lang') ?? 'ts' };
    } else {
      scope = await this.scopeWizard(args);
    }

    const langStr = scope.lang;
    const langs = langStr.split('+').map((l) => l.trim());
    const validLangs = ['ts', 'go', 'rust'];
    const invalid = langs.filter((l) => !validLangs.includes(l));
    if (invalid.length > 0) {
      this.writeLn(`Invalid language(s): ${invalid.join(', ')}. Must be: ${validLangs.join(', ')}`);
      return 1;
    }

    const primaryLang = langs[0];
    const ext = primaryLang === 'ts' ? 'ts' : primaryLang === 'go' ? 'go' : 'rs';
    const isMultiStack = langs.length > 1;

    // ── Phase 2: Generate PRD.md ──────────────────────

    this.writeLn('');
    this.writeLn('Generating PRD.md...');
    const prd = this.generatePrd(scope);

    this.writeLn(`Scaffolding ${scope.name} (${langStr})...`);
    this.writeLn('');

    const created: string[] = [];
    const skipped: string[] = [];

    // Override package name with wizard input
    const pkgJson = JSON.parse(TEMPLATES.packageJson);
    pkgJson.name = scope.name;
    const packageJsonContent = JSON.stringify(pkgJson, null, 2) + '\n';

    /** Write a file only if it does not already exist. Paths are relative to rootPath. */
    const safeWrite = async (relPath: string, content: string) => {
      if (await this.ctx.fs.exists(relPath)) {
        skipped.push(relPath);
      } else {
        await this.ctx.fs.write(relPath, content);
        created.push(relPath);
      }
    };

    // ── Starter source files ──────────────────────────

    if (isMultiStack) {
      // Multi-stack: separate directories per language (e.g., backend/ + frontend/)
      for (const lang of langs) {
        const langExt = lang === 'ts' ? 'ts' : lang === 'go' ? 'go' : 'rs';
        const dir = lang === 'go' || lang === 'rust' ? 'backend' : 'frontend';
        await safeWrite(`${dir}/src/core/ports/index.${langExt}`, TEMPLATES.ports(langExt));
        await safeWrite(`${dir}/src/core/domain/index.${langExt}`, TEMPLATES.domain(langExt));
        await safeWrite(`${dir}/src/core/usecases/.gitkeep`, '');
        await safeWrite(`${dir}/src/adapters/primary/.gitkeep`, '');
        await safeWrite(`${dir}/src/adapters/secondary/.gitkeep`, '');
        await safeWrite(`${dir}/src/infrastructure/.gitkeep`, '');
        await safeWrite(`${dir}/src/composition-root.${langExt}`, TEMPLATES.compositionRoot(langExt));
        await safeWrite(`${dir}/tests/unit/.gitkeep`, '');
        await safeWrite(`${dir}/tests/integration/.gitkeep`, '');
        await safeWrite(`${dir}/CLAUDE.md`, TEMPLATES.claudeMd(lang));
      }
    } else {
      await safeWrite(`src/core/ports/index.${ext}`, TEMPLATES.ports(ext));
      await safeWrite(`src/core/domain/index.${ext}`, TEMPLATES.domain(ext));
      await safeWrite(`src/core/usecases/.gitkeep`, '');
      await safeWrite(`src/adapters/primary/.gitkeep`, '');
      await safeWrite(`src/adapters/secondary/.gitkeep`, '');
      await safeWrite(`src/infrastructure/.gitkeep`, '');
      await safeWrite(`src/composition-root.${ext}`, TEMPLATES.compositionRoot(ext));
    }

    // ── Test directories (single-stack only, multi-stack has them per dir) ──

    if (!isMultiStack) {
      await safeWrite('tests/unit/.gitkeep', '');
      await safeWrite('tests/integration/.gitkeep', '');
    }

    // ── Config directory ──────────────────────────────

    await safeWrite('config/.gitkeep', '');

    // ── Root config files ─────────────────────────────

    if (langs.includes('ts')) {
      const tsconfigPath = isMultiStack ? 'frontend/tsconfig.json' : 'tsconfig.json';
      await safeWrite(tsconfigPath, TEMPLATES.tsconfig);
    }
    await safeWrite('package.json', packageJsonContent);
    await safeWrite('.gitignore', TEMPLATES.gitignore);
    await safeWrite('README.md', TEMPLATES.readme);
    if (isMultiStack) {
      // Root CLAUDE.md references both stacks
      const rootClaude = [
        `# ${scope.name} — Multi-Stack Hexagonal Architecture`,
        '',
        'This project has multiple stacks, each with its own hex boundaries:',
        '',
        ...langs.map((l) => {
          const dir = l === 'go' || l === 'rust' ? 'backend' : 'frontend';
          return `- **${dir}/** — ${l === 'ts' ? 'TypeScript' : l === 'go' ? 'Go' : 'Rust'} (see ${dir}/CLAUDE.md)`;
        }),
        '',
        '## Cross-Stack Rules',
        '',
        '- Backend and frontend communicate ONLY via API contracts (HTTP/gRPC)',
        '- Shared types belong in a `shared/` directory or are duplicated per stack',
        '- Each stack has its own CLAUDE.md with language-specific hex rules',
        '- NEVER import code across stack boundaries',
        '',
        '## Security',
        '',
        '- Never commit `.env` files — use `.env.example`',
        '- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.',
        '',
        '## On Startup',
        '',
        'When a new conversation begins in this project:',
        '',
        '1. Read `PRD.md` to understand the project scope and requirements',
        '2. Scan `backend/` and `frontend/` to assess build progress',
        '3. Present the user with a **status summary**:',
        '   - What has been built so far in each stack',
        '   - What the next logical step is (following the development pipeline)',
        '   - Any issues found (missing ports, empty adapters, no tests)',
        '4. Ask the user what they want to work on, suggesting the next step',
        '',
        '## Development Pipeline (follow this order, per stack)',
        '',
        '1. **Domain** — Define entities and value objects in `domain/`',
        '2. **Ports** — Define typed interfaces (contracts) in `ports/`',
        '3. **Use Cases** — Implement business logic in `usecases/`',
        '4. **Adapters** — Implement primary (input) and secondary (output) adapters',
        '5. **Composition Root** — Wire adapters to ports',
        '6. **Tests** — Unit tests + integration tests',
        '7. **Validate** — Run `hex-intf analyze .` to check architecture health',
        '',
      ].join('\n');
      await safeWrite('CLAUDE.md', rootClaude);
    } else {
      await safeWrite('CLAUDE.md', TEMPLATES.claudeMd(primaryLang));
    }
    await safeWrite('PRD.md', prd);

    // ── Initialize ruflo swarm if not already running ──

    try {
      const swarmStatus = await this.ctx.swarm.status();
      if (swarmStatus.status === 'running' || swarmStatus.status === 'idle') {
        this.writeLn(`Swarm already initialized (${swarmStatus.status}).`);
      } else {
        await this.initSwarm();
      }
    } catch {
      await this.initSwarm();
    }
    this.writeLn('');

    // ── Register project ────────────────────────────────

    try {
      const { resolve } = await import('node:path');
      const absPath = resolve(this.ctx.rootPath);
      const existing = await this.ctx.registry.readLocalIdentity(absPath);
      if (existing) {
        await this.ctx.registry.touch(existing.id);
        this.writeLn(`Project registered: ${existing.name} (${existing.id.slice(0, 8)})`);
      } else {
        const reg = await this.ctx.registry.register(absPath, scope.name);
        await this.ctx.registry.writeLocalIdentity(absPath, {
          id: reg.id,
          name: reg.name,
          createdAt: reg.createdAt,
        });
        this.writeLn(`Project registered: ${reg.name} (${reg.id.slice(0, 8)}) on port ${reg.port}`);
      }
    } catch (err) {
      this.writeLn(`Registry: ${err instanceof Error ? err.message : String(err)}`);
    }

    // ── Install session-start hook ─────────────────────

    await this.installStartupHook();

    // ── Run setup (grammars + skills) ─────────────────

    this.writeLn('Running setup (grammars + skills)...');
    this.writeLn('');
    await this.setup();
    this.writeLn('');

    // ── Summary ───────────────────────────────────────

    if (created.length > 0) {
      this.writeLn(`Created (${created.length}):`);
      for (const f of created) {
        this.writeLn(`  + ${f}`);
      }
    }
    if (skipped.length > 0) {
      this.writeLn(`Skipped (already exist) (${skipped.length}):`);
      for (const f of skipped) {
        this.writeLn(`  - ${f}`);
      }
    }

    this.writeLn('');
    this.writeLn('Done. Next steps:');
    this.writeLn('  bun install');
    this.writeLn('  claude              # Start AI-guided development');
    this.writeLn('');
    this.writeLn('When Claude starts, type "start" and it will:');
    this.writeLn('  - Read your PRD.md and assess progress');
    this.writeLn('  - Show which hex layers are built vs pending');
    this.writeLn('  - Guide you through the next step');

    return 0;
  }

  // ── projects ──────────────────────────────────

  private async projects(): Promise<number> {
    const projects = await this.ctx.registry.list();
    if (projects.length === 0) {
      this.writeLn('No registered projects. Run "hex-intf init" in a project directory.');
      return 0;
    }

    this.writeLn(`Registered projects (${projects.length}):`);
    this.writeLn('');
    for (const p of projects) {
      const age = Math.round((Date.now() - p.lastSeenAt) / 60000);
      const stale = age > 1440 ? ' (stale)' : '';
      this.writeLn(`  ${p.id.slice(0, 8)}  ${p.name.padEnd(20)} :${p.port}  ${p.status}${stale}`);
      this.writeLn(`           ${p.rootPath}`);
    }
    return 0;
  }

  // ── scopeWizard ────────────────────────────────

  private async scopeWizard(args: ParsedArgs): Promise<{ name: string; summary: string; lang: string }> {
    this.writeLn('─── hex-intf project setup ───────────────────');
    this.writeLn('');

    const name = await this.prompt('Project name', 'my-hex-project');
    const summary = await this.prompt('Describe what this project does', '');

    // Infer language(s) from summary or flag
    const flagLang = args.flags.get('lang');
    const lang = flagLang ?? this.inferLang(summary);

    this.writeLn('');
    this.writeLn(`  Inferred stack: ${lang}`);

    return { name, summary, lang };
  }

  private inferLang(summary: string): string {
    const lower = summary.toLowerCase();
    const langs: string[] = [];

    if (lower.includes('go') || lower.includes('golang')) langs.push('go');
    if (lower.includes('ts') || lower.includes('typescript') || lower.includes('htmx') || lower.includes('react') || lower.includes('frontend') || lower.includes('front end') || lower.includes('front-end') || lower.includes('web ui') || lower.includes('next')) langs.push('ts');
    if (lower.includes('rust') || lower.includes('cargo')) langs.push('rust');

    if (langs.length === 0) return 'ts';
    return langs.join('+');
  }

  // ── parsePrd ─────────────────────────────────

  private async parsePrd(): Promise<{ name: string; summary: string; lang: string }> {
    const content = await this.ctx.fs.read('PRD.md');
    const lines = content.split('\n');

    // Extract name from first heading: "# project-name — ..."
    const titleLine = lines.find((l) => l.startsWith('# '));
    const name = titleLine
      ? titleLine.replace(/^#\s+/, '').split(/\s*[—–-]\s*/)[0].trim()
      : 'my-hex-project';

    // Extract summary from the section after "## Summary"
    const summaryIdx = lines.findIndex((l) => /^##\s+summary/i.test(l));
    let summary = '';
    if (summaryIdx >= 0) {
      for (let i = summaryIdx + 1; i < lines.length; i++) {
        if (lines[i].startsWith('##')) break;
        const trimmed = lines[i].trim();
        if (trimmed && !trimmed.startsWith('_')) {
          summary = trimmed;
          break;
        }
      }
    }

    // Extract lang from "**Stack:**" or "**Language:**" line, fallback to inferring from summary
    const stackLine = lines.find((l) => /\*\*(Stack|Language)\*\*/.test(l));
    let lang = 'ts';
    if (stackLine) {
      const lower = stackLine.toLowerCase();
      const langs: string[] = [];
      if (lower.includes('go')) langs.push('go');
      if (lower.includes('typescript') || lower.includes('ts')) langs.push('ts');
      if (lower.includes('rust')) langs.push('rust');
      if (langs.length > 0) lang = langs.join('+');
    }
    // If Stack line only found one lang but summary mentions more, re-infer
    if (summary) {
      const inferred = this.inferLang(summary);
      const inferredLangs = inferred.split('+');
      const currentLangs = lang.split('+');
      if (inferredLangs.length > currentLangs.length) {
        lang = inferred;
      }
    }

    this.writeLn(`  Name: ${name}`);
    this.writeLn(`  Stack: ${lang}`);
    this.writeLn(`  Summary: ${summary || '(none)'}`);

    return { name, summary, lang };
  }

  // ── generatePrd ──────────────────────────────

  private generatePrd(scope: { name: string; summary: string; lang: string }): string {
    const langs = scope.lang.split('+');
    const langNames: Record<string, string> = { ts: 'TypeScript', go: 'Go', rust: 'Rust' };
    const langFull = langs.map((l) => langNames[l] ?? l).join(' + ');
    const isMulti = langs.length > 1;

    return [
      `# ${scope.name} — Product Requirements`,
      '',
      '## Summary',
      '',
      scope.summary || '_No description provided._',
      '',
      '## Technical Decisions',
      '',
      `- **Stack:** ${langFull}`,
      ...(isMulti ? [`- **Structure:** Multi-stack (${langs.includes('go') || langs.includes('rust') ? 'backend/' : ''}${langs.includes('go') || langs.includes('rust') ? ' + ' : ''}${langs.includes('ts') ? 'frontend/' : ''})`] : []),
      `- **Architecture:** Hexagonal (ports & adapters)`,
      `- **Scaffolded by:** hex-intf`,
      '',
      '## Scope',
      '',
      '### In Scope',
      '',
      '- [ ] Define domain entities and value objects',
      '- [ ] Define port interfaces (contracts)',
      '- [ ] Implement primary adapter(s)',
      '- [ ] Implement secondary adapter(s)',
      '- [ ] Wire composition root',
      '- [ ] Unit tests (London-school mocks)',
      '',
      '### Out of Scope',
      '',
      '- _TBD — add items as the project evolves_',
      '',
      '## Architecture',
      '',
      '```',
      'src/',
      '  core/',
      '    domain/          # Pure business logic, zero external deps',
      '    ports/           # Typed interfaces (contracts)',
      '    usecases/        # Application logic composing ports',
      '  adapters/',
      '    primary/         # Driving adapters (CLI, HTTP, browser)',
      '    secondary/       # Driven adapters (DB, API, filesystem)',
      '  composition-root   # Wires adapters to ports',
      '```',
      '',
      '## Next Steps',
      '',
      '1. Fill in domain entities based on the summary above',
      '2. Define port interfaces for each boundary',
      '3. Implement adapters',
      '4. Run `hex-intf analyze .` to validate architecture',
      '',
    ].join('\n');
  }

  // ── prompt ───────────────────────────────────

  private prompt(question: string, defaultValue: string): Promise<string> {
    const { createInterface } = require('readline');
    const rl = createInterface({ input: process.stdin, output: process.stdout });
    const suffix = defaultValue ? ` (${defaultValue})` : '';
    return new Promise((resolve) => {
      rl.question(`  ${question}${suffix}: `, (answer: string) => {
        rl.close();
        resolve(answer.trim() || defaultValue);
      });
    });
  }

  // ── installStartupHook ─────────────────────────

  private async installStartupHook(): Promise<void> {
    const { mkdir, writeFile, readFile } = await import('node:fs/promises');
    const { resolve, join } = await import('node:path');

    const claudeDir = resolve(this.ctx.rootPath, '.claude');
    const scriptsDir = resolve(this.ctx.rootPath, 'scripts');
    const settingsPath = join(claudeDir, 'settings.json');

    // Ensure directories exist
    await mkdir(claudeDir, { recursive: true });
    await mkdir(scriptsDir, { recursive: true });

    // Write the startup script
    const startupScript = this.generateStartupScript();
    const scriptPath = join(scriptsDir, 'hex-startup.sh');
    await writeFile(scriptPath, startupScript, { mode: 0o755 });

    // Read or create settings.json
    let settings: Record<string, unknown> = {};
    try {
      const existing = await readFile(settingsPath, 'utf-8');
      const parsed: unknown = JSON.parse(existing);
      if (typeof parsed === 'object' && parsed !== null) {
        settings = parsed as Record<string, unknown>;
      }
    } catch {
      // Settings file doesn't exist yet — start with empty defaults
    }

    // Add the SessionStart hook if not already present
    const hooksObj = (settings.hooks ?? {}) as Record<string, unknown>;
    const sessionHooks = (hooksObj.SessionStart ?? []) as Array<Record<string, unknown>>;
    const hasStartup = sessionHooks.some((entry) => {
      const innerHooks = (entry.hooks ?? []) as Array<Record<string, unknown>>;
      return innerHooks.some((h) => typeof h.command === 'string' && h.command.includes('hex-startup'));
    });

    if (!hasStartup) {
      sessionHooks.push({
        hooks: [{ type: 'command', command: 'bash scripts/hex-startup.sh', timeout: 10000 }],
      });
      hooksObj.SessionStart = sessionHooks;
      settings.hooks = hooksObj;
      await writeFile(settingsPath, JSON.stringify(settings, null, 2) + '\n');
      this.writeLn('Startup hook installed (.claude/settings.json).');
    }
  }

  private generateStartupScript(): string {
    return [
      '#!/bin/bash',
      '# hex-intf session-start hook — presents project context on first prompt',
      'set -e',
      '',
      '# Only run in hex-intf projects',
      '[ ! -f "PRD.md" ] || [ ! -f "CLAUDE.md" ] && exit 0',
      '',
      'echo ""',
      'echo "=== hex-intf Project ==="',
      'echo ""',
      '',
      '# Extract project info from PRD.md',
      'NAME=$(head -1 PRD.md | sed \'s/^# //\' | sed \'s/ —.*//\')',
      'SUMMARY=$(awk \'/^## Summary/{f=1;next} /^##/{f=0} f && NF && !/^_/\' PRD.md | head -1)',
      'echo "Project: $NAME"',
      '[ -n "$SUMMARY" ] && echo "Goal: $SUMMARY"',
      'echo ""',
      '',
      '# Check pipeline progress',
      'BASE="."',
      '[ -d "backend" ] && BASE="backend"',
      '',
      'status() { [ "$1" -gt 0 ] && echo "done" || echo "todo"; }',
      'count() { find "$1" \\( -name "*.ts" -o -name "*.go" -o -name "*.rs" \\) 2>/dev/null | grep -v gitkeep | wc -l | tr -d " "; }',
      '',
      'D=$(count "$BASE/src/core/domain")',
      'P=$(count "$BASE/src/core/ports")',
      'U=$(count "$BASE/src/core/usecases")',
      'PA=$(count "$BASE/src/adapters/primary")',
      'SA=$(count "$BASE/src/adapters/secondary")',
      'T=$(find "$BASE/tests" -name "*.test.*" -o -name "*_test.*" 2>/dev/null | wc -l | tr -d " ")',
      '',
      'echo "Pipeline:"',
      'echo "  [$(status $D)] Domain ($D)  [$(status $P)] Ports ($P)  [$(status $U)] UseCases ($U)"',
      'echo "  [$(status $PA)] Primary ($PA)  [$(status $SA)] Secondary ($SA)  [$(status $T)] Tests ($T)"',
      'echo ""',
      '',
      '# Suggest next step',
      'if [ "$D" -eq 0 ]; then echo "Next: Define domain entities in $BASE/src/core/domain/"',
      'elif [ "$P" -eq 0 ]; then echo "Next: Define port interfaces in $BASE/src/core/ports/"',
      'elif [ "$U" -eq 0 ]; then echo "Next: Implement use cases in $BASE/src/core/usecases/"',
      'elif [ "$PA" -eq 0 ] && [ "$SA" -eq 0 ]; then echo "Next: Implement adapters"',
      'elif [ "$T" -eq 0 ]; then echo "Next: Add tests"',
      'else echo "Next: Run hex-intf analyze . to validate"',
      'fi',
      'echo "==========================="',
      '',
    ].join('\n');
  }

  // ── initSwarm ──────────────────────────────────

  private async initSwarm(): Promise<void> {
    this.writeLn('Initializing ruflo swarm...');
    try {
      const status = await this.ctx.swarm.init({
        topology: 'hierarchical',
        maxAgents: 5,
        strategy: 'specialized',
        consensus: 'raft',
        memoryNamespace: 'hex-intf',
      });
      this.writeLn(`Swarm initialized: ${status.id} (${status.topology})`);
    } catch (err) {
      this.writeLn(`Swarm init skipped: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  // ── help ────────────────────────────────────────────

  private help(): number {
    this.writeLn('hex-intf - Hexagonal Architecture framework for agentic coding');
    this.writeLn('');
    this.writeLn('Usage: hex-intf <command> [options]');
    this.writeLn('');
    this.writeLn('Commands:');
    this.writeLn('  mcp                             Start as MCP server (stdio transport)');
    this.writeLn('  analyze [path] [--json]         Analyze architecture health');
    this.writeLn('  summarize <file> [--level L]     Print AST summary (L0-L3)');
    this.writeLn('  generate <spec> [--adapter N]    Generate code from a spec file');
    this.writeLn('    [--lang ts|go|rust] [--output path]');
    this.writeLn('  plan <requirements...>           Create a workplan from requirements');
    this.writeLn('    [--lang ts|go|rust]');
    this.writeLn('  setup                           Download grammars + install skills/agents');
    this.writeLn('  dashboard [--port N]             Open web dashboard (default: 3847)');
    this.writeLn('  hub [paths...] [--port N]       Multi-project dashboard broker');
    this.writeLn('  status                          Show swarm progress');
    this.writeLn('  init [--lang ts|go|rust]        Scaffold a hex project');
    this.writeLn('  help                            Show this help');
    this.writeLn('');
    this.writeLn('MCP Server (add to any project):');
    this.writeLn('  # .claude/settings.local.json');
    this.writeLn('  { "mcpServers": { "hex-intf": { "command": "npx", "args": ["hex-intf", "mcp"] } } }');
    this.writeLn('');
    this.writeLn('Examples:');
    this.writeLn('  hex-intf mcp                                 # Start MCP server');
    this.writeLn('  hex-intf analyze ./src --json                # CI-friendly output');
    this.writeLn('  hex-intf generate spec.txt --output src/adapters/primary/api.ts');
    this.writeLn('  hex-intf plan "add caching layer" "implement retry logic"');
    this.writeLn('  hex-intf init --lang ts');

    return 0;
  }

  // ── setup ──────────────────────────────────────────

  private async setup(): Promise<number> {
    this.writeLn('Setting up hex-intf...');
    this.writeLn('');

    const languages = ['typescript', 'go', 'rust'];

    // Install tree-sitter-wasms if not present
    const hasWasms = await this.ctx.fs.exists('node_modules/tree-sitter-wasms/out');
    if (!hasWasms) {
      this.writeLn('Installing tree-sitter WASM grammars...');
      const { execFile: execFileCb } = await import('child_process');
      const { promisify } = await import('util');
      const run = promisify(execFileCb);
      try {
        await run('bun', ['add', 'tree-sitter-wasms'], { cwd: this.ctx.rootPath, timeout: 30000 });
        this.writeLn('  tree-sitter-wasms installed.');
      } catch {
        this.writeLn('  Failed. Run manually: bun add tree-sitter-wasms');
        return 1;
      }
    }

    // Check grammar availability — use absolute paths since grammars
    // may be in hex-intf's own node_modules or the project's
    const { access } = await import('node:fs/promises');
    const { resolve } = await import('node:path');
    // Resolve from: 1) project's config, 2) project's node_modules,
    // 3) hex-intf's own node_modules (for global install via npm link)
    const { dirname } = await import('node:path');
    const cliDir = typeof import.meta.dir === 'string' ? import.meta.dir : dirname(import.meta.url.replace('file://', ''));
    const hexIntfRoot = resolve(cliDir, '..');  // dist/ -> project root
    const checkDirs = [
      resolve(this.ctx.rootPath, 'config/grammars'),
      resolve(this.ctx.rootPath, 'node_modules/tree-sitter-wasms/out'),
      resolve(hexIntfRoot, 'node_modules/tree-sitter-wasms/out'),
    ];

    this.writeLn('');
    this.writeLn('Tree-sitter grammars:');
    for (const lang of languages) {
      let found = false;
      let foundAt = '';
      for (const dir of checkDirs) {
        const fullPath = resolve(dir, `tree-sitter-${lang}.wasm`);
        try {
          await access(fullPath);
          found = true;
          foundAt = fullPath;
          break;
        } catch { /* not here */ }
      }
      this.writeLn(`  ${lang}: ${found ? 'found' : 'not found'}`);
      if (found) this.writeLn(`    ${foundAt}`);
    }

    // Install skills and agents into Claude Code
    const { mkdir, copyFile, readdir } = await import('node:fs/promises');
    const { join } = await import('node:path');

    this.writeLn('');
    this.writeLn('Installing Claude Code skills and agents...');

    const claudeDir = resolve(this.ctx.rootPath, '.claude');
    const skillsTarget = join(claudeDir, 'skills');
    const agentsTarget = join(claudeDir, 'agents', 'hex-intf');

    // Find hex-intf's own skills/ and agents/ directories
    const skillsSrc = resolve(hexIntfRoot, 'skills');
    const agentsSrc = resolve(hexIntfRoot, 'agents');

    try {
      await mkdir(skillsTarget, { recursive: true });
      await mkdir(agentsTarget, { recursive: true });

      // Copy skills
      let skillCount = 0;
      try {
        const skillFiles = await readdir(skillsSrc);
        for (const f of skillFiles) {
          if (f.endsWith('.md') || f.endsWith('.yml') || f.endsWith('.yaml')) {
            await copyFile(join(skillsSrc, f), join(skillsTarget, f));
            skillCount++;
          }
        }
      } catch { /* skills dir may not exist */ }
      this.writeLn(`  Skills: ${skillCount} installed to .claude/skills/`);

      // Copy agent definitions
      let agentCount = 0;
      try {
        const agentFiles = await readdir(agentsSrc);
        for (const f of agentFiles) {
          if (f.endsWith('.yml') || f.endsWith('.yaml')) {
            await copyFile(join(agentsSrc, f), join(agentsTarget, f));
            agentCount++;
          }
        }
      } catch { /* agents dir may not exist */ }
      this.writeLn(`  Agents: ${agentCount} installed to .claude/agents/hex-intf/`);

    } catch (err) {
      this.writeLn(`  Failed to install skills/agents: ${err instanceof Error ? err.message : String(err)}`);
    }

    this.writeLn('');
    this.writeLn('Setup complete. Available commands:');
    this.writeLn('  hex-intf analyze .     Check architecture health');
    this.writeLn('  hex-intf summarize     AST summary of a file');
    this.writeLn('  hex-intf init          Scaffold a new hex project');
    this.writeLn('  hex-intf help          Show all commands');
    return 0;
  }
}
