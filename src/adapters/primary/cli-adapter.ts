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

import type { IArchAnalysisPort, IASTPort, IFileSystemPort, ASTSummary } from '../../core/ports/index.js';

/** Minimal context needed by the CLI — uses port interfaces only */
export interface AppContext {
  rootPath: string;
  archAnalyzer: IArchAnalysisPort;
  ast: IASTPort;
  fs: IFileSystemPort;
}

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
        case 'status':
          return await this.status();
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
    this.writeLn('  status                          Show swarm progress');
    this.writeLn('  init [--lang ts|go|rust]        Scaffold a hex project');
    this.writeLn('  help                            Show this help');
    this.writeLn('');
    this.writeLn('Examples:');
    this.writeLn('  hex-intf analyze ./src');
    this.writeLn('  hex-intf summarize src/core/ports/index.ts --level L2');
    this.writeLn('  hex-intf status');
    this.writeLn('  hex-intf init --lang ts');

    return 0;
  }

}
