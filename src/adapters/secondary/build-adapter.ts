// @ts-nocheck — legacy adapter, replaced by Rust CLI (ADR-010)
/**
 * Build secondary adapter -- implements IBuildPort.
 *
 * Runs compile, lint, and test commands for TypeScript projects using
 * Bun/tsc/eslint via execFile (no shell injection). Designed for future
 * extension to Go and Rust.
 */
import { createRequire } from 'node:module';
import { promisify } from 'node:util';
// Use createRequire to avoid Bun's ESM named-export race under parallel test load
const _require = createRequire(import.meta.url);
const { execFile: execFileCb } = _require('node:child_process');
import type {
  BuildResult,
  IBuildPort,
  Language,
  LintError,
  LintResult,
  Project,
  TestFailure,
  TestResult,
  TestSuite,
} from '../../core/ports/index.js';

const execFile = promisify(execFileCb);

class BuildError extends Error {
  override readonly name = 'BuildError';
  constructor(message: string, public readonly command: string) {
    super(message);
  }
}

export class BuildAdapter implements IBuildPort {
  constructor(
    private readonly projectPath: string,
    private readonly language: Language = 'typescript',
  ) {}

  /** The language this adapter was configured for. */
  get configuredLanguage(): Language {
    return this.language;
  }

  async compile(project: Project): Promise<BuildResult> {
    switch (this.language) {
      case 'go': return this.compileGo(project);
      case 'rust': return this.compileRust(project);
      default: return this.compileTypeScript(project);
    }
  }

  async lint(project: Project): Promise<LintResult> {
    switch (this.language) {
      case 'go': return this.lintGo(project);
      case 'rust': return this.lintRust(project);
      default: return this.lintTypeScript(project);
    }
  }

  async test(project: Project, suite: TestSuite): Promise<TestResult> {
    switch (this.language) {
      case 'go': return this.testGo(project, suite);
      case 'rust': return this.testRust(project, suite);
      default: return this.testTypeScript(project, suite);
    }
  }

  // ── TypeScript ────────────────────────────────────────────

  private async compileTypeScript(project: Project): Promise<BuildResult> {
    const start = Date.now();
    try {
      await this.run('npx', ['tsc', '--noEmit'], project.rootPath);
      return { success: true, errors: [], duration: Date.now() - start };
    } catch (err: unknown) {
      const e = err as { stdout?: string; stderr?: string };
      const output = (e.stdout ?? '') + (e.stderr ?? '');
      const errors = output.split('\n').filter((l) => /error TS\d+/.test(l));
      return { success: false, errors, duration: Date.now() - start };
    }
  }

  private async lintTypeScript(project: Project): Promise<LintResult> {
    try {
      await this.run('npx', ['eslint', '--format', 'json', '.'], project.rootPath);
      return { success: true, errors: [], warningCount: 0, errorCount: 0 };
    } catch (err: unknown) {
      const e = err as { stdout?: string };
      return this.parseEslintOutput(e.stdout ?? '[]');
    }
  }

  private async testTypeScript(project: Project, suite: TestSuite): Promise<TestResult> {
    const args = ['test', ...suite.filePaths];
    const start = Date.now();
    try {
      const { stdout } = await this.run('bun', args, project.rootPath);
      return this.parseTestOutput(stdout, Date.now() - start);
    } catch (err: unknown) {
      const e = err as { stdout?: string };
      return this.parseTestOutput(e.stdout ?? '', Date.now() - start);
    }
  }

  // ── Go ────────────────────────────────────────────────────

  private async compileGo(project: Project): Promise<BuildResult> {
    const start = Date.now();
    try {
      await this.run('go', ['build', './...'], project.rootPath);
      return { success: true, errors: [], duration: Date.now() - start };
    } catch (err: unknown) {
      const e = err as { stdout?: string; stderr?: string };
      const output = (e.stdout ?? '') + (e.stderr ?? '');
      const errors = output.split('\n').filter((l) => l.includes('.go:'));
      return { success: false, errors, duration: Date.now() - start };
    }
  }

  private async lintGo(project: Project): Promise<LintResult> {
    try {
      await this.run('golangci-lint', ['run', '--out-format', 'json'], project.rootPath);
      return { success: true, errors: [], warningCount: 0, errorCount: 0 };
    } catch (err: unknown) {
      const e = err as { stdout?: string };
      return this.parseGolangciOutput(e.stdout ?? '{}');
    }
  }

  private async testGo(project: Project, suite: TestSuite): Promise<TestResult> {
    const args = suite.filePaths.length > 0
      ? ['test', '-json', ...suite.filePaths]
      : ['test', '-json', './...'];
    const start = Date.now();
    try {
      const { stdout } = await this.run('go', args, project.rootPath);
      return this.parseGoTestOutput(stdout, Date.now() - start);
    } catch (err: unknown) {
      const e = err as { stdout?: string };
      return this.parseGoTestOutput(e.stdout ?? '', Date.now() - start);
    }
  }

  // ── Rust ──────────────────────────────────────────────────

  private async compileRust(project: Project): Promise<BuildResult> {
    const start = Date.now();
    try {
      await this.run('cargo', ['check'], project.rootPath);
      return { success: true, errors: [], duration: Date.now() - start };
    } catch (err: unknown) {
      const e = err as { stdout?: string; stderr?: string };
      const output = (e.stdout ?? '') + (e.stderr ?? '');
      const errors = output.split('\n').filter((l) => /^error/.test(l));
      return { success: false, errors, duration: Date.now() - start };
    }
  }

  private async lintRust(project: Project): Promise<LintResult> {
    try {
      await this.run('cargo', ['clippy', '--', '-D', 'warnings'], project.rootPath);
      return { success: true, errors: [], warningCount: 0, errorCount: 0 };
    } catch (err: unknown) {
      const e = err as { stdout?: string; stderr?: string };
      return this.parseClippyOutput((e.stderr ?? '') + (e.stdout ?? ''));
    }
  }

  private async testRust(project: Project, suite: TestSuite): Promise<TestResult> {
    const args = suite.filePaths.length > 0
      ? ['test', '--', ...suite.filePaths]
      : ['test'];
    const start = Date.now();
    try {
      const { stdout } = await this.run('cargo', args, project.rootPath);
      return this.parseCargoTestOutput(stdout, Date.now() - start);
    } catch (err: unknown) {
      const e = err as { stdout?: string };
      return this.parseCargoTestOutput(e.stdout ?? '', Date.now() - start);
    }
  }

  // ── Private helpers ─────────────────────────────────────────

  private parseEslintOutput(raw: string): LintResult {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!Array.isArray(parsed)) {
        return { success: false, errors: [], warningCount: 0, errorCount: 1 };
      }
      const files = parsed as Array<{
        filePath: string;
        messages: Array<{
          line: number; column: number;
          severity: number; message: string; ruleId: string | null;
        }>;
      }>;
      const errors: LintError[] = [];
      let warningCount = 0;
      let errorCount = 0;
      for (const file of files) {
        for (const msg of file.messages) {
          const severity = msg.severity === 2 ? 'error' as const : 'warning' as const;
          if (severity === 'error') errorCount++;
          else warningCount++;
          errors.push({
            filePath: file.filePath,
            line: msg.line,
            column: msg.column,
            severity,
            message: msg.message,
            rule: msg.ruleId ?? 'unknown',
          });
        }
      }
      return { success: errorCount === 0, errors, warningCount, errorCount };
    } catch (e) {
      // ESLint output may not be valid JSON (e.g., eslint crashed or produced text errors)
      console.error('Warning: failed to parse eslint output:', e);
      return { success: false, errors: [], warningCount: 0, errorCount: 1 };
    }
  }

  private parseTestOutput(stdout: string, duration: number): TestResult {
    const passMatch = stdout.match(/(\d+)\s+pass/i);
    const failMatch = stdout.match(/(\d+)\s+fail/i);
    const skipMatch = stdout.match(/(\d+)\s+skip/i);
    const passed = passMatch ? Number(passMatch[1]) : 0;
    const failed = failMatch ? Number(failMatch[1]) : 0;
    const skipped = skipMatch ? Number(skipMatch[1]) : 0;

    const failures: TestFailure[] = [];
    const failBlocks = stdout.split(/(?=FAIL|fail\s)/);
    for (const block of failBlocks) {
      const nameMatch = block.match(/(?:FAIL|fail)\s+(.+)/);
      const msgMatch = block.match(/(?:error|Error):\s*(.+)/);
      if (nameMatch) {
        failures.push({
          testName: nameMatch[1].trim(),
          message: msgMatch?.[1]?.trim() ?? 'Unknown failure',
        });
      }
    }

    return { success: failed === 0, passed, failed, skipped, duration, failures };
  }

  // ── Go output parsers ──────────────────────────────────

  private parseGolangciOutput(raw: string): LintResult {
    try {
      const parsed = JSON.parse(raw) as { Issues?: Array<{
        FromLinter: string; Text: string; Severity: string;
        Pos: { Filename: string; Line: number; Column: number };
      }> };
      const issues = parsed.Issues ?? [];
      const errors: LintError[] = issues.map((i) => ({
        filePath: i.Pos.Filename,
        line: i.Pos.Line,
        column: i.Pos.Column,
        severity: i.Severity === 'error' ? 'error' as const : 'warning' as const,
        message: i.Text,
        rule: i.FromLinter,
      }));
      const errorCount = errors.filter((e) => e.severity === 'error').length;
      const warningCount = errors.length - errorCount;
      return { success: errorCount === 0, errors, warningCount, errorCount };
    } catch {
      return { success: false, errors: [], warningCount: 0, errorCount: 1 };
    }
  }

  private parseGoTestOutput(raw: string, duration: number): TestResult {
    let passed = 0, failed = 0, skipped = 0;
    const failures: TestFailure[] = [];
    for (const line of raw.split('\n')) {
      if (!line.trim()) continue;
      try {
        const ev = JSON.parse(line) as { Action: string; Test?: string; Output?: string };
        if (!ev.Test) continue;
        if (ev.Action === 'pass') passed++;
        else if (ev.Action === 'fail') {
          failed++;
          failures.push({ testName: ev.Test, message: ev.Output?.trim() ?? 'Failed' });
        } else if (ev.Action === 'skip') skipped++;
      } catch { /* non-JSON line */ }
    }
    return { success: failed === 0, passed, failed, skipped, duration, failures };
  }

  // ── Rust output parsers ───────────────────────────────────

  private parseClippyOutput(raw: string): LintResult {
    const errors: LintError[] = [];
    const pattern = /^(warning|error)(?:\[(\w+)])?:\s+(.+)\n\s+-->\s+(.+):(\d+):(\d+)/gm;
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(raw)) !== null) {
      errors.push({
        filePath: match[4],
        line: Number(match[5]),
        column: Number(match[6]),
        severity: match[1] === 'error' ? 'error' : 'warning',
        message: match[3],
        rule: match[2] ?? match[1],
      });
    }
    const errorCount = errors.filter((e) => e.severity === 'error').length;
    const warningCount = errors.length - errorCount;
    return { success: errorCount === 0, errors, warningCount, errorCount };
  }

  private parseCargoTestOutput(raw: string, duration: number): TestResult {
    let passed = 0, failed = 0, skipped = 0;
    const failures: TestFailure[] = [];
    const summaryRe = /test result: \w+\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored/g;
    let sm: RegExpExecArray | null;
    while ((sm = summaryRe.exec(raw)) !== null) {
      passed += Number(sm[1]);
      failed += Number(sm[2]);
      skipped += Number(sm[3]);
    }
    const failRe = /^test (.+) \.\.\. FAILED$/gm;
    let fm: RegExpExecArray | null;
    while ((fm = failRe.exec(raw)) !== null) {
      failures.push({ testName: fm[1], message: 'Test failed' });
    }
    return { success: failed === 0, passed, failed, skipped, duration, failures };
  }

  // ── Shared helpers ────────────────────────────────────────
  // NOTE: Uses execFile (not exec) to prevent shell injection — see ADR-001 security rules.

  private async run(
    cmd: string,
    args: string[],
    cwd?: string,
  ): Promise<{ stdout: string; stderr: string }> {
    return execFile(cmd, args, {
      cwd: cwd ?? this.projectPath,
      maxBuffer: 10 * 1024 * 1024,
    });
  }
}
