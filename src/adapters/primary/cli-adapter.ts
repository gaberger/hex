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
import type { AppContext as FullAppContext } from '../../composition-root.js';

/**
 * CLIAppContext — the subset of AppContext the CLI adapter needs.
 * Derived from the canonical AppContext in composition-root via Pick,
 * ensuring a single source of truth with no contract divergence.
 * The dashboard command casts to FullAppContext for the HTTP server.
 */
export type AppContext = Pick<
  FullAppContext,
  'rootPath' | 'archAnalyzer' | 'ast' | 'astIsStub' | 'fs' | 'codeGenerator' | 'workplanExecutor' | 'summaryService'
>;

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
        case 'status':
          return await this.status();
        case 'setup':
          return await this.setup();
        case 'init':
          return this.init(args);
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

    if (this.ctx.astIsStub) {
      this.writeLn('\u26a0 Running without tree-sitter grammars \u2014 results may be incomplete');
      this.writeLn('');
    }

    this.writeLn(`Analyzing architecture at: ${targetPath}`);
    this.writeLn('');

    const result = await this.ctx.archAnalyzer.analyzeArchitecture(targetPath);
    const s = result.summary;

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

    this.writeLn(`Generated: ${result.filePath}`);
    this.writeLn(`Language:  ${result.language}`);
    this.writeLn('');
    this.writeLn(result.content);

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
    const port = parseInt(args.flags.get('port') ?? '3847', 10);
    if (isNaN(port) || port < 1 || port > 65535) {
      this.writeLn('Invalid port number. Must be 1-65535.');
      return 1;
    }

    // Dynamic import to avoid loading http server when not needed
    const { startDashboard } = await import('./dashboard-adapter.js');
    const { url } = await startDashboard(this.ctx as FullAppContext, port);
    this.writeLn(`Dashboard running at ${url}`);
    this.writeLn('Press Ctrl+C to stop.');

    // Keep the process alive until interrupted
    await new Promise(() => {});
    return 0;
  }

  // ── status ──────────────────────────────────────────

  private async status(): Promise<number> {
    this.writeLn('Swarm status: use "hex-intf analyze" to check project health.');
    return 0;
  }

  // ── init ────────────────────────────────────────────

  private init(args: ParsedArgs): number {
    const lang = args.flags.get('lang') ?? 'ts';
    const validLangs = ['ts', 'go', 'rust'];
    if (!validLangs.includes(lang)) {
      this.writeLn(`Invalid language: ${lang}. Must be one of: ${validLangs.join(', ')}`);
      return 1;
    }

    this.writeLn(`Scaffolding hex-intf project (${lang}):`);
    this.writeLn('');
    this.writeLn('  src/');
    this.writeLn('    core/');
    this.writeLn('      domain/       Domain entities and value objects');
    this.writeLn('      ports/        Port interfaces (input + output)');
    this.writeLn('      usecases/     Use case implementations');
    this.writeLn('    adapters/');
    this.writeLn('      primary/      Driving adapters (CLI, HTTP, etc.)');
    this.writeLn('      secondary/    Driven adapters (DB, FS, API, etc.)');
    this.writeLn('    infrastructure/ Cross-cutting concerns');
    this.writeLn('    composition-root.ts');
    this.writeLn('    cli.ts');
    this.writeLn('    index.ts');
    this.writeLn('  tests/');
    this.writeLn('');
    this.writeLn('Run "hex-intf analyze" after scaffolding to validate boundaries.');

    return 0;
  }

  // ── help ────────────────────────────────────────────

  private help(): number {
    this.writeLn('hex-intf - Hexagonal Architecture toolkit');
    this.writeLn('');
    this.writeLn('Usage: hex-intf <command> [options]');
    this.writeLn('');
    this.writeLn('Commands:');
    this.writeLn('  analyze [path]                  Analyze architecture health');
    this.writeLn('  summarize <file> [--level L]     Print AST summary (L0-L3)');
    this.writeLn('  generate <spec> [--adapter N]    Generate code from a spec file');
    this.writeLn('    [--lang ts|go|rust]');
    this.writeLn('  plan <requirements...>           Create a workplan from requirements');
    this.writeLn('    [--lang ts|go|rust]');
    this.writeLn('  setup                           Download tree-sitter grammars');
    this.writeLn('  dashboard [--port N]             Open web dashboard (default: 3847)');
    this.writeLn('  status                          Show swarm progress');
    this.writeLn('  init [--lang ts|go|rust]        Scaffold a hex project');
    this.writeLn('  help                            Show this help');
    this.writeLn('');
    this.writeLn('Examples:');
    this.writeLn('  hex-intf analyze ./src');
    this.writeLn('  hex-intf summarize src/core/ports/index.ts --level L2');
    this.writeLn('  hex-intf generate spec.txt --adapter redis --lang ts');
    this.writeLn('  hex-intf plan "add caching layer" "implement retry logic"');
    this.writeLn('  hex-intf status');
    this.writeLn('  hex-intf init --lang ts');

    return 0;
  }

  // ── setup ──────────────────────────────────────────

  private async setup(): Promise<number> {
    this.writeLn('Setting up hex-intf...');
    this.writeLn('');

    const searchPaths = ['config/grammars', 'node_modules/tree-sitter-wasms/out'];
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
          if (f.endsWith('.yml') || f.endsWith('.yaml')) {
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
