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
import { formatArchReport } from '../../core/ports/index.js';
import {
  bold, dim, muted, green, cyan, red, yellow, gray,
  header, section, kv,
  didYouMean, elapsed, setColorEnabled, setUnicodeEnabled,
  error as errorColor, warn, accent, glyph, hr,
} from './cli-fmt.js';

/** Colorize a plain-text architecture report with ANSI colors.
 *  Uses cli-fmt helpers already imported above (green, red, yellow, etc.). */
function colorizeReport(report: string, _score: number): string {
  return report
    // Status badges
    .replace(/\bPASS\b/g, green('PASS'))
    .replace(/\bFAIL\b/g, red('FAIL'))
    .replace(/\bWARN\b/g, yellow('WARN'))
    .replace(/\bINFO\b/g, cyan('INFO'))
    .replace(/\bOK\b/g, green('OK'))
    .replace(/\bSKIP\b/g, gray('SKIP'))
    // Severity tags
    .replace(/\[CRITICAL\]/g, red('[CRITICAL]'))
    .replace(/\[WARNING\]/g, yellow('[WARNING]'))
    .replace(/\[INFO\]/g, cyan('[INFO]'))
    // Section headers
    .replace(/^(={60})$/gm, bold('$1'))
    .replace(/^( {2}HEXAGONAL ARCHITECTURE HEALTH REPORT)$/gm, bold('$1'))
    .replace(/^( {2}[A-Z &]+)$/gm, bold('$1'))
    // Grade and score in footer
    .replace(/Score: (\d+)\/100/g, (_: string, s: string) => {
      const n = parseInt(s, 10);
      const colorFn = n >= 80 ? green : n >= 50 ? yellow : red;
      return `Score: ${colorFn(`${s}/100`)}`;
    })
    .replace(/Grade: ([A-F])/g, (_: string, g: string) => {
      const colorFn = g === 'A' || g === 'B' ? green : g === 'C' ? yellow : red;
      return `Grade: ${colorFn(g)}`;
    });
}

/** Result from runCLI — captures output for testing */
interface CLIResult {
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
        postinstall: 'cd node_modules/agentdb/dist && ln -sf src/controllers controllers 2>/dev/null || true',
      },
      dependencies: {
        ruflo: 'latest',
        agentdb: 'latest',
      },
      devDependencies: {
        typescript: '^5.0.0',
        'web-tree-sitter': '^0.24.0',
        'tree-sitter-wasms': '^0.1.0',
      },
    },
    null,
    2,
  ) + '\n',

  gitignore: [
    'node_modules/',
    'dist/',
    '.env',
    '.hex/',
    '*.tsbuildinfo',
    '',
  ].join('\n'),

  readme: [
    '# My Hex Project',
    '',
    'Scaffolded with [hex](https://github.com/your-org/hex).',
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
    '## Architecture Decision Records (ADRs)',
    '',
    'Record significant architectural decisions in `docs/adrs/`. Use ADRs when:',
    '',
    '- Adding a new adapter or port interface',
    '- Choosing a library, framework, or external service',
    '- Changing dependency direction or layer boundaries',
    '- Making trade-offs that affect multiple components',
    '',
    'Format: `docs/adrs/ADR-NNN-short-title.md` (see ADR-001 for template).',
    'ADRs are immutable once accepted — supersede, don\'t edit.',
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
    '7. **Validate** — Run `hex analyze .` to check architecture health',
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
    if (argv.includes('--no-color')) {
      setColorEnabled(false);
    }
    if (argv.includes('--ascii')) {
      setUnicodeEnabled(false);
    }
    const args = parseArgs(argv);

    // Handle --version / version before command dispatch
    if (args.flags.has('version') || args.command === 'version' || args.command === '--version') {
      this.writeLn(`${accent(glyph('\u2b21', '*'))} hex ${bold(this.ctx.version.getCliVersion().toString())}`);
      return 0;
    }

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
        case 'daemon':
          return await this.daemon(args);
        case 'setup':
          return await this.setup();
        case 'init':
          return await this.init(args);
        case 'mcp':
          return await this.mcp();
        case 'projects':
          return await this.projects();
        case 'secrets':
          return await this.secrets(args);
        case 'go':
        case 'build':
          return await this.go(args);
        case 'scaffold':
          return await this.scaffold(args);
        case 'validate':
          return await this.validate(args);
        case 'orchestrate':
          return await this.orchestrate(args);
        case 'compare':
          return await this.compare(args);
        case 'adr':
          return await this.adr(args);
        case 'help':
        case '--help':
        case '-h':
          return this.help();
        default: {
          this.writeLn(`${errorColor('Unknown command:')} ${args.command}`);
          const commands = [
            'analyze', 'summarize', 'generate', 'plan', 'dashboard', 'hub',
            'status', 'daemon', 'setup', 'init', 'mcp', 'projects', 'secrets',
            'go', 'build', 'scaffold', 'validate', 'orchestrate', 'compare', 'adr', 'help',
          ];
          const suggestion = didYouMean(args.command, commands);
          if (suggestion) {
            this.writeLn(`Did you mean ${bold(suggestion)}?`);
          }
          this.writeLn(`${muted('Run')} hex help ${muted('for usage.')}`);
          return 1;
        }
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
    const compactMode = args.flags.has('compact');
    const fullPaths = args.flags.has('full-paths');
    const startTime = Date.now();

    // Guard: refuse to scan directories that don't look like a code project.
    // Globbing **/*.ts from ~ or / would be catastrophically slow and hit
    // permission-protected OS directories (Library/Caches, etc.).
    const resolvedRoot = targetPath === '.' ? this.ctx.rootPath : targetPath;
    const projectMarkers = ['package.json', 'go.mod', 'Cargo.toml', 'pyproject.toml', 'src'];
    const looksLikeProject = await Promise.all(
      projectMarkers.map((m) => this.ctx.fs.exists(
        targetPath === '.' ? m : `${targetPath}/${m}`,
      )),
    );
    if (!looksLikeProject.some(Boolean)) {
      this.writeLn(`${errorColor('Error:')} ${bold(resolvedRoot)} does not look like a project directory.`);
      this.writeLn(`${muted('Expected one of:')} ${projectMarkers.join(', ')}`);
      this.writeLn(`${muted('Run')} hex analyze ${muted('from your project root, or pass the path explicitly.')}`);
      return 1;
    }

    if (!jsonMode) {
      this.writeLn(header());
      this.writeLn(`${muted('Analyzing')} ${bold(targetPath)} ${muted('...')}`);
      this.writeLn('');
    }

    const result = await this.ctx.archAnalyzer.analyzeArchitecture(targetPath);
    const s = result.summary;

    // Machine-readable JSON output for CI/CD pipelines
    if (jsonMode) {
      this.writeLn(JSON.stringify(result, null, 2));
      return s.healthScore >= 50 ? 0 : 1;
    }

    if (this.ctx.astIsStub) {
      this.writeLn(`${glyph('\u26a0', '!')} Running without tree-sitter grammars ${glyph('\u2014', '--')} results may be incomplete`);
      this.writeLn('');
    }

    // Generate the formatted report and add CLI colors
    const report = formatArchReport(result, targetPath, {
      fullPaths,
      showRulesReference: !compactMode,
    });
    this.writeLn(colorizeReport(report, s.healthScore));

    if (!jsonMode) {
      this.writeLn('');
      this.writeLn(`${muted('Completed in')} ${elapsed(Date.now() - startTime)}`);
    }

    return s.healthScore >= 50 ? 0 : 1;
  }

  // ── summarize ───────────────────────────────────────

  private async summarize(args: ParsedArgs): Promise<number> {
    const filePath = args.positional[0];
    if (!filePath) {
      this.writeLn('Usage: hex summarize <file> [--level L0|L1|L2|L3]');
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
      this.writeLn('Tip: Inside Claude Code, use hex_generate via MCP — Claude IS the LLM.');
      return 1;
    }

    const specFile = args.positional[0];
    if (!specFile) {
      this.writeLn('Usage: hex generate <spec-file> [--adapter <name>] [--lang ts|go|rust]');
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
      this.writeLn('Tip: Inside Claude Code, use hex_plan via MCP — Claude IS the LLM.');
      return 1;
    }

    const requirements = args.positional;
    if (requirements.length === 0) {
      this.writeLn('Usage: hex plan <requirements...>');
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

  private async dashboard(_args: ParsedArgs): Promise<number> {
    if (!this.ctx.hubLauncher) { this.writeLn('hex-hub not available'); return 1; }
    const launcher = this.ctx.hubLauncher;

    // Ensure hex-hub is running
    if (!(await launcher.isRunning())) {
      try {
        const result = await launcher.start();
        this.writeLn(`hex-hub started at ${result.url}`);
      } catch (err) {
        this.writeLn(`Error: ${err instanceof Error ? err.message : String(err)}`);
        this.writeLn('Run "hex setup" to build and install the hex-hub binary.');
        return 1;
      }
    }

    // Register project AND push all data (health, tokens, swarm, graph)
    try {
      const { DashboardAdapter } = await import('./dashboard-adapter.js');
      const adapter = new DashboardAdapter(this.ctx);
      this.writeLn('Registering project and pushing architecture data...');

      const { url } = await adapter.startAndPushOnce();
      this.writeLn(`Dashboard: ${url}`);
      this.writeLn('Data pushed. Listening for commands... (Ctrl+C to stop)');

      // Keep process alive to handle WebSocket commands from the hub.
      // Use setInterval (not bare Promise) — Bun busy-waits on unresolved promises.
      await new Promise<void>((resolve) => {
        const keepAlive = setInterval(() => {}, 60_000);
        const onSignal = () => { clearInterval(keepAlive); adapter.stop(); resolve(); };
        process.on('SIGINT', onSignal);
        process.on('SIGTERM', onSignal);
      });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      this.writeLn(`Dashboard data push failed: ${msg}`);
      this.writeLn('Dashboard: http://127.0.0.1:5555 (connected without data)');
    }

    return 0;
  }

  // ── hub (Rust hex-hub binary management) ────────────

  private async hub(args: ParsedArgs): Promise<number> {
    // HEX_DAEMON=1 or --daemon: start hex-hub as background daemon
    if (process.env['HEX_DAEMON'] === '1' || args.flags.has('daemon')) {
      if (!this.ctx.hubLauncher) { this.writeLn('hex-hub not available'); return 1; }
      const launcher = this.ctx.hubLauncher;
      try {
        const { started, url } = await launcher.start();
        this.writeLn(started ? `hex-hub daemon at ${url}` : `hex-hub already running at ${url}`);
        return 0;
      } catch (err) {
        this.writeLn(`Error: ${err instanceof Error ? err.message : String(err)}`);
        return 1;
      }
    }

    const subCmd = args.positional[0] ?? 'status';
    if (!this.ctx.hubLauncher) { this.writeLn('hex-hub not available'); return 1; }
    const launcher = this.ctx.hubLauncher;

    switch (subCmd) {
      case 'start': {
        const token = args.flags.get('token') ?? process.env['HEX_DASHBOARD_TOKEN'];
        try {
          const { started, url } = await launcher.start(token);
          if (started) {
            this.writeLn(`hex-hub started at ${url}`);
          } else {
            this.writeLn(`hex-hub already running at ${url}`);
          }
          return 0;
        } catch (err) {
          this.writeLn(`Error: ${err instanceof Error ? err.message : String(err)}`);
          return 1;
        }
      }
      case 'stop': {
        const stopped = await launcher.stop();
        this.writeLn(stopped ? 'hex-hub stopped.' : 'hex-hub was not running.');
        return stopped ? 0 : 1;
      }
      case 'status': {
        const status = await launcher.status();
        if (status.running) {
          this.writeLn(`hex-hub: running at ${status.url}`);
          this.writeLn(`Projects: ${status.projects} registered`);
          // Show version info
          try {
            const info = await this.ctx.version.getVersionInfo();
            this.writeLn(`  CLI:  ${info.cli}`);
            if (info.hub) this.writeLn(`  Hub:  ${info.hub}${info.hubBinaryPath ? ` (${info.hubBinaryPath})` : ''}`);
            if (info.mismatch) this.writeLn(`  Warning: version mismatch — run "hex setup" to rebuild hex-hub`);
          } catch { /* version check is best-effort */ }
        } else {
          this.writeLn('hex-hub: not running');
          const binary = launcher.findBinary();
          if (binary) {
            this.writeLn(`Binary: ${binary}`);
            this.writeLn('Start with: hex hub start');
          } else {
            this.writeLn('Binary: not found — run "hex setup" to install');
          }
        }
        return 0;
      }
      default:
        this.writeLn(`Unknown hub command: ${subCmd}`);
        this.writeLn('Usage: hex hub [start|stop|status]');
        return 1;
    }
  }

  /**
   * Start the dashboard hub on fixed port 5555.
   * Used by both `dashboard` and `hub` commands.
   */
  private async startHub(_args: ParsedArgs, isDaemon: boolean): Promise<number> {
    if (!this.ctx.hubLauncher) { this.writeLn('hex-hub not available'); return 1; }
    const launcher = this.ctx.hubLauncher;

    const token = isDaemon ? (process.env['HEX_DAEMON_TOKEN'] ?? undefined) : undefined;
    const { started, url } = await launcher.start(token);

    if (started) {
      this.writeLn(`hex-hub started at ${url}`);
    } else {
      this.writeLn(`hex-hub already running at ${url}`);
    }
    this.writeLn(`Projects push data to this hub on port 5555.`);

    // If running as daemon, write lock file
    if (isDaemon) {
      const { DaemonManager } = await import('./daemon-manager.js');
      const daemon = new DaemonManager();
      daemon.registerSelf(token ?? '', '1.0.0');

      process.on('SIGTERM', () => { daemon.unregisterSelf(); process.exit(0); });
      process.on('SIGINT', () => { daemon.unregisterSelf(); process.exit(0); });
    } else {
      this.writeLn('Press Ctrl+C to stop.');
    }

    await new Promise(() => {});
    return 0;
  }

  // ── daemon ──────────────────────────────────────────

  private async daemon(args: ParsedArgs): Promise<number> {
    const { DaemonManager } = await import('./daemon-manager.js');
    const daemon = new DaemonManager();
    const subCmd = args.positional[0] ?? 'status';

    switch (subCmd) {
      case 'status': {
        const status = await daemon.status();
        if (status.running) {
          this.writeLn(`Dashboard daemon running`);
          this.writeLn(`  PID:      ${status.pid}`);
          this.writeLn(`  Port:     ${status.port}`);
          this.writeLn(`  Uptime:   ${Math.round((status.uptime ?? 0) / 1000)}s`);
          this.writeLn(`  URL:      http://localhost:${status.port}`);
        } else {
          this.writeLn('Dashboard daemon is not running.');
          this.writeLn('Start with: hex daemon start');
        }
        return 0;
      }

      case 'start': {
        const status = await daemon.status();
        if (status.running) {
          this.writeLn(`Already running at http://localhost:${status.port} (PID ${status.pid})`);
          return 0;
        }

        // Spawn detached daemon using this same CLI
        const entryPath = process.argv[1];
        const result = await daemon.findOrStart(entryPath);
        this.writeLn(`Dashboard daemon started at http://localhost:${result.port}`);
        return 0;
      }

      case 'stop': {
        const stopped = await daemon.stop();
        this.writeLn(stopped ? 'Dashboard daemon stopped.' : 'No daemon running.');
        return 0;
      }

      case 'logs': {
        const { readFileSync } = await import('node:fs');
        try {
          const log = readFileSync(daemon.paths.log, 'utf-8');
          const lines = log.split('\n').slice(-50);
          this.writeLn(lines.join('\n'));
        } catch {
          this.writeLn('No logs found.');
        }
        return 0;
      }

      default:
        this.writeLn(`Unknown daemon command: ${subCmd}`);
        this.writeLn('Usage: hex daemon [status|start|stop|logs]');
        return 1;
    }
  }

  // ── status ──────────────────────────────────────────

  private async status(): Promise<number> {
    // Version info
    try {
      const info = await this.ctx.version.getVersionInfo();
      this.writeLn(header());
      this.writeLn(kv('Version', `${info.cli}${info.hub ? ` ${muted('|')} Hub ${info.hub}${info.mismatch ? ` ${warn('(outdated)')}` : ''}` : ''}`));
    } catch { /* best-effort */ }

    const { DaemonManager } = await import('./daemon-manager.js');
    const daemon = new DaemonManager();
    const daemonStatus = await daemon.status();

    if (daemonStatus.running) {
      this.writeLn(`Dashboard:  http://localhost:${daemonStatus.port} (PID ${daemonStatus.pid})`);
      this.writeLn(`Uptime:     ${Math.round((daemonStatus.uptime ?? 0) / 1000)}s`);
    } else {
      this.writeLn(kv('Dashboard', `${muted('not running')} ${dim('(start with: hex dashboard)')}`));
    }

    try {
      const progress = await this.ctx.swarm.getProgressReport();
      this.writeLn(`Tasks:      ${progress.tasks.length} (${progress.overallPercent}% complete)`);
      this.writeLn(`Agents:     ${progress.agents.length}`);
      this.writeLn(`Patterns:   ${progress.patterns.total} (${progress.patterns.recentlyUsed} recently used)`);
    } catch {
      this.writeLn(kv('Swarm', muted('unavailable')));
    }

    return 0;
  }

  // ── mcp ────────────────────────────────────────────
  // Starts hex as a stdio MCP server so any project can use it:
  //   npx hex mcp
  // Or in .claude/settings.local.json:
  //   { "mcpServers": { "hex": { "command": "npx", "args": ["hex", "mcp"] } } }

  private async mcp(): Promise<number> {
    const { MCPAdapter } = await import('./mcp-adapter.js');

    const adapter = new MCPAdapter({
      archAnalyzer: this.ctx.archAnalyzer,
      ast: this.ctx.ast,
      fs: this.ctx.fs,
      codeGenerator: this.ctx.codeGenerator,
      workplanExecutor: this.ctx.workplanExecutor,
      swarmOrchestrator: this.ctx.swarmOrchestrator,
      scaffold: this.ctx.scaffold,
      createDashboard: this.ctx.createDashboard,
    });

    const hexTools = adapter.getTools();

    // ── Embed claude-flow tools (single MCP server for everything) ──
    // claude-flow exports a programmatic API — no subprocess needed.
    // hex is the agentic coding harness; claude-flow is an implementation detail.
    type FlowToolDef = { name: string; description: string; inputSchema: Record<string, unknown> };
    let flowTools: FlowToolDef[] = [];
    let callFlowTool: ((name: string, input: Record<string, unknown>) => Promise<unknown>) | null = null;

    try {
      const flow = await import('@claude-flow/cli/dist/src/mcp-client.js');
      const rawTools = flow.listMCPTools?.() ?? [];
      flowTools = rawTools.map((t: any) => ({
        name: t.name,
        description: t.description ?? '',
        inputSchema: t.inputSchema ?? { type: 'object', properties: {}, required: [] },
      }));
      callFlowTool = flow.callMCPTool?.bind(flow) ?? null;
      process.stderr.write(`[hex] claude-flow embedded: ${flowTools.length} tools\n`);
    } catch {
      process.stderr.write(`[hex] claude-flow not available — hex tools only\n`);
    }

    // Merge all tools: hex tools first, then claude-flow tools
    const allTools = [
      ...hexTools.map((t) => ({ name: t.name, description: t.description, inputSchema: t.inputSchema })),
      ...flowTools,
    ];

    // Build a set of hex tool names for routing
    const hexToolNames = new Set(hexTools.map((t) => t.name));

    // JSON-RPC stdio MCP server — reads from stdin, writes to stdout
    const readline = await import('node:readline');
    const rl = readline.createInterface({ input: process.stdin });

    const send = (msg: unknown) => {
      process.stdout.write(JSON.stringify(msg) + '\n');
    };

    process.stderr.write(`[hex] MCP server started with ${allTools.length} tools (${hexTools.length} hex + ${flowTools.length} flow)\n`);

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
              serverInfo: { name: 'hex', version: '1.0.0' },
            },
          });
          break;

        case 'notifications/initialized':
          // Client ack — no response needed
          break;

        case 'tools/list':
          send({
            jsonrpc: '2.0', id,
            result: { tools: allTools },
          });
          break;

        case 'tools/call': {
          const toolName = (params as any)?.name as string;
          const toolArgs = (params as any)?.arguments as Record<string, unknown> ?? {};

          try {
            if (hexToolNames.has(toolName)) {
              // Route to hex adapter
              const result = await adapter.handleToolCall({ name: toolName, arguments: toolArgs });
              send({ jsonrpc: '2.0', id, result: { content: result.content, isError: result.isError } });
            } else if (callFlowTool) {
              // Route to embedded claude-flow
              const result = await callFlowTool(toolName, toolArgs);
              // Normalize claude-flow result to MCP format
              const content = typeof result === 'object' && result !== null && 'content' in result
                ? (result as any).content
                : [{ type: 'text', text: typeof result === 'string' ? result : JSON.stringify(result) }];
              send({ jsonrpc: '2.0', id, result: { content } });
            } else {
              send({ jsonrpc: '2.0', id, error: { code: -32601, message: `Unknown tool: ${toolName}` } });
            }
          } catch (err) {
            const message = err instanceof Error ? err.message : String(err);
            send({ jsonrpc: '2.0', id, result: { content: [{ type: 'text', text: `Error: ${message}` }], isError: true } });
          }
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

    // ── Docs directory (ADRs + specs) ──────────────────

    const adrTemplate = [
      '# ADR-001: Hexagonal Architecture',
      '',
      '**Status:** Accepted',
      `**Date:** ${new Date().toISOString().split('T')[0]}`,
      '',
      '## Context',
      '',
      'This project uses hexagonal architecture (ports & adapters) to enforce',
      'clean dependency boundaries and enable adapter swappability.',
      '',
      '## Decision',
      '',
      '- Domain layer has zero external dependencies',
      '- All cross-boundary communication flows through typed port interfaces',
      '- Adapters are the only layer that touches external systems',
      '- composition-root is the single wiring point',
      '',
      '## Consequences',
      '',
      '- New features decompose inside-out: domain → ports → adapters',
      '- Testing is straightforward: mock ports, test use cases in isolation',
      '- Adapter swaps (e.g., DB migration) require zero domain changes',
      '',
      '## ADR Template',
      '',
      'When making architectural decisions, copy this file and fill in:',
      '',
      '```',
      '# ADR-NNN: Title',
      '',
      '**Status:** Proposed | Accepted | Deprecated | Superseded',
      '**Date:** YYYY-MM-DD',
      '',
      '## Context',
      'What is the issue? What forces are at play?',
      '',
      '## Decision',
      'What did we decide? Why?',
      '',
      '## Consequences',
      'What are the trade-offs? What becomes easier or harder?',
      '```',
      '',
    ].join('\n');

    if (isMultiStack) {
      await safeWrite('docs/adrs/ADR-001-hexagonal-architecture.md', adrTemplate);
      await safeWrite('docs/specs/.gitkeep', '');
    } else {
      await safeWrite('docs/adrs/ADR-001-hexagonal-architecture.md', adrTemplate);
      await safeWrite('docs/specs/.gitkeep', '');
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
        '7. **Validate** — Run `hex analyze .` to check architecture health',
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
      this.writeLn('No registered projects. Run "hex init" in a project directory.');
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
    this.writeLn(`${hr(3)} hex project setup ${hr(23)}`);
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
      `- **Scaffolded by:** hex`,
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
      '4. Run `hex analyze .` to validate architecture',
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
      '# hex session-start hook — presents project context on first prompt',
      'set -e',
      '',
      '# Only run in hex projects',
      '[ ! -f "PRD.md" ] || [ ! -f "CLAUDE.md" ] && exit 0',
      '',
      'echo ""',
      'echo "=== hex Project ==="',
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
      'else echo "Next: Run hex analyze . to validate"',
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
        memoryNamespace: 'hex',
      });
      this.writeLn(`Swarm initialized: ${status.id} (${status.topology})`);
    } catch (err) {
      this.writeLn(`Swarm init skipped: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  // ── help ────────────────────────────────────────────

  // ── secrets ─────────────────────────────────────────

  private async secrets(args: ParsedArgs): Promise<number> {
    const subCmd = args.positional[0] ?? 'status';

    switch (subCmd) {
      case 'status':
        return this.secretsStatus();
      case 'list':
        return this.secretsList(args);
      case 'init':
        return await this.secretsInit(args);
      case 'set':
        return await this.secretsSet(args);
      case 'get':
        return await this.secretsGet(args);
      case 'remove':
      case 'rm':
        return await this.secretsRemove(args);
      default:
        this.writeLn(`Unknown secrets command: ${subCmd}`);
        this.writeLn('Usage: hex secrets <command>');
        this.writeLn('');
        this.writeLn('Commands:');
        this.writeLn('  status              Show secrets backend info');
        this.writeLn('  list [--json]       List secret keys (no values)');
        this.writeLn('  init                Create a local encrypted vault (prompts for password)');
        this.writeLn('  set <key> [value]   Add or update a secret (prompts if value omitted)');
        this.writeLn('  get <key>           Retrieve a secret value');
        this.writeLn('  remove <key>        Remove a secret from the vault');
        return 1;
    }
  }

  private async secretsStatus(): Promise<number> {
    // Determine backend type by probing listSecrets behavior
    const adapter = await this.resolveSecretsAdapter();
    const metadata = await adapter.listSecrets();

    if (metadata.length === 0) {
      // EnvSecretsAdapter always returns [] — check if it can resolve anything
      // to distinguish "env backend" from "Infisical with zero secrets"
      const hasEnvMarker = await adapter.hasSecret('PATH');
      if (hasEnvMarker) {
        // Environment variable backend (can see PATH)
        this.writeLn('Backend:     Environment variables (read-only)');
        this.writeLn('');
        this.writeLn('No vault configured. To manage secrets locally:');
        this.writeLn('');
        this.writeLn('  hex secrets init                    Create encrypted vault');
        this.writeLn('  hex secrets set <key>               Store a secret (prompts for value)');
        this.writeLn('  hex secrets get <key>               Retrieve a secret');
        this.writeLn('  hex secrets list                    List all keys');
        this.writeLn('');
        this.writeLn('Set HEX_VAULT_PASSWORD env var for subsequent operations.');
        return 0;
      }
    }

    // Infisical or local vault backend — has metadata
    // Detect local vault vs Infisical by checking environment field
    const isLocal = metadata.length > 0 && metadata[0].environment === 'local';
    this.writeLn(`Backend:     ${isLocal ? 'Local vault' : 'Infisical'}`);
    this.writeLn(`Secrets:     ${metadata.length} keys accessible`);
    return 0;
  }

  private async secretsList(args: ParsedArgs): Promise<number> {
    const jsonMode = args.flags.has('json');
    const adapter = await this.resolveSecretsAdapter();
    const metadata = await adapter.listSecrets();

    if (metadata.length === 0) {
      // Check if this is the env adapter (no metadata support)
      const hasEnvMarker = await adapter.hasSecret('PATH');
      if (hasEnvMarker) {
        if (jsonMode) {
          this.writeLn(JSON.stringify([]));
        } else {
          this.writeLn('No vault configured. Create one with:');
          this.writeLn('');
          this.writeLn('  hex secrets init');
        }
        return 0;
      }
    }

    if (jsonMode) {
      this.writeLn(JSON.stringify(metadata, null, 2));
      return 0;
    }

    if (metadata.length === 0) {
      this.writeLn('No secrets found.');
      return 0;
    }

    // Table header
    this.writeLn('Key                            Env       Version  Updated');
    this.writeLn(hr(72));
    for (const m of metadata) {
      const key = m.key.padEnd(30);
      const env = m.environment.padEnd(9);
      const ver = String(m.version).padEnd(8);
      this.writeLn(`${key} ${env} ${ver} ${m.updatedAt}`);
    }
    this.writeLn('');
    this.writeLn(`Total: ${metadata.length} secrets`);

    return 0;
  }

  private async secretsInit(args: ParsedArgs): Promise<number> {
    const { resolve } = await import('node:path');
    const { existsSync } = await import('node:fs');

    const vaultPath = resolve(this.ctx.rootPath, '.hex/vault.enc');

    if (existsSync(vaultPath)) {
      this.writeLn(`Vault already exists at ${vaultPath}`);
      this.writeLn('To reset, delete the file and run this command again.');
      return 1;
    }

    // Password priority: --password flag > env var > interactive prompt
    let password = args.flags.get('password')
      ?? process.env['HEX_VAULT_PASSWORD'];
    if (!password) {
      password = await this.readSecret('Vault password: ');
      if (!password) {
        this.writeLn('Error: vault password required.');
        return 1;
      }
      // Confirm password on interactive input
      const confirm = await this.readSecret('Confirm password: ');
      if (password !== confirm) {
        this.writeLn('Error: passwords do not match.');
        return 1;
      }
    }

    const { mkdir } = await import('node:fs/promises');
    await mkdir(resolve(this.ctx.rootPath, '.hex'), { recursive: true }).catch(() => {});

    this.ctx.vaultManager.createVault(vaultPath, password);

    // Write secrets config to use local vault
    const configPath = resolve(this.ctx.rootPath, '.hex/secrets.json');
    const { writeFileSync } = await import('node:fs');
    writeFileSync(configPath, JSON.stringify({ backend: 'local-vault', localVault: { path: vaultPath } }, null, 2));

    this.writeLn(`Vault created at ${vaultPath}`);
    this.writeLn('Config written to .hex/secrets.json');
    this.writeLn('');
    this.writeLn('Usage:');
    this.writeLn('  hex secrets set ANTHROPIC_API_KEY sk-ant-...');
    this.writeLn('  hex secrets get ANTHROPIC_API_KEY');
    this.writeLn('  hex secrets list');
    return 0;
  }

  private async secretsSet(args: ParsedArgs): Promise<number> {
    const key = args.positional[1];
    if (!key) {
      this.writeLn('Usage: hex secrets set <key> [value]');
      this.writeLn('If value is omitted, it will be read from stdin (hidden input).');
      return 1;
    }

    // Read value from arg or stdin (avoids secrets in shell history)
    let value = args.positional[2];
    if (!value) {
      value = await this.readSecret(`Value for ${key}: `);
      if (!value) {
        this.writeLn('Error: secret value required.');
        return 1;
      }
    }

    const vault = this.resolveVaultManager();
    vault.addSecret(key, value);
    this.writeLn(`Secret "${key}" saved.`);
    return 0;
  }

  private async secretsGet(args: ParsedArgs): Promise<number> {
    const key = args.positional[1];
    if (!key) {
      this.writeLn('Usage: hex secrets get <key>');
      return 1;
    }

    const adapter = await this.resolveSecretsAdapter();
    const result = await adapter.resolveSecret(key);
    if (!result.ok) {
      this.writeLn(`Secret "${key}" not found.`);
      return 1;
    }

    // Print the value (user explicitly asked for it)
    this.writeLn(result.value);
    return 0;
  }

  private async secretsRemove(args: ParsedArgs): Promise<number> {
    const key = args.positional[1];
    if (!key) {
      this.writeLn('Usage: hex secrets remove <key>');
      return 1;
    }

    const vault = this.resolveVaultManager();
    const exists = await this.ctx.secrets.hasSecret(key);
    if (!exists) {
      this.writeLn(`Secret "${key}" not found.`);
      return 1;
    }

    vault.removeSecret(key);
    this.writeLn(`Secret "${key}" removed.`);
    return 0;
  }

  // ── adr ────────────────────────────────────────────

  private async adr(args: ParsedArgs): Promise<number> {
    if (!this.ctx.adrQuery) {
      this.writeLn('ADR tracking not available.');
      return 1;
    }
    const subCmd = args.positional[0] ?? 'list';

    switch (subCmd) {
      case 'list': {
        const statusFilter = args.flags.get('status');
        const entries = await this.ctx.adrQuery.list(statusFilter);
        if (entries.length === 0) {
          this.writeLn('No ADRs found.');
          return 0;
        }
        for (const e of entries) {
          const statusTag = e.status === 'accepted' ? green(e.status)
            : e.status === 'proposed' ? warn(e.status)
            : muted(e.status);
          this.writeLn(`  ${bold(e.id)}  ${statusTag}  ${e.title}`);
        }
        this.writeLn(`${muted(`\n${entries.length} ADR(s)`)}`);
        return 0;
      }

      case 'status': {
        const id = args.positional[1];
        if (!id) {
          this.writeLn('Usage: hex adr status <ADR-ID>');
          return 1;
        }
        const entry = await this.ctx.adrQuery.status(id.toUpperCase());
        if (!entry) {
          this.writeLn(`ADR "${id}" not found.`);
          return 1;
        }
        this.writeLn(kv(bold(entry.id), entry.title));
        this.writeLn(kv('Status', entry.status));
        this.writeLn(kv('Date', entry.date || muted('unknown')));
        this.writeLn(kv('File', entry.filePath));
        if (entry.sections.length > 0) {
          this.writeLn(kv('Sections', entry.sections.join(', ')));
        }
        if (entry.linkedFeatures.length > 0) {
          this.writeLn(kv('Features', entry.linkedFeatures.join(', ')));
        }
        return 0;
      }

      case 'search': {
        const query = args.positional.slice(1).join(' ') || args.flags.get('query') || '';
        if (!query) {
          this.writeLn('Usage: hex adr search <query>');
          return 1;
        }
        const results = await this.ctx.adrQuery.search(query);
        if (results.length === 0) {
          this.writeLn(`No ADRs matching "${query}".`);
          return 0;
        }
        for (const e of results) {
          this.writeLn(`  ${bold(e.id)}  ${muted(e.status)}  ${e.title}`);
        }
        return 0;
      }

      case 'abandoned': {
        const days = parseInt(args.flags.get('days') ?? '14', 10);
        const reports = await this.ctx.adrQuery.findAbandoned(days);
        if (reports.length === 0) {
          this.writeLn(green('No abandoned ADRs found.'));
          return 0;
        }
        for (const r of reports) {
          const age = r.daysSinceModified < 0 ? 'unknown' : `${r.daysSinceModified}d`;
          const rec = r.recommendation === 'close' ? errorColor(r.recommendation)
            : r.recommendation === 'review' ? warn(r.recommendation)
            : muted(r.recommendation);
          this.writeLn(`  ${bold(r.adrId)}  ${muted(age)} old  worktree:${muted(r.linkedWorktreeStatus)}  ${rec}`);
        }
        this.writeLn(`${muted(`\n${reports.length} ADR(s) need attention`)}`);
        return 0;
      }

      case 'reindex': {
        this.writeLn('Re-indexing ADRs into AgentDB...');
        const result = await this.ctx.adrQuery.reindex();
        this.writeLn(`Indexed: ${result.indexed}, Errors: ${result.errors}`);
        return result.errors > 0 ? 1 : 0;
      }

      default:
        this.writeLn(`Unknown adr command: ${subCmd}`);
        this.writeLn('Usage: hex adr <list|status|search|abandoned|reindex>');
        return 1;
    }
  }

  /**
   * Resolve the best available secrets adapter for read operations.
   * If a local-vault config exists, prompts for password (or reads HEX_VAULT_PASSWORD)
   * so that list/get/status work without pre-setting the env var.
   * Falls back to this.ctx.secrets (the startup-wired adapter) otherwise.
   */
  private async resolveSecretsAdapter(): Promise<import('../../core/ports/secrets.js').ISecretsPort> {
    const { resolve } = await import('node:path');
    const { existsSync, readFileSync } = await import('node:fs');

    const configPath = resolve(this.ctx.rootPath, '.hex/secrets.json');
    if (!existsSync(configPath)) {
      return this.ctx.secrets;
    }

    try {
      const config = JSON.parse(readFileSync(configPath, 'utf-8'));
      if (config.backend !== 'local-vault') {
        return this.ctx.secrets;
      }
    } catch {
      return this.ctx.secrets;
    }

    // If vault is open, this.ctx.secrets IS the LocalVaultAdapter (wired by composition-root)
    return this.ctx.secrets;
  }

  /** Resolve the vault manager from AppContext */
  private resolveVaultManager(): AppContext['vaultManager'] {
    return this.ctx.vaultManager;
  }

  /**
   * Read a secret from stdin with echo disabled.
   * Returns empty string if stdin is not a TTY (e.g., piped input).
   */
  private async readSecret(prompt: string): Promise<string> {
    // If not a TTY (piped, CI, etc.), read a line from stdin without prompting
    if (!process.stdin.isTTY) {
      return this.readLineFromStdin();
    }

    const { createInterface } = await import('node:readline');
    return new Promise<string>((resolve) => {
      // Write prompt to stderr so it doesn't mix with stdout output
      process.stderr.write(prompt);

      // Disable echo for password input
      if (process.stdin.setRawMode) {
        process.stdin.setRawMode(true);
      }
      process.stdin.resume();

      let input = '';
      const onData = (chunk: Buffer) => {
        const char = chunk.toString();
        // Enter
        if (char === '\n' || char === '\r' || char === '\u0004') {
          process.stderr.write('\n');
          if (process.stdin.setRawMode) {
            process.stdin.setRawMode(false);
          }
          process.stdin.pause();
          process.stdin.removeListener('data', onData);
          resolve(input);
        // Backspace
        } else if (char === '\u007F' || char === '\b') {
          if (input.length > 0) {
            input = input.slice(0, -1);
            process.stderr.write('\b \b');
          }
        // Ctrl+C
        } else if (char === '\u0003') {
          process.stderr.write('\n');
          if (process.stdin.setRawMode) {
            process.stdin.setRawMode(false);
          }
          process.stdin.pause();
          process.stdin.removeListener('data', onData);
          resolve('');
        } else {
          input += char;
          process.stderr.write('*');
        }
      };

      process.stdin.on('data', onData);
    });
  }

  private async readLineFromStdin(): Promise<string> {
    // Non-TTY with no piped data — return empty immediately
    // This prevents hanging in tests and non-interactive environments
    if (!process.stdin.readable || process.stdin.readableEnded) {
      return '';
    }
    const { createInterface } = await import('node:readline');
    const rl = createInterface({ input: process.stdin });
    return new Promise<string>((resolve) => {
      const timeout = setTimeout(() => { rl.close(); resolve(''); }, 100);
      rl.once('line', (line) => {
        clearTimeout(timeout);
        rl.close();
        resolve(line.trim());
      });
      rl.once('close', () => { clearTimeout(timeout); resolve(''); });
    });
  }

  // ── go ──────────────────────────────────────────────

  private async go(args: ParsedArgs): Promise<number> {
    const prompt = args.positional.join(' ').trim();
    if (!prompt) {
      this.writeLn('Usage: hex go <prompt> [--yolo] [--review] [--no-worktree]');
      this.writeLn('');
      this.writeLn('Autonomous coding — give hex a task and let it build.');
      this.writeLn('');
      this.writeLn('Options:');
      this.writeLn('  --yolo          Auto-merge on PASS (no confirmation)');
      this.writeLn('  --review        Pause after each phase for approval');
      this.writeLn('  --no-worktree   Work directly on current branch (dangerous)');
      this.writeLn('');
      this.writeLn('Examples:');
      this.writeLn('  hex go "add user authentication with JWT"');
      this.writeLn('  hex go "refactor the payment module" --review');
      this.writeLn('  hex go "fix the flaky timeout in tests" --yolo');
      return 1;
    }

    const yolo = args.flags.has('yolo');
    const review = args.flags.has('review');
    const noWorktree = args.flags.has('no-worktree');
    const dryRun = args.flags.has('dry-run');

    // Generate session identifiers
    const timestamp = Date.now();
    const slug = prompt
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 40);
    const branchName = `hex-go/${slug}`;
    const worktreeDir = `${this.ctx.rootPath}/../hex-go-${timestamp}`;

    // ── Phase 1: Worktree isolation ──
    let workDir = this.ctx.rootPath;
    if (!noWorktree) {
      // Try the base branch name first, then a timestamped variant on collision
      const branchCandidates = [branchName, `${branchName}-${timestamp}`];
      let worktreeCreated = false;

      for (const candidate of branchCandidates) {
        this.writeLn(`[hex go] Creating worktree: ${candidate}`);
        try {
          const wt = await this.ctx.worktree.create(candidate);
          workDir = wt.absolutePath;
          this.writeLn(`[hex go] Isolated at: ${workDir}`);
          worktreeCreated = true;
          break;
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          if (msg.includes('already exists') && candidate === branchName) {
            this.writeLn(`[hex go] Branch exists, trying timestamped variant...`);
            continue;
          }
          this.writeLn(`[hex go] Worktree failed: ${msg}`);
          break;
        }
      }

      if (!worktreeCreated) {
        this.writeLn('[hex go] ERROR: Could not create worktree. Clean stale worktrees with:');
        this.writeLn(`  git worktree remove --force <path> && git branch -D ${branchName}`);
        this.writeLn('[hex go] Or use --no-worktree to work directly (not recommended)');
        dashboard?.stop();
        return 1;
      }
    } else {
      this.writeLn('[hex go] --no-worktree: working directly on current branch');
    }

    // ── Phase 2: Build context for the agent ──
    const projectName = this.ctx.rootPath.split('/').pop() ?? 'project';
    let portsContext = '';
    try {
      // Use L1 tree-sitter summaries for ALL port files — 5-10x more coverage
      // than raw-reading one file. L1 gives signatures without bodies (~6% tokens).
      const portGlobs = await Promise.all([
        this.ctx.fs.glob('src/core/ports/**/*.ts'),
        this.ctx.fs.glob('src/core/ports/**/*.go'),
        this.ctx.fs.glob('src/core/ports/**/*.rs'),
        this.ctx.fs.glob('internal/ports/**/*.go'),
        this.ctx.fs.glob('pkg/**/*.go'),
      ]);
      const portFiles = portGlobs.flat().filter(f => !f.includes('.test.'));
      const portSections: string[] = [];

      for (const pf of portFiles) {
        try {
          const summary = await this.ctx.summaryService.summarizeFile(pf, 'L1');
          const exports = summary.exports
            .map(e => `  ${e.kind} ${e.name}${e.signature ? `: ${e.signature}` : ''}`)
            .join('\n');
          if (exports.trim()) {
            portSections.push(`### ${pf}\n${exports}`);
          }
        } catch { /* skip unparseable files */ }
      }

      portsContext = portSections.join('\n\n');

      // Fallback: if tree-sitter found nothing (no grammars), use raw read
      if (!portsContext) {
        const raw = await this.ctx.fs.read('src/core/ports/index.ts');
        portsContext = raw.slice(0, 3000);
      }
    } catch { /* ports files may not exist in target project */ }

    // L0 project overview — full file list (~2% tokens, gives structural awareness)
    let projectOverview = '';
    try {
      const allSourceGlobs = await Promise.all([
        this.ctx.fs.glob('src/**/*.ts'),
        this.ctx.fs.glob('src/**/*.go'),
        this.ctx.fs.glob('src/**/*.rs'),
        this.ctx.fs.glob('internal/**/*.go'),
        this.ctx.fs.glob('cmd/**/*.go'),
      ]);
      const allSourceFiles = allSourceGlobs.flat()
        .filter(f => !f.includes('node_modules') && !f.includes('dist') && !f.includes('.test.'));
      if (allSourceFiles.length > 0) {
        projectOverview = allSourceFiles.map(f => `- ${f}`).join('\n');
      }
    } catch { /* no source files found */ }

    let claudeMd = '';
    try {
      claudeMd = await this.ctx.fs.read('CLAUDE.md');
    } catch { /* no CLAUDE.md */ }

    // Resolve secrets for the agent environment
    let secretsInfo = 'Secrets: env-var fallback (no .hex/secrets.json)';
    try {
      const hasInfisical = await this.ctx.secrets.hasSecret('ANTHROPIC_API_KEY');
      if (hasInfisical) {
        secretsInfo = 'Secrets: configured and available';
      }
    } catch { /* secrets check failed */ }

    const mode = yolo ? 'YOLO (auto-merge on PASS)'
      : review ? 'REVIEW (pause after each phase)'
      : 'DEFAULT (confirm before merge)';

    // ── Connect to dashboard hub (best-effort) ──
    let dashboard: { broadcast: (event: string, data: unknown) => void; stop: () => void } | null = null;
    try {
      const { DashboardAdapter } = await import('./dashboard-adapter.js');
      const adapter = new DashboardAdapter(this.ctx);
      await adapter.start();
      dashboard = { broadcast: (e, d) => adapter.broadcast(e, d), stop: () => adapter.stop() };
      dashboard.broadcast('go-started', { prompt, mode, branch: branchName, workDir, timestamp });
      this.writeLn('[hex go] Dashboard connected — pushing events to hub');
    } catch (dashErr) {
      const msg = dashErr instanceof Error ? dashErr.message : String(dashErr);
      this.writeLn(`[hex go] Dashboard unavailable: ${msg} — running without live updates`);
    }

    this.writeLn('');
    this.writeLn('+-------------------------------------------+');
    this.writeLn('|  hex go -- autonomous coding              |');
    this.writeLn('+-------------------------------------------+');
    this.writeLn(`|  Prompt:  ${prompt.slice(0, 30).padEnd(30)} |`);
    this.writeLn(`|  Mode:    ${mode.slice(0, 30).padEnd(30)} |`);
    this.writeLn(`|  Branch:  ${(noWorktree ? '(current)' : branchName).slice(0, 30).padEnd(30)} |`);
    this.writeLn(`|  ${secretsInfo.slice(0, 39).padEnd(39)} |`);
    this.writeLn('+-------------------------------------------+');
    this.writeLn('');

    // Dry-run: stop after setup (for testing and previewing)
    if (dryRun) {
      this.writeLn('[hex go] Dry run — stopping before agent launch.');
      dashboard?.stop();
      return 0;
    }

    // ── Phase 3: Spawn autonomous agent ──
    // The agent command is printed so the user can run it manually or
    // hex's MCP/daemon layer can spawn it programmatically.
    const agentPrompt = [
      `You are an autonomous hex-coder working on project "${projectName}".`,
      '',
      `## Task`,
      prompt,
      '',
      `## Working Directory`,
      workDir,
      '',
      `## Mode`,
      review ? 'REVIEW: Pause after planning and after coding to ask for approval.' :
      yolo ? 'YOLO: Execute fully autonomously. Plan, code, test, commit — no pauses.' :
      'DEFAULT: Execute autonomously but ask before final commit.',
      '',
      `## Hex Architecture Rules`,
      '- domain/ imports only domain/',
      '- ports/ imports only domain/',
      '- usecases/ imports domain/ and ports/ only',
      '- adapters/ imports ports/ only',
      '- composition-root.ts is the only file that crosses boundaries',
      '- All relative imports use .js extensions',
      '',
      claudeMd ? `## Project Instructions (CLAUDE.md)\n${claudeMd.slice(0, 2000)}` : '',
      projectOverview ? `## Project Structure (L0)\n${projectOverview}` : '',
      portsContext ? `## Port Interfaces (L1 Summaries)\n${portsContext}` : '',
      '',
      `## Workflow`,
      '1. Read the codebase structure (src/core/ports, src/adapters)',
      '2. Plan your changes (which layers, which files)',
      '3. Write tests first (TDD red phase)',
      '4. Implement the feature (TDD green phase)',
      '5. Run: bun test (or the project\'s test command)',
      '6. Commit with a descriptive message',
      '',
      `## Validation Gate`,
      'Before declaring done:',
      '- All tests must pass',
      '- No hex boundary violations (check imports)',
      '- The app must be runnable, not just compilable',
      '',
      'When finished, report: files changed, tests added, commit hash.',
    ].filter(Boolean).join('\n');

    // Write the agent prompt to a file so it can be picked up by Claude Code
    const promptFile = `${workDir}/.hex/go-prompt.md`;
    try {
      const { mkdir, writeFile } = await import('node:fs/promises');
      await mkdir(`${workDir}/.hex`, { recursive: true }).catch(() => {});
      await writeFile(promptFile, agentPrompt);
      this.writeLn(`[hex go] Agent prompt written to: ${promptFile}`);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      this.writeLn(`[hex go] Could not write prompt file: ${msg}`);
    }

    // ── Phase 3b: Install hub-push hook into target project ──
    try {
      const { mkdir, writeFile, readFile } = await import('node:fs/promises');
      const { resolve: resolvePath } = await import('node:path');
      const hookSrc = resolvePath(this.ctx.rootPath, '.claude/helpers/hub-push.cjs');
      const hookContent = await readFile(hookSrc, 'utf-8');
      const targetHelpersDir = `${workDir}/.claude/helpers`;
      await mkdir(targetHelpersDir, { recursive: true }).catch(() => {});
      await writeFile(`${targetHelpersDir}/hub-push.cjs`, hookContent);

      // Merge hub-push hooks into target project's settings.json
      const targetSettingsPath = `${workDir}/.claude/settings.json`;
      let settings: any = {};
      try { settings = JSON.parse(await readFile(targetSettingsPath, 'utf-8')); } catch { /* new file */ }
      if (!settings.hooks) settings.hooks = {};
      if (!settings.hooks.PostToolUse) settings.hooks.PostToolUse = [];

      // Add hub-push hooks if not already present
      const hasHubPush = JSON.stringify(settings.hooks).includes('hub-push.cjs');
      if (!hasHubPush) {
        settings.hooks.PostToolUse.push({
          matcher: 'Write|Edit|MultiEdit|Bash|Read',
          hooks: [{ type: 'command', command: 'node "$CLAUDE_PROJECT_DIR/.claude/helpers/hub-push.cjs" tool-use', timeout: 3000 }],
        });
        await writeFile(targetSettingsPath, JSON.stringify(settings, null, 2));
        this.writeLn('[hex go] Hub-push hook installed in target project');
      }
    } catch {
      // Non-critical — agent runs without dashboard events
    }

    // ── Phase 4: Launch Claude Code ──
    this.writeLn('[hex go] Launching Claude Code...');
    dashboard?.broadcast('go-agent-launched', { prompt, workDir, timestamp: Date.now() });
    this.writeLn('');

    const { execFile: execFileCb } = await import('child_process');
    const { promisify } = await import('util');
    const execFile = promisify(execFileCb);

    // Build claude command args
    const claudeArgs = [
      '--print',        // non-interactive, stream output
      '--dangerously-skip-permissions',  // YOLO mode — agent needs full autonomy
      agentPrompt,
    ];

    try {
      const result = await execFile('claude', claudeArgs, {
        cwd: workDir,
        maxBuffer: 50 * 1024 * 1024, // 50MB — agents produce a lot of output
        timeout: 30 * 60 * 1000, // 30 minute timeout
        env: {
          ...process.env,
          HEX_PROJECT_ROOT: this.ctx.rootPath,
          HEX_HUB_PORT: String(5555),
        },
      });

      if (result.stdout) {
        this.writeLn(result.stdout);
      }

      this.writeLn('');
      this.writeLn('[hex go] Agent completed.');
      dashboard?.broadcast('go-agent-completed', { prompt, workDir, timestamp: Date.now() });
    } catch (err: any) {
      if (err.stdout) this.writeLn(err.stdout);
      if (err.stderr) this.writeLn(err.stderr);
      this.writeLn(`[hex go] Agent exited with error: ${err.message ?? 'unknown'}`);
      dashboard?.broadcast('go-agent-failed', { prompt, error: err.message ?? 'unknown', timestamp: Date.now() });

      if (!noWorktree) {
        this.writeLn(`[hex go] Worktree preserved at: ${workDir}`);
        this.writeLn(`[hex go] Branch: ${branchName}`);
        this.writeLn('[hex go] To resume: cd into the worktree and continue manually');
      }
      dashboard?.stop();
      return 1;
    }

    // ── Phase 5: Validation ──
    this.writeLn('[hex go] Running validation...');
    try {
      const buildResult = await this.ctx.build.test(
        { name: projectName, rootPath: workDir, language: 'typescript', adapters: [] },
        { name: 'all', filePaths: [], type: 'unit' },
      );

      if (!buildResult.success) {
        this.writeLn(`[hex go] FAIL — ${buildResult.failed} test(s) failed`);
        dashboard?.broadcast('go-validation', { result: 'fail', failed: buildResult.failed, timestamp: Date.now() });
        if (!noWorktree) {
          this.writeLn(`[hex go] Worktree preserved at: ${workDir}`);
        }
        dashboard?.stop();
        return 1;
      }

      this.writeLn(`[hex go] PASS — ${buildResult.passed} tests passed`);
      dashboard?.broadcast('go-validation', { result: 'pass', passed: buildResult.passed, timestamp: Date.now() });
    } catch {
      this.writeLn('[hex go] Validation skipped (no test runner found)');
      dashboard?.broadcast('go-validation', { result: 'skipped', timestamp: Date.now() });
    }

    // ── Phase 6: Merge decision ──
    if (!noWorktree) {
      if (yolo) {
        this.writeLn('[hex go] --yolo: auto-merging to current branch...');
        try {
          const mergeResult = await this.ctx.worktree.merge(
            { absolutePath: workDir, branch: branchName },
            await this.ctx.git.currentBranch(),
          );
          if (mergeResult.success) {
            this.writeLn(`[hex go] Merged: ${mergeResult.commitHash ?? branchName}`);
            await this.ctx.worktree.cleanup({ absolutePath: workDir, branch: branchName });
            this.writeLn('[hex go] Worktree cleaned up.');
          } else {
            this.writeLn(`[hex go] Merge conflicts in: ${mergeResult.conflicts.join(', ')}`);
            this.writeLn(`[hex go] Resolve manually in: ${workDir}`);
            return 1;
          }
        } catch (err) {
          const msg = err instanceof Error ? err.message : String(err);
          this.writeLn(`[hex go] Merge failed: ${msg}`);
          this.writeLn(`[hex go] Worktree preserved at: ${workDir}`);
          return 1;
        }
      } else {
        this.writeLn('');
        this.writeLn('[hex go] Ready to merge. Review the changes:');
        this.writeLn(`  cd ${workDir}`);
        this.writeLn(`  git diff main...${branchName}`);
        this.writeLn('');
        this.writeLn('To merge:');
        this.writeLn(`  git merge ${branchName}`);
        this.writeLn(`  git worktree remove ${workDir}`);
        this.writeLn(`  git branch -d ${branchName}`);
      }
    }

    this.writeLn('');
    this.writeLn('[hex go] Done.');
    dashboard?.broadcast('go-done', { prompt, mode, timestamp: Date.now() });
    dashboard?.stop();
    return 0;
  }

  // ── compare ──────────────────────────────────────────

  private async compare(args: ParsedArgs): Promise<number> {
    if (args.positional.length === 0) {
      this.writeLn('Usage: hex compare <specification> [--model MODEL]');
      this.writeLn('');
      this.writeLn('Runs the same task on both Claude Code CLI and Anthropic API,');
      this.writeLn('then compares build success, test pass rate, arch health, speed, and tokens.');
      return 1;
    }

    if (!this.ctx.anthropicExecutor) {
      this.writeLn('Error: ANTHROPIC_API_KEY is required for comparison mode.');
      this.writeLn('Set it in your environment or .hex/secrets.json');
      return 1;
    }

    if (!this.ctx.claudeCodeExecutor) {
      this.writeLn('Error: Claude Code CLI is required for comparison mode.');
      this.writeLn('Install it via: npm install -g @anthropic-ai/claude-code');
      return 1;
    }

    const specification = args.positional.join(' ');
    const model = args.flags.get('model');

    this.writeLn(`${glyph('╔', '+')}${glyph('═', '=').repeat(54)}${glyph('╗', '+')}`);
    this.writeLn(`${glyph('║', '|')}  hex compare ${glyph('—', '--')} Claude Code vs Anthropic API         ${glyph('║', '|')}`);
    this.writeLn(`${glyph('╚', '+')}${glyph('═', '=').repeat(54)}${glyph('╝', '+')}`);
    this.writeLn('');
    this.writeLn(`Specification: ${specification}`);
    this.writeLn('');

    if (!this.ctx.comparator) {
      this.writeLn('Comparator not available — both Anthropic API and Claude Code executors are required.');
      return 1;
    }
    const comparator = this.ctx.comparator;

    this.writeLn('Starting parallel execution...');
    this.writeLn('  [CC]  Claude Code CLI');
    this.writeLn('  [API] Anthropic API (direct)');
    this.writeLn('');

    const report = await comparator.compare(
      specification,
      { prompt: specification, role: 'coder', ...(model ? { model } : {}) },
      (backend, chunk) => {
        const prefix = backend === 'claude-code' ? '[CC] ' : '[API]';
        const firstLine = chunk.split('\n')[0]?.slice(0, 80);
        if (firstLine?.trim()) {
          this.writeLn(`  ${prefix} ${firstLine}`);
        }
      },
    );

    // Display results
    this.writeLn('');
    this.writeLn(glyph('═', '=').repeat(56));
    this.writeLn('  RESULTS');
    this.writeLn(glyph('═', '=').repeat(56));
    this.writeLn('');

    for (const entry of report.entries) {
      const tag = entry.backend === 'claude-code' ? 'Claude Code' : 'Anthropic API';
      const status = entry.result.status === 'success' ? 'OK' : entry.result.status;
      this.writeLn(`  ${tag}:`);
      this.writeLn(`    Status:      ${status}`);
      this.writeLn(`    Build:       ${entry.buildSuccess ? 'PASS' : 'FAIL'}`);
      this.writeLn(`    Tests:       ${Math.round(entry.testPassRate * 100)}% pass rate`);
      this.writeLn(`    Arch Score:  ${entry.archHealthScore}/100`);
      this.writeLn(`    Tokens:      ${entry.result.metrics.totalInputTokens + entry.result.metrics.totalOutputTokens} total`);
      this.writeLn(`    Duration:    ${(entry.result.metrics.durationMs / 1000).toFixed(1)}s`);
      this.writeLn(`    Tool Calls:  ${entry.result.metrics.totalToolCalls}`);
      this.writeLn(`    Turns:       ${entry.result.metrics.totalTurns}`);
      this.writeLn('');
    }

    this.writeLn(hr(56));
    const winnerLabel = report.winner === 'tie' ? 'TIE'
      : report.winner === 'claude-code' ? 'Claude Code' : 'Anthropic API';
    this.writeLn(`  Winner: ${winnerLabel}`);
    this.writeLn(`  Quality: CC=${report.summary.quality.claudeCode} vs API=${report.summary.quality.anthropicApi}`);
    this.writeLn(hr(56));

    // Write report to disk
    const reportPath = `${this.ctx.outputDir}/compare-${report.id}.json`;
    await this.ctx.fs.write(reportPath, JSON.stringify(report, null, 2));
    this.writeLn(`\nFull report: ${reportPath}`);

    return 0;
  }

  // ── scaffold (alias for init) ──────────────────────

  private async scaffold(args: ParsedArgs): Promise<number> {
    const name = args.positional[0];
    if (name && !args.flags.has('skip-wizard')) {
      args.flags.set('skip-wizard', 'true');
    }
    return this.init(args);
  }

  // ── validate ──────────────────────────────────────

  private async validate(args: ParsedArgs): Promise<number> {
    const targetPath = args.positional[0] ?? '.';

    // Phase 1: Architecture validation (always available)
    this.writeLn('Phase 1: Architecture validation');
    const archResult = await this.ctx.archAnalyzer.analyzeArchitecture(targetPath);
    const s = archResult.summary;

    const archPass = s.violationCount === 0 && s.circularCount === 0;
    this.writeLn(`  Hex violations:  ${s.violationCount}`);
    this.writeLn(`  Circular deps:   ${s.circularCount}`);
    this.writeLn(`  Dead exports:    ${s.deadExportCount}`);
    this.writeLn(`  Health score:    ${s.healthScore}/100`);
    this.writeLn(`  ${archPass ? 'PASS' : 'FAIL'}`);
    this.writeLn('');

    // Phase 2: Semantic validation (requires LLM for spec generation)
    if (this.ctx.validator) {
      this.writeLn('Phase 2: Semantic validation');
      try {
        const specs = await this.ctx.validator.generateBehavioralSpecs(
          `Validate project at ${targetPath}`,
        );
        const properties = await this.ctx.validator.generatePropertySpecs([]);
        const scenarios = await this.ctx.validator.generateSmokeScenarios(specs);
        const conventions = await this.ctx.validator.auditSignConventions(targetPath);
        const verdict = await this.ctx.validator.validate(specs, properties, scenarios, conventions);

        this.writeLn(`  Behavioral:  ${verdict.behavioralPass ? 'PASS' : 'FAIL'}`);
        this.writeLn(`  Properties:  ${verdict.propertyPass ? 'PASS' : 'FAIL'}`);
        this.writeLn(`  Smoke:       ${verdict.smokePass ? 'PASS' : 'FAIL'}`);
        this.writeLn(`  Verdict:     ${verdict.pass ? 'PASS' : 'FAIL'}`);

        return (archPass && verdict.pass) ? 0 : 1;
      } catch (err) {
        this.writeLn(`  Error: ${err instanceof Error ? err.message : String(err)}`);
        return archPass ? 0 : 1;
      }
    } else {
      this.writeLn('Phase 2: Semantic validation (skipped — no LLM configured)');
      this.writeLn('  Set ANTHROPIC_API_KEY to enable behavioral/property/smoke validation.');
    }

    return archPass ? 0 : 1;
  }

  // ── orchestrate ───────────────────────────────────

  private async orchestrate(args: ParsedArgs): Promise<number> {
    const workplanFile = args.positional[0];

    if (!workplanFile) {
      this.writeLn('Usage: hex orchestrate <workplan-file>');
      this.writeLn('');
      this.writeLn('Execute a workplan via swarm agents.');
      this.writeLn('Generate a workplan first: hex plan <requirements>');
      return 1;
    }

    if (!this.ctx.workplanExecutor) {
      this.writeLn('LLM not configured. Set ANTHROPIC_API_KEY or OPENAI_API_KEY.');
      this.writeLn('Tip: Inside Claude Code, use hex_orchestrate via MCP.');
      return 1;
    }

    const content = await this.ctx.fs.read(workplanFile);
    let workplan;
    try {
      workplan = JSON.parse(content);
    } catch {
      this.writeLn(`Error: ${workplanFile} is not valid JSON.`);
      return 1;
    }

    this.writeLn(`Orchestrating: ${workplan.title ?? workplanFile}`);
    this.writeLn(`Steps: ${workplan.steps?.length ?? 0}`);
    this.writeLn('');

    let stepNum = 0;
    for await (const result of this.ctx.workplanExecutor.executePlan(workplan)) {
      stepNum++;
      const status = result.success ? 'PASS' : 'FAIL';
      this.writeLn(`  [${stepNum}] ${result.stepId}: ${status}`);
      if (!result.success && result.error) {
        this.writeLn(`       ${result.error}`);
      }
    }

    this.writeLn('');
    this.writeLn('Orchestration complete.');
    return 0;
  }

  // ── help ────────────────────────────────────────────

  private help(): number {
    this.writeLn(header());
    this.writeLn('');
    this.writeLn(`${bold('Usage:')} hex ${muted('<command>')} ${muted('[options]')}`);

    this.writeLn(section('Build & Ship'));
    this.writeLn(`  ${bold('go')} ${muted('<prompt>')} ${muted('[--yolo|--review]')}     Autonomous coding agent`);
    this.writeLn(`  ${bold('build')} ${muted('<prompt>')}                       ${dim('Alias for go')}`);
    this.writeLn(`  ${bold('orchestrate')} ${muted('<workplan.json>')}          Execute workplan via swarm agents`);

    this.writeLn(section('Scaffold & Generate'));
    this.writeLn(`  ${bold('init')} ${muted('[--lang ts|go|rust]')}             Scaffold a new hex project`);
    this.writeLn(`  ${bold('scaffold')} ${muted('<name> [--lang L]')}           ${dim('Alias for init')}`);
    this.writeLn(`  ${bold('generate')} ${muted('<spec> [--adapter N]')}        Generate code from a spec file`);
    this.writeLn(`  ${bold('plan')} ${muted('<requirements...>')}               Create a workplan from requirements`);

    this.writeLn(section('Analyze & Validate'));
    this.writeLn(`  ${bold('analyze')} ${muted('[path] [--json|--compact]')}    Architecture health check`);
    this.writeLn(`  ${bold('validate')} ${muted('[path]')}                      Post-build semantic validation`);
    this.writeLn(`  ${bold('summarize')} ${muted('<file> [--level L0-L3]')}     Print AST summary`);
    this.writeLn(`  ${bold('compare')} ${muted('<spec> [--model M]')}           Compare agent backends`);

    this.writeLn(section('Dashboard & Services'));
    this.writeLn(`  ${bold('dashboard')}                             Start live dashboard`);
    this.writeLn(`  ${bold('hub')} ${muted('[start|stop|status]')}              Manage hex-hub daemon`);
    this.writeLn(`  ${bold('daemon')} ${muted('[status|start|stop|logs]')}      Background service`);
    this.writeLn(`  ${bold('status')}                                Show project status`);
    this.writeLn(`  ${bold('projects')}                              List registered projects`);

    this.writeLn(section('Architecture Decisions'));
    this.writeLn(`  ${bold('adr')} ${muted('list [--status S]')}                 List ADRs with optional filter`);
    this.writeLn(`  ${bold('adr')} ${muted('status <id>')}                      Show ADR detail`);
    this.writeLn(`  ${bold('adr')} ${muted('search <query>')}                   Search ADRs via AgentDB`);
    this.writeLn(`  ${bold('adr')} ${muted('abandoned [--days N]')}             Find stale proposed ADRs`);
    this.writeLn(`  ${bold('adr')} ${muted('reindex')}                          Re-index ADRs into AgentDB`);

    this.writeLn(section('Configuration'));
    this.writeLn(`  ${bold('setup')}                                 Install grammars + skills + agents`);
    this.writeLn(`  ${bold('secrets')} ${muted('<cmd> [args]')}                 Local vault management`);
    this.writeLn(`  ${bold('mcp')}                                   Start as MCP server (stdio)`);

    this.writeLn('');
    this.writeLn(`${muted('MCP integration (add to any project):')}`);
    this.writeLn(`  ${dim('{')} ${cyan('"mcpServers"')}: ${dim('{')} ${cyan('"hex"')}: ${dim('{')} ${cyan('"command"')}: ${green('"npx"')}, ${cyan('"args"')}: [${green('"hex"')}, ${green('"mcp"')}] ${dim('} } }')}`);

    this.writeLn('');
    this.writeLn(`${muted('Examples:')}`);
    this.writeLn(`  ${dim('$')} hex go "add user authentication" --yolo`);
    this.writeLn(`  ${dim('$')} hex analyze ./src --json`);
    this.writeLn(`  ${dim('$')} hex init --lang rust`);

    return 0;
  }

  // ── setup ──────────────────────────────────────────

  private async setup(): Promise<number> {
    this.writeLn('Setting up hex...');
    this.writeLn('');

    const languages = ['typescript', 'go', 'rust'];
    const { execFile: execFileCb } = await import('child_process');
    const { promisify } = await import('util');
    const run = promisify(execFileCb);

    // Install core dependencies (ruflo, agentdb, tree-sitter)
    const coreDeps = [
      { pkg: 'ruflo', check: 'node_modules/ruflo' },
      { pkg: 'agentdb', check: 'node_modules/agentdb' },
      { pkg: 'tree-sitter-wasms', check: 'node_modules/tree-sitter-wasms/out' },
      { pkg: 'web-tree-sitter', check: 'node_modules/web-tree-sitter' },
    ];

    for (const dep of coreDeps) {
      if (!(await this.ctx.fs.exists(dep.check))) {
        this.writeLn(`Installing ${dep.pkg}...`);
        try {
          await run('bun', ['add', dep.pkg], { cwd: this.ctx.rootPath, timeout: 60000 });
          this.writeLn(`  ${dep.pkg} installed.`);
        } catch {
          this.writeLn(`  Failed. Run manually: bun add ${dep.pkg}`);
        }
      } else {
        this.writeLn(`  ${dep.pkg}: already installed`);
      }
    }

    // Fix agentdb controller path (dist/controllers -> dist/src/controllers)
    const agentdbControllers = 'node_modules/agentdb/dist/controllers';
    if (!(await this.ctx.fs.exists(agentdbControllers))) {
      const { symlinkSync } = await import('node:fs');
      const { resolve } = await import('node:path');
      try {
        symlinkSync(
          resolve(this.ctx.rootPath, 'node_modules/agentdb/dist/src/controllers'),
          resolve(this.ctx.rootPath, agentdbControllers),
        );
        this.writeLn('  agentdb controllers symlinked.');
      } catch { /* already exists or no agentdb */ }
    }

    // Install tree-sitter-wasms if not present (legacy path for older setups)
    const hasWasms = await this.ctx.fs.exists('node_modules/tree-sitter-wasms/out');
    if (!hasWasms) {
      this.writeLn('Installing tree-sitter WASM grammars...');
      try {
        await run('bun', ['add', 'tree-sitter-wasms'], { cwd: this.ctx.rootPath, timeout: 30000 });
        this.writeLn('  tree-sitter-wasms installed.');
      } catch {
        this.writeLn('  Failed. Run manually: bun add tree-sitter-wasms');
        return 1;
      }
    }

    // Check grammar availability — use absolute paths since grammars
    // may be in hex's own node_modules or the project's
    const { access } = await import('node:fs/promises');
    const { resolve } = await import('node:path');
    // Resolve from: 1) project's config, 2) project's node_modules,
    // 3) hex's own node_modules (for global install via npm link)
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
    const agentsTarget = join(claudeDir, 'agents', 'hex');

    // Find hex's own skills/ and agents/ directories
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
      this.writeLn(`  Agents: ${agentCount} installed to .claude/agents/hex/`);

    } catch (err) {
      this.writeLn(`  Failed to install skills/agents: ${err instanceof Error ? err.message : String(err)}`);
    }

    // Register ruflo MCP server in project-local Claude Code settings
    await this.registerRufloMCP(claudeDir, join);

    // Build/install hex-hub Rust binary
    this.writeLn('');
    this.writeLn('Installing hex-hub dashboard...');
    const { existsSync } = await import('node:fs');
    const { homedir } = await import('node:os');
    const { dirname: pathDirname } = await import('node:path');
    const { fileURLToPath } = await import('node:url');

    const hexBinDir = join(homedir(), '.hex', 'bin');
    const hexBinDest = join(hexBinDir, 'hex-hub');

    // Check if already installed
    if (existsSync(hexBinDest)) {
      this.writeLn(`  hex-hub already installed at ${hexBinDest}`);
    } else {
      // Search for Cargo.toml in: CWD/hex-hub, rootPath/hex-hub, and relative to this module
      const thisDir = pathDirname(fileURLToPath(import.meta.url));
      const candidates = [
        join(process.cwd(), 'hex-hub', 'Cargo.toml'),
        join(this.ctx.rootPath, 'hex-hub', 'Cargo.toml'),
      ];
      // Walk up from this module to find hex-hub/ (works in dev and bundled layouts)
      let walkDir = thisDir;
      for (let i = 0; i < 6; i++) {
        candidates.push(join(walkDir, 'hex-hub', 'Cargo.toml'));
        const parent = pathDirname(walkDir);
        if (parent === walkDir) break;
        walkDir = parent;
      }

      const cargoToml = candidates.find((p) => existsSync(p));

      if (cargoToml) {
        const hubDir = pathDirname(cargoToml);
        const prebuilt = join(hubDir, 'target', 'release', 'hex-hub');

        if (existsSync(prebuilt)) {
          // Pre-built binary exists — just copy it
          const { mkdirSync, copyFileSync, chmodSync } = await import('node:fs');
          mkdirSync(hexBinDir, { recursive: true });
          copyFileSync(prebuilt, hexBinDest);
          chmodSync(hexBinDest, 0o755);
          this.writeLn(`  hex-hub installed from pre-built binary to ${hexBinDest}`);
        } else {
          // Build from source
          try {
            const { promisify } = await import('util');
            const { execFile: execFileCb } = await import('child_process');
            const execFile = promisify(execFileCb);

            this.writeLn('  Building hex-hub from source (cargo build --release)...');
            await execFile('cargo', ['build', '--release', '--manifest-path', cargoToml], {
              timeout: 120_000,
            });

            const { mkdirSync, copyFileSync, chmodSync } = await import('node:fs');
            mkdirSync(hexBinDir, { recursive: true });
            copyFileSync(prebuilt, hexBinDest);
            chmodSync(hexBinDest, 0o755);
            this.writeLn(`  hex-hub installed to ${hexBinDest}`);
          } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            this.writeLn(`  hex-hub build failed: ${msg}`);
            this.writeLn('  Install Rust toolchain (rustup.rs) and retry.');
          }
        }
      } else {
        // Check if pre-built binary exists in CWD search paths
        const found = this.ctx.hubLauncher?.findBinary();
        if (found) {
          const { mkdirSync, copyFileSync, chmodSync } = await import('node:fs');
          mkdirSync(hexBinDir, { recursive: true });
          copyFileSync(found, hexBinDest);
          chmodSync(hexBinDest, 0o755);
          this.writeLn(`  hex-hub installed from ${found} to ${hexBinDest}`);
        } else {
          this.writeLn('  hex-hub binary not found.');
          this.writeLn('  To install: clone the hex repo and run "cargo build --release" in hex-hub/');
          this.writeLn('  Then re-run "hex setup".');
        }
      }
    }

    this.writeLn('');
    this.writeLn('Setup complete. Available commands:');
    this.writeLn('  hex analyze .     Check architecture health');
    this.writeLn('  hex summarize     AST summary of a file');
    this.writeLn('  hex init          Scaffold a new hex project');
    this.writeLn('  hex help          Show all commands');
    return 0;
  }

  // ── registerRufloMCP ──────────────────────────────

  private async registerRufloMCP(claudeDir: string, join: (...args: string[]) => string): Promise<void> {
    const { readFile, writeFile } = await import('node:fs/promises');
    const settingsPath = join(claudeDir, 'settings.local.json');

    // Find ruflo binary path
    const { execFile: execFileCb } = await import('child_process');
    const { promisify } = await import('util');
    const run = promisify(execFileCb);

    let rufloPath = '';
    try {
      const { stdout } = await run('which', ['ruflo'], { timeout: 5000 });
      rufloPath = stdout.trim();
    } catch {
      // Try npx resolution
      try {
        const { stdout } = await run('npx', ['--yes', 'ruflo', '--version'], { timeout: 15000 });
        if (stdout.trim()) rufloPath = 'npx';
      } catch { /* ruflo not available */ }
    }

    if (!rufloPath) {
      this.writeLn('');
      this.writeLn('  Ruflo MCP: skipped (ruflo not found)');
      this.writeLn('  Install with: npm install -g ruflo');
      return;
    }

    // Read existing settings or create new
    let settings: Record<string, unknown> = {};
    try {
      const existing = await readFile(settingsPath, 'utf-8');
      settings = JSON.parse(existing);
    } catch { /* file doesn't exist yet */ }

    // Check if ruflo MCP is already registered
    const mcpServers = (settings.mcpServers ?? {}) as Record<string, unknown>;
    if (mcpServers.ruflo) {
      this.writeLn('');
      this.writeLn('  Ruflo MCP: already registered');
      return;
    }

    // Register ruflo as stdio MCP server
    const rufloConfig = rufloPath === 'npx'
      ? { command: 'npx', args: ['--yes', 'ruflo', 'mcp', 'start'], type: 'stdio' }
      : { command: rufloPath, args: ['mcp', 'start'], type: 'stdio' };

    mcpServers.ruflo = rufloConfig;
    settings.mcpServers = mcpServers;

    await writeFile(settingsPath, JSON.stringify(settings, null, 2) + '\n', 'utf-8');
    this.writeLn('');
    this.writeLn(`  Ruflo MCP: registered in .claude/settings.local.json`);
    this.writeLn(`  Binary: ${rufloPath}`);
    this.writeLn('  Restart Claude Code to activate swarm tools');
  }
}
